use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface::{burn, Burn, Mint, TokenAccount};

use crate::constants::{
    AUTHORITY_SEED, BATCH_SEED, MERCHANT_SEED, OBLIGATION_SEED, OFFER_SEED, POINTS_SEED,
    PROTOCOL_SEED,
};
use crate::error::ObligoError;
use crate::events::Redeemed;
use crate::hook_cpi;
use crate::math::{face, BPS};
use crate::state::{AcceptanceOffer, Merchant, MerchantStatus, Obligation, PointBatch, Protocol};

/// Boxed almost throughout. Eighteen accounts, several of them Token-2022 states with extensions,
/// overflow Anchor's generated `try_accounts` frame past the 4KB an SBF stack gets; the heap does
/// not care.
#[derive(Accounts)]
pub struct Redeem<'info> {
    /// Whoever is paying rent for the permit, the escrow and — the first time these two merchants
    /// meet — the obligation edge between them. Split out from the customer so a merchant's till,
    /// or a relayer, can carry the cost of a customer who has never held a lamport.
    #[account(mut)]
    pub payer: Signer<'info>,

    /// The customer, signing for their own points. The acceptor does not sign: its offer *is* its
    /// consent, posted in public and budgeted on chain. That is what makes it an auction and not
    /// an advertisement — a bid you can withdraw at the till is not a bid.
    pub customer: Signer<'info>,

    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Box<Account<'info, Protocol>>,

    /// The merchant whose points these are, and who will owe for them.
    #[account(
        mut,
        seeds = [MERCHANT_SEED, issuer.authority.as_ref()],
        bump = issuer.bump,
    )]
    pub issuer: Box<Account<'info, Merchant>>,

    /// The merchant honouring them, and who will be owed.
    #[account(
        mut,
        seeds = [MERCHANT_SEED, acceptor.authority.as_ref()],
        bump = acceptor.bump,
    )]
    pub acceptor: Box<Account<'info, Merchant>>,

    #[account(
        mut,
        seeds = [OFFER_SEED, acceptor.key().as_ref(), issuer.key().as_ref()],
        bump = offer.bump,
    )]
    pub offer: Box<Account<'info, AcceptanceOffer>>,

    /// The `issuer -> acceptor` edge of the debt graph, created the first time these two merchants
    /// transact and reused forever after.
    #[account(
        init_if_needed,
        payer = payer,
        space = 8 + Obligation::INIT_SPACE,
        seeds = [OBLIGATION_SEED, issuer.key().as_ref(), acceptor.key().as_ref()],
        bump
    )]
    pub obligation: Box<Account<'info, Obligation>>,

    #[account(
        mut,
        seeds = [POINTS_SEED, issuer.key().as_ref()],
        bump = issuer.mint_bump,
        mint::token_program = token_program,
    )]
    pub points_mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(
        mut,
        associated_token::mint = points_mint,
        associated_token::authority = customer,
        associated_token::token_program = token_program,
    )]
    pub customer_points: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The issuer's redemption escrow. Points land here for the length of one instruction and are
    /// burned before it ends; its balance is zero before and zero after. It exists because the
    /// hook only runs on a *transfer* — see the handler.
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = points_mint,
        associated_token::authority = issuer,
        associated_token::token_program = token_program,
    )]
    pub redemption_escrow: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [BATCH_SEED, issuer.key().as_ref(), customer.key().as_ref()],
        bump = batch.bump,
    )]
    pub batch: Box<Account<'info, PointBatch>>,

    /// CHECK: the program's signer PDA, and the only signature the hook will grant a permit to.
    #[account(seeds = [AUTHORITY_SEED], bump = protocol.authority_bump)]
    pub core_authority: UncheckedAccount<'info>,

    /// CHECK: `[b"permit", customer_points]` under the hook program, which pins the address itself
    /// when it grants the permit and re-derives it from a stored bump when it spends it. Passing
    /// the wrong account here gets the transaction rejected by the hook, not by us.
    #[account(mut)]
    pub permit: UncheckedAccount<'info>,

    /// CHECK: the mint's ExtraAccountMetaList. Token-2022 derives the address it expects from the
    /// mint and the hook program and looks it up among the accounts we hand it; a substitute is
    /// simply not found.
    pub extra_account_meta_list: UncheckedAccount<'info>,

    /// CHECK: the hook the protocol was born with. Fixed at genesis and never updatable.
    #[account(executable, address = protocol.hook_program)]
    pub hook_program: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// A customer spends issuer A's points at acceptor B.
