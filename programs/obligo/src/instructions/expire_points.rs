use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface::{burn, Burn, Mint, TokenAccount};

use crate::constants::{AUTHORITY_SEED, BATCH_SEED, MERCHANT_SEED, POINTS_SEED, PROTOCOL_SEED};
use crate::error::ObligoError;
use crate::events::Breakage;
use crate::hook_cpi;
use crate::math::face;
use crate::state::{Merchant, PointBatch, Protocol};

/// Boxed for the same reason `redeem` is: fifteen accounts, several of them Token-2022 states with
/// extensions, will not fit in the 4KB an SBF stack frame gets, and the toolchain does not stop you.
#[derive(Accounts)]
pub struct ExpirePoints<'info> {
    /// Anybody. Pays for the permit and, on the off-chance nothing has ever been redeemed from this
    /// merchant, the escrow.
    #[account(mut)]
    pub cranker: Signer<'info>,

    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Box<Account<'info, Protocol>>,

    /// Not gated on status. A defaulted merchant's points can *only* be expired — redemption is
    /// shut to them — and refusing to expire them would leave a dead liability on the books of an
    /// estate that is trying to close.
    #[account(
        mut,
        seeds = [MERCHANT_SEED, merchant.authority.as_ref()],
        bump = merchant.bump,
        has_one = points_mint,
    )]
    pub merchant: Box<Account<'info, Merchant>>,

    #[account(
        mut,
        seeds = [POINTS_SEED, merchant.key().as_ref()],
        bump = merchant.mint_bump,
        mint::token_program = token_program,
    )]
    pub points_mint: Box<InterfaceAccount<'info, Mint>>,

    /// CHECK: the customer whose points have lapsed. Nothing is read from it: its key derives the
    /// batch and the account the points sit in. **It does not sign, and that is the instruction.**
    pub customer: UncheckedAccount<'info>,

    #[account(
        mut,
        associated_token::mint = points_mint,
        associated_token::authority = customer,
        associated_token::token_program = token_program,
    )]
    pub customer_points: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The same turnstile a redemption uses. Zero before, zero after.
    #[account(
        init_if_needed,
        payer = cranker,
        associated_token::mint = points_mint,
        associated_token::authority = merchant,
        associated_token::token_program = token_program,
    )]
    pub redemption_escrow: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [BATCH_SEED, merchant.key().as_ref(), customer.key().as_ref()],
        bump = batch.bump,
    )]
    pub batch: Box<Account<'info, PointBatch>>,

    /// CHECK: the program's signer PDA, and the only signature the hook will grant a permit to.
    #[account(seeds = [AUTHORITY_SEED], bump = protocol.authority_bump)]
    pub core_authority: UncheckedAccount<'info>,

    /// CHECK: `[b"permit", customer_points]` under the hook program, which pins the address when it
    /// grants the permit and re-derives it from a stored bump when it spends it.
    #[account(mut)]
    pub permit: UncheckedAccount<'info>,

    /// CHECK: the mint's ExtraAccountMetaList. Token-2022 derives the address it expects and looks
    /// it up among the accounts we hand it; a substitute is simply not found.
    pub extra_account_meta_list: UncheckedAccount<'info>,

    /// CHECK: the hook the protocol was born with. Fixed at genesis and never updatable.
    #[account(executable, address = protocol.hook_program)]
    pub hook_program: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Burn a customer's lapsed points and release the reserve behind them.
