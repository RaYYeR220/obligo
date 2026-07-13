use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};

use crate::constants::{MERCHANT_SEED, OBLIGATION_SEED, PROTOCOL_SEED};
use crate::error::ObligoError;
use crate::events::Settled;
use crate::math::is_solvent;
use crate::state::{Merchant, MerchantStatus, Obligation, Protocol};

/// Boxed: two merchants, two vaults, two edges and a mint is enough state to matter, and the SBF
/// stack is 4KB whether or not Anchor's generated `try_accounts` cares.
#[derive(Accounts)]
pub struct Settle<'info> {
    /// Anybody at all. See the handler.
    #[account(mut)]
    pub cranker: Signer<'info>,

    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Box<Account<'info, Protocol>>,

    #[account(
        mut,
        seeds = [MERCHANT_SEED, merchant_a.authority.as_ref()],
        bump = merchant_a.bump,
    )]
    pub merchant_a: Box<Account<'info, Merchant>>,

    /// The two must be different merchants. A self-edge cannot exist — `post_offer` refuses an
    /// acceptor bidding on its own points — but if one ever did, `edge_ab` and `edge_ba` would
    /// derive to the same address and the same account would be loaded twice.
    #[account(
        mut,
        seeds = [MERCHANT_SEED, merchant_b.authority.as_ref()],
        bump = merchant_b.bump,
        constraint = merchant_b.key() != merchant_a.key() @ ObligoError::InvalidCycle,
    )]
    pub merchant_b: Box<Account<'info, Merchant>>,

    #[account(mut, address = merchant_a.vault)]
    pub vault_a: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(mut, address = merchant_b.vault)]
    pub vault_b: Box<InterfaceAccount<'info, TokenAccount>>,

    /// What A owes B. It has to exist: with no edge in this direction there is nothing to settle,
    /// and the caller should have named the pair the other way round.
    #[account(
        mut,
        seeds = [OBLIGATION_SEED, merchant_a.key().as_ref(), merchant_b.key().as_ref()],
        bump = edge_ab.bump,
    )]
    pub edge_ab: Box<Account<'info, Obligation>>,

    /// What B owes A — which may be nothing, and may never have been anything.
    ///
    /// It is created here, at the cranker's expense, rather than being made optional, and that is
    /// a deliberate and slightly expensive choice. An optional account would mean taking the
    /// caller's word for it that no counter-claim exists. It does not cost an attacker anything to
    /// say that: hide a live `B -> A` edge, and A pays B *gross* instead of net — draining A's
    /// vault down to a debt other creditors have an equal claim on, and pushing a solvent merchant
    /// under so it can be liquidated. The seeds are the only thing that can prove a negative here,
    /// so the account is always present, always derived, and reads zero when there is nothing.
    #[account(
        init_if_needed,
        payer = cranker,
        space = 8 + Obligation::INIT_SPACE,
        seeds = [OBLIGATION_SEED, merchant_b.key().as_ref(), merchant_a.key().as_ref()],
        bump
    )]
    pub edge_ba: Box<Account<'info, Obligation>>,

    #[account(address = protocol.usdc_mint)]
    pub usdc_mint: Box<InterfaceAccount<'info, Mint>>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