///
/// **There is no collateral check on A here, and adding one would be the bug.**
///
/// Before this instruction, A's points were a *probabilistic* liability: most loyalty points are
/// never spent, so A backs them with a fractional reserve — 30% of face, say — and the invariant in
/// `issue_points` holds it to that. Redemption is the moment the probability collapses. The points
/// come home, the reserve against them is released, and in its place A owes a named creditor the
/// **full face value**, in full, today. A's health falls. It is *supposed* to fall: that is what it
/// means for a promise to be called in.
///
/// A protocol that refused the redemption because it would leave A under-collateralised would be
/// refusing to let A's own customer spend A's own promise, in order to keep A's ratio pretty. The
/// customer would be turned away at the till to protect the merchant from having issued the point.
/// So we let it through, we mark A's books honestly, and if A ends up unable to pay, `liquidate`
/// is permissionless and B — who read A's health before it bid, on chain, for free — knew exactly
/// what it was underwriting.
///
/// The movement itself is three CPIs and the middle one is the whole reason for the other two:
///
///   1. `grant_permit` — the core tells the hook: this source, this many, for a redemption.
///   2. `transfer_checked` into A's escrow — **Token-2022 fires the hook here**, and the hook
///      spends the permit. This is why the points take a detour through an escrow instead of being
///      burned where they sit: Token-2022 does not invoke a transfer hook on `Burn`. Burn the
///      points directly and the hook never runs, the permit is never spent, and the one component
///      that makes a point unmovable-by-default has been quietly routed around.
///   3. `burn` from the escrow — no hook, none needed; the points are already inside the protocol.
pub(crate) fn handler(ctx: Context<Redeem>, points: u64) -> Result<()> {
    require!(points > 0, ObligoError::InvalidAmount);

    let now = Clock::get()?.unix_timestamp;

    // A defaulted issuer's points are a claim in an estate, not a currency. Honouring one at face
    // would let this acceptor jump the queue ahead of the creditors already waiting on the same
    // collateral.
    require!(
        ctx.accounts.issuer.status == MerchantStatus::Active,
        ObligoError::IssuerDefaulted
    );

    // The clock kills the points, not the crank. `expire_points` reclaims the reserve and books
    // the breakage, but a customer cannot spend a lapsed point in the window before someone gets
    // around to turning it.
    let dead_at = ctx
        .accounts
        .batch
        .issued_at
        .checked_add(ctx.accounts.issuer.point_ttl)
        .ok_or(ObligoError::Overflow)?;
    require!(now < dead_at, ObligoError::PointsExpired);
    require!(
        ctx.accounts.batch.amount >= points,
        ObligoError::InsufficientPoints
    );

    // What B will claim from A: always 100% of face, whatever B bid.
    let value_face = face(points, ctx.accounts.issuer.usdc_per_point)?;

    let offer = &ctx.accounts.offer;
    require!(offer.expires_at > now, ObligoError::OfferExpired);

    // The acquisition budget, enforced. B drew this line itself; the chain holds it to it even
    // against B's own till.
    let consumed = offer
        .consumed
        .checked_add(value_face)
        .ok_or(ObligoError::Overflow)?;
    require!(consumed <= offer.capacity, ObligoError::OfferExhausted);

    // What B hands the customer. Above face, the difference is B's cost of acquiring a customer it
    // did not have; below face, it is B's discount on A's credit. B chose. Only the goods move on
    // this number — the claim on A does not.
    let goods_value = u64::try_from(
        (value_face as u128)
            .checked_mul(offer.rate_bps as u128)
            .ok_or(ObligoError::Overflow)?
            .checked_div(BPS)
            .ok_or(ObligoError::Overflow)?,
    )
    .map_err(|_| ObligoError::Overflow)?;

    let issuer_key = ctx.accounts.issuer.key();
    let acceptor_key = ctx.accounts.acceptor.key();
    let rate_bps = offer.rate_bps;

    // ---- the points move ------------------------------------------------------------------

    let authority_bump = ctx.accounts.protocol.authority_bump;
    let authority_seeds: &[&[u8]] = &[AUTHORITY_SEED, &[authority_bump]];

    hook_cpi::grant_permit(
        &ctx.accounts.hook_program.to_account_info(),
        &ctx.accounts.payer.to_account_info(),
        &ctx.accounts.core_authority.to_account_info(),
        &ctx.accounts.customer_points.to_account_info(),
        &ctx.accounts.permit.to_account_info(),
        &ctx.accounts.system_program.to_account_info(),
        hook_cpi::PERMIT_KIND_REDEEM,
        points,
        &[authority_seeds],
    )?;

    // The hook runs inside this call, and refuses the transfer outright if the permit above does
    // not cover it. The customer's own signature carries through: the points are theirs to spend.
    hook_cpi::transfer_points(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.customer_points.to_account_info(),
        &ctx.accounts.points_mint.to_account_info(),
        &ctx.accounts.redemption_escrow.to_account_info(),
        &ctx.accounts.customer.to_account_info(),
        &ctx.accounts.hook_program.to_account_info(),
        &ctx.accounts.extra_account_meta_list.to_account_info(),
        &ctx.accounts.permit.to_account_info(),
        points,
        ctx.accounts.points_mint.decimals,
        // No PDA signs this one: the customer signed the transaction, and these are their points.
        &[],
    )?;

    // Defence in depth: we granted a permit for exactly `points` and moved exactly `points`, so the
    // hook must have spent it to nothing. Assert it, so an over-grant could never leave a live bearer
    // authorization behind for some later instruction to pick up.
    require!(
        hook_cpi::permit_remaining(&ctx.accounts.permit.to_account_info())? == 0,
        ObligoError::PermitNotConsumed
    );

    let issuer_authority = ctx.accounts.issuer.authority;
    let issuer_bump = ctx.accounts.issuer.bump;
    let issuer_seeds: &[&[u8]] = &[MERCHANT_SEED, issuer_authority.as_ref(), &[issuer_bump]];

    burn(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Burn {
                mint: ctx.accounts.points_mint.to_account_info(),
                from: ctx.accounts.redemption_escrow.to_account_info(),
                authority: ctx.accounts.issuer.to_account_info(),
            },
            &[issuer_seeds],
        ),
        points,
    )?;

    // ---- the books --------------------------------------------------------------------------

    let issuer = &mut ctx.accounts.issuer;
    issuer.points_outstanding = issuer
        .points_outstanding
        .checked_sub(points)
        .ok_or(ObligoError::Overflow)?;
    issuer.total_redeemed = issuer
        .total_redeemed
        .checked_add(points)
        .ok_or(ObligoError::Overflow)?;
    // Reserve-backed liability becomes debt. In full, and to a name.
    issuer.obligations_out = issuer
        .obligations_out
        .checked_add(value_face)
        .ok_or(ObligoError::Overflow)?;

    let acceptor = &mut ctx.accounts.acceptor;
    acceptor.obligations_in = acceptor
        .obligations_in
        .checked_add(value_face)
        .ok_or(ObligoError::Overflow)?;

    let obligation = &mut ctx.accounts.obligation;
    obligation.debtor = issuer_key;
    obligation.creditor = acceptor_key;
    obligation.amount = obligation
        .amount
        .checked_add(value_face)
        .ok_or(ObligoError::Overflow)?;
    obligation.bump = ctx.bumps.obligation;
    let edge_total = obligation.amount;

    ctx.accounts.offer.consumed = consumed;

    let batch = &mut ctx.accounts.batch;
    batch.amount = batch
        .amount
        .checked_sub(points)
        .ok_or(ObligoError::Overflow)?;

    emit!(Redeemed {
        issuer: issuer_key,
        acceptor: acceptor_key,
        customer: ctx.accounts.customer.key(),
        points,
        value_face,
        goods_value,
        rate_bps,
        obligation: edge_total,
    });

    Ok(())
}