///
/// Breakage — points issued and never spent — is the oldest revenue line in retail. It is normally
/// recognised in a back office, on a schedule nobody outside the company sees, and the liability
/// simply stops being mentioned. Here it is an instruction, **anyone** may call it, it may not be
/// called a second before the deadline the merchant itself published, and it emits an event saying
/// exactly how much of a promise was just cancelled and what it was worth. That is the same
/// accounting, done where the people holding the points can watch.
///
/// The reserve behind those points is freed as a matter of arithmetic, not policy: `points_
/// outstanding` falls, so `required_collateral` falls, so the merchant may now withdraw collateral
/// that a minute ago it could not. That is breakage recognised as revenue, and it is the merchant's
/// own incentive to turn this crank — which is why the crank needs no bounty attached to it.
///
/// **Why the points take a detour through the escrow instead of being burned where they sit.**
/// Token-2022 does not invoke a transfer hook on `Burn`. Burn a customer's points directly and the
/// hook never runs, no permit is ever asked for or spent, and the one component that makes a point
/// unmovable-by-default has been quietly routed around by the protocol that built it. So expiry
/// moves the points — a real `transfer_checked`, with the hook on the critical path, under a permit
/// of kind `Expire` — and only then burns them from an account the protocol owns. The hook governs
/// **every** movement of a point, including the protocol's own.
///
/// **Who signs for the customer's points.** Nobody. The customer will not sign for the destruction
/// of their own lapsed points, and a crank that needed them to would not be a crank. The merchant
/// PDA moves them as the mint's `PermanentDelegate` — an authority with no private key, reachable
/// only from inside this program, and used as a transfer authority in this one instruction, behind
/// the TTL check below. It still needs a permit from the hook like everybody else.
pub(crate) fn handler(ctx: Context<ExpirePoints>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    // The TTL runs from the customer's last activity, which `issue_points` stamps on the batch.
    // `redeem` refuses on `now >= dead_at`; this one requires it. There is no gap between them and
    // no overlap: at the instant a point dies it stops being spendable and starts being expirable.
    let dead_at = ctx
        .accounts
        .batch
        .issued_at
        .checked_add(ctx.accounts.merchant.point_ttl)
        .ok_or(ObligoError::Overflow)?;
    require!(now >= dead_at, ObligoError::NotYetExpired);

    let points = ctx.accounts.batch.amount;
    require!(points > 0, ObligoError::InsufficientPoints);
    // The books and the token account should never disagree — points cannot move without a permit,
    // and no permit is ever granted for anything but this instruction and a redemption, both of
    // which write both sides. If they ever do disagree, fail loudly rather than burn a number.
    require!(
        ctx.accounts.customer_points.amount >= points,
        ObligoError::InsufficientPoints
    );

    let face_value = face(points, ctx.accounts.merchant.usdc_per_point)?;

    // ---- the points move ------------------------------------------------------------------

    let authority_bump = ctx.accounts.protocol.authority_bump;
    let authority_seeds: &[&[u8]] = &[AUTHORITY_SEED, &[authority_bump]];

    hook_cpi::grant_permit(
        &ctx.accounts.hook_program.to_account_info(),
        &ctx.accounts.cranker.to_account_info(),
        &ctx.accounts.core_authority.to_account_info(),
        &ctx.accounts.customer_points.to_account_info(),
        &ctx.accounts.permit.to_account_info(),
        &ctx.accounts.system_program.to_account_info(),
        hook_cpi::PERMIT_KIND_EXPIRE,
        points,
        &[authority_seeds],
    )?;

    let merchant_key = ctx.accounts.merchant.key();
    let merchant_authority = ctx.accounts.merchant.authority;
    let merchant_bump = ctx.accounts.merchant.bump;
    let merchant_seeds: &[&[u8]] = &[MERCHANT_SEED, merchant_authority.as_ref(), &[merchant_bump]];

    // The merchant PDA signs as the mint's permanent delegate. The hook still runs, still checks
    // the permit, and would still refuse the movement if the core had not granted one.
    hook_cpi::transfer_points(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.customer_points.to_account_info(),
        &ctx.accounts.points_mint.to_account_info(),
        &ctx.accounts.redemption_escrow.to_account_info(),
        &ctx.accounts.merchant.to_account_info(),
        &ctx.accounts.hook_program.to_account_info(),
        &ctx.accounts.extra_account_meta_list.to_account_info(),
        &ctx.accounts.permit.to_account_info(),
        points,
        ctx.accounts.points_mint.decimals,
        &[merchant_seeds],
    )?;

    burn(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Burn {
                mint: ctx.accounts.points_mint.to_account_info(),
                from: ctx.accounts.redemption_escrow.to_account_info(),
                authority: ctx.accounts.merchant.to_account_info(),
            },
            &[merchant_seeds],
        ),
        points,
    )?;

    // ---- the books --------------------------------------------------------------------------

    let merchant = &mut ctx.accounts.merchant;
    merchant.points_outstanding = merchant
        .points_outstanding
        .checked_sub(points)
        .ok_or(ObligoError::Overflow)?;
    merchant.total_expired = merchant
        .total_expired
        .checked_add(points)
        .ok_or(ObligoError::Overflow)?;

    // The batch is left in place at zero rather than closed. It costs the merchant nothing it has
    // not already spent, and `issue_points` will reuse it — with a fresh `issued_at` — the next time
    // this customer walks in. A crank that closed accounts for rent would be a crank with an
    // incentive to race the customer to the door.
    ctx.accounts.batch.amount = 0;

    emit!(Breakage {
        merchant: merchant_key,
        customer: ctx.accounts.customer.key(),
        points,
        face_value,
    });

    Ok(())
}