/// Net two merchants' mutual debt and move only what is left over.
///
/// Permissionless, and it has to be. Settlement makes the debtor healthier and the creditor
/// wealthier; it takes nothing from anybody. A settlement only the creditor could trigger would be
/// a settlement the creditor could *sit on* — holding a claim open against a rival's balance sheet
/// is a strategy, and the network should not have to hope nobody plays it. So it is a crank, and
/// the crank is a public good: whoever turns it gets nothing, and everybody's books get shorter.
///
/// The number that matters is the one that does not move. Two merchants owing each other $10 and
/// $8 do not need $18 of liquidity between them; they need $2. The other $16 is debt cancelling
/// against debt, and it never touches a vault.
///
/// `paid = min(net, collateral)`. A debtor that cannot cover the net is not refused and is not
/// forgiven: it pays everything it has, the residual stays on the edge with the creditor's name on
/// it, and the merchant is now visibly insolvent. That is `liquidate`'s business, not this
/// instruction's.
pub(crate) fn handler(ctx: Context<Settle>) -> Result<()> {
    let a_key = ctx.accounts.merchant_a.key();
    let b_key = ctx.accounts.merchant_b.key();

    // Stamps a freshly created reverse edge, and rewrites the same three values onto an existing
    // one. The seeds pinned the address before we got here, so there is nothing else they could be.
    {
        let edge_ba = &mut ctx.accounts.edge_ba;
        edge_ba.debtor = b_key;
        edge_ba.creditor = a_key;
        edge_ba.bump = ctx.bumps.edge_ba;
    }

    let owed_ab = ctx.accounts.edge_ab.amount;
    let owed_ba = ctx.accounts.edge_ba.amount;
    require!(owed_ab > 0 || owed_ba > 0, ObligoError::NothingToSettle);

    // Debt that cancels against debt, and the remainder that cannot.
    let offset = owed_ab.min(owed_ba);
    let net = owed_ab.abs_diff(owed_ba);

    // The graph decides who pays, not the order of the arguments.
    let a_owes_more = owed_ab >= owed_ba;

    let (debtor_key, creditor_key) = if a_owes_more {
        (a_key, b_key)
    } else {
        (b_key, a_key)
    };
    let (status, collateral, obligations_out, debtor_authority, debtor_bump) = if a_owes_more {
        let m = &ctx.accounts.merchant_a;
        (
            m.status,
            m.collateral,
            m.obligations_out,
            m.authority,
            m.bump,
        )
    } else {
        let m = &ctx.accounts.merchant_b;
        (
            m.status,
            m.collateral,
            m.obligations_out,
            m.authority,
            m.bump,
        )
    };

    // A defaulted merchant's collateral is an estate, and it belongs to all of its creditors in
    // proportion — that is what `liquidate` is for. Left open, this instruction would be a
    // preference: the first creditor to crank it takes the vault up to its own claim and everyone
    // behind it finds the cupboard bare. The *creditor's* status is nobody's business here; money
    // arriving in a defaulted merchant's vault only helps the people it owes.
    require!(
        status == MerchantStatus::Active,
        ObligoError::MerchantDefaulted
    );

    // The same estate reasoning applies before the `Defaulted` flag is ever set. A merchant becomes
    // insolvent the instant a redemption pushes `obligations_out` past its collateral, and it stays
    // `Active` until somebody troubles to `liquidate` it. In that window paying one creditor out of
    // the vault is the very preference we refuse a defaulted merchant — and the identical action
    // through `withdraw_collateral` is already gated on solvency, so leaving it open here is just a
    // side door onto the same estate. So: settle the cash leg only while the debtor can cover every
    // debt it owes. The `offset` below is symmetric debt cancelling symmetric debt and moves no
    // money, so it is always safe and is applied regardless.
    let paid = if is_solvent(collateral, obligations_out) {
        // solvent ⇒ collateral ≥ obligations_out ≥ net, so the full net still leaves every other
        // creditor covered.
        net.min(collateral)
    } else {
        0
    };
    let cleared = offset.checked_add(paid).ok_or(ObligoError::Overflow)?;

    if paid > 0 {
        let (from, to, authority) = if a_owes_more {
            (
                ctx.accounts.vault_a.to_account_info(),
                ctx.accounts.vault_b.to_account_info(),
                ctx.accounts.merchant_a.to_account_info(),
            )
        } else {
            (
                ctx.accounts.vault_b.to_account_info(),
                ctx.accounts.vault_a.to_account_info(),
                ctx.accounts.merchant_b.to_account_info(),
            )
        };

        let seeds: &[&[u8]] = &[MERCHANT_SEED, debtor_authority.as_ref(), &[debtor_bump]];

        transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                TransferChecked {
                    from,
                    mint: ctx.accounts.usdc_mint.to_account_info(),
                    to,
                    authority,
                },
                &[seeds],
            ),
            paid,
            ctx.accounts.usdc_mint.decimals,
        )?;
    }

    // The books. The debtor's edge loses everything that was settled — the part that cancelled and
    // the part that was paid. The creditor's edge loses the part that cancelled, which is all of
    // it: the smaller of two mutual debts always goes to zero.
    let residual = if a_owes_more {
        let edge = &mut ctx.accounts.edge_ab;
        edge.amount = edge
            .amount
            .checked_sub(cleared)
            .ok_or(ObligoError::Overflow)?;
        let residual = edge.amount;

        let edge = &mut ctx.accounts.edge_ba;
        edge.amount = edge
            .amount
            .checked_sub(offset)
            .ok_or(ObligoError::Overflow)?;

        let a = &mut ctx.accounts.merchant_a;
        a.obligations_out = a
            .obligations_out
            .checked_sub(cleared)
            .ok_or(ObligoError::Overflow)?;
        a.obligations_in = a
            .obligations_in
            .checked_sub(offset)
            .ok_or(ObligoError::Overflow)?;
        a.collateral = a
            .collateral
            .checked_sub(paid)
            .ok_or(ObligoError::Overflow)?;

        let b = &mut ctx.accounts.merchant_b;
        b.obligations_in = b
            .obligations_in
            .checked_sub(cleared)
            .ok_or(ObligoError::Overflow)?;
        b.obligations_out = b
            .obligations_out
            .checked_sub(offset)
            .ok_or(ObligoError::Overflow)?;
        b.collateral = b
            .collateral
            .checked_add(paid)
            .ok_or(ObligoError::Overflow)?;

        residual
    } else {
        let edge = &mut ctx.accounts.edge_ba;
        edge.amount = edge
            .amount
            .checked_sub(cleared)
            .ok_or(ObligoError::Overflow)?;
        let residual = edge.amount;

        let edge = &mut ctx.accounts.edge_ab;
        edge.amount = edge
            .amount
            .checked_sub(offset)
            .ok_or(ObligoError::Overflow)?;

        let b = &mut ctx.accounts.merchant_b;
        b.obligations_out = b
            .obligations_out
            .checked_sub(cleared)
            .ok_or(ObligoError::Overflow)?;
        b.obligations_in = b
            .obligations_in
            .checked_sub(offset)
            .ok_or(ObligoError::Overflow)?;
        b.collateral = b
            .collateral
            .checked_sub(paid)
            .ok_or(ObligoError::Overflow)?;

        let a = &mut ctx.accounts.merchant_a;
        a.obligations_in = a
            .obligations_in
            .checked_sub(cleared)
            .ok_or(ObligoError::Overflow)?;
        a.obligations_out = a
            .obligations_out
            .checked_sub(offset)
            .ok_or(ObligoError::Overflow)?;
        a.collateral = a
            .collateral
            .checked_add(paid)
            .ok_or(ObligoError::Overflow)?;

        residual
    };

    emit!(Settled {
        debtor: debtor_key,
        creditor: creditor_key,
        offset,
        paid,
        residual,
    });

    Ok(())
}
