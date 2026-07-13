use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};

use crate::constants::{MERCHANT_SEED, OBLIGATION_SEED, PROTOCOL_SEED};
use crate::error::ObligoError;
use crate::events::Liquidated;
use crate::math::is_solvent;
use crate::state::{Merchant, MerchantStatus, Obligation, Protocol};

/// Boxed. Two merchants, two vaults, an edge, a mint and the protocol is more state than a 4KB SBF
/// stack frame wants to hold, and the failure mode if it does not fit is not a compile error.
#[derive(Accounts)]
pub struct Liquidate<'info> {
    /// Anybody at all — see the handler. Nothing is created, so nothing is paid for.
    pub cranker: Signer<'info>,

    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Box<Account<'info, Protocol>>,

    /// The merchant that cannot pay.
    #[account(
        mut,
        seeds = [MERCHANT_SEED, debtor.authority.as_ref()],
        bump = debtor.bump,
    )]
    pub debtor: Box<Account<'info, Merchant>>,

    /// The merchant being paid out. It does not sign: a creditor should not have to be online to
    /// be paid, and an estate should not wait on it.
    #[account(
        mut,
        seeds = [MERCHANT_SEED, creditor.authority.as_ref()],
        bump = creditor.bump,
        constraint = creditor.key() != debtor.key() @ ObligoError::NoClaim,
    )]
    pub creditor: Box<Account<'info, Merchant>>,

    #[account(mut, address = debtor.vault)]
    pub debtor_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(mut, address = creditor.vault)]
    pub creditor_vault: Box<InterfaceAccount<'info, TokenAccount>>,

    /// The claim being settled. Pinned by seeds, so the caller cannot present somebody else's
    /// claim and collect on it.
    #[account(
        mut,
        seeds = [OBLIGATION_SEED, debtor.key().as_ref(), creditor.key().as_ref()],
        bump = edge.bump,
    )]
    pub edge: Box<Account<'info, Obligation>>,

    #[account(address = protocol.usdc_mint)]
    pub usdc_mint: Box<InterfaceAccount<'info, Mint>>,

    pub token_program: Interface<'info, TokenInterface>,
}

/// Distribute an insolvent issuer's estate to one of its creditors, pro rata.
///
/// **Permissionless, and it must be.** A liquidation that only the debtor could authorise is not a
/// liquidation, and one that only the creditor could authorise is a race — the fastest creditor
/// takes the vault up to its own claim and the slow ones find the cupboard bare. So anyone may call
/// it, for any creditor, and the arithmetic is the same whoever does.
///
/// The trigger is solvency, not health. Health can and should fall below 1.0 all day long: a
/// merchant running a 30% reserve is *supposed* to have issued more points than it can instantly
/// cover, and a redemption is *supposed* to hurt. Insolvency is a harder fact — the merchant owes
/// named creditors, today, more USDC than it holds — and there is nothing to wait for.
///
/// ```text
/// paid = claim * collateral / obligations_out      (floor, in u128)
/// ```
///
/// The sum of a merchant's outgoing edges *is* its `obligations_out` — every redemption adds the
/// same face value to both — so `claim <= obligations_out` and therefore `paid <= collateral`,
/// always. And because each liquidation removes `claim` from `obligations_out` and `paid` from
/// `collateral`, the same relation holds for the next creditor, and the next: **the estate can never
/// pay out more than it holds, and the rounding dust stays in the vault rather than being conjured
/// out of it.** The invariant suite asserts exactly that, because it is the one an auditor will
/// look for first.
///
/// The creditor's claim is then discharged **in full**, whatever it recovered. That is the part
/// that deserves an argument, because it means the creditor eats the difference:
///
/// - It is what a pro-rata distribution *means*. Paying $1.50 against a $6.00 claim and leaving
///   $4.50 on the edge would let the next creditor's share be computed against a debt that has
///   already been partly paid, and the second creditor would get less than the first for an
///   identical claim. Pro rata would stop being pro rata after the first payout.
/// - The alternative — debt that survives the estate — is a fiction here anyway. A merchant is a
///   keypair. A debt that outlives a merchant's collateral is a debt it escapes by registering
///   again with a new keypair, and a protocol that pretends otherwise is lying to its creditors
///   about the protection they have.
///
/// So the loss is booked where it actually fell: on the acceptor, which read the issuer's health on
/// chain before it bid, chose its own `rate_bps`, and capped its own exposure with `capacity`. That
/// is what those three numbers are *for*. And the default is written into the debtor's account
/// permanently — `defaults` never goes down, and `reinstate` does not touch it.
pub(crate) fn handler(ctx: Context<Liquidate>) -> Result<()> {
    let collateral = ctx.accounts.debtor.collateral;
    let obligations_out = ctx.accounts.debtor.obligations_out;

    // Insolvency implies `obligations_out > collateral >= 0`, so the divisor below is never zero.
    require!(
        !is_solvent(collateral, obligations_out),
        ObligoError::NotLiquidatable
    );

    let claim = ctx.accounts.edge.amount;
    require!(claim > 0, ObligoError::NoClaim);

    let paid = u64::try_from(
        (claim as u128)
            .checked_mul(collateral as u128)
            .ok_or(ObligoError::Overflow)?
            .checked_div(obligations_out as u128)
            .ok_or(ObligoError::Overflow)?,
    )
    .map_err(|_| ObligoError::Overflow)?;

    // `claim <= obligations_out` makes this unreachable. It is here because the day it is not
    // unreachable is the day this program pays a creditor with another creditor's money, and a
    // failed transaction is a much better outcome than that.
    require!(paid <= collateral, ObligoError::Overflow);

    if paid > 0 {
        let debtor_authority = ctx.accounts.debtor.authority;
        let debtor_bump = ctx.accounts.debtor.bump;
        let seeds: &[&[u8]] = &[MERCHANT_SEED, debtor_authority.as_ref(), &[debtor_bump]];

        transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                TransferChecked {
                    from: ctx.accounts.debtor_vault.to_account_info(),
                    mint: ctx.accounts.usdc_mint.to_account_info(),
                    to: ctx.accounts.creditor_vault.to_account_info(),
                    authority: ctx.accounts.debtor.to_account_info(),
                },
                &[seeds],
            ),
            paid,
            ctx.accounts.usdc_mint.decimals,
        )?;
    }

    let written_off = claim.checked_sub(paid).ok_or(ObligoError::Overflow)?;

    let debtor = &mut ctx.accounts.debtor;
    debtor.collateral = debtor
        .collateral
        .checked_sub(paid)
        .ok_or(ObligoError::Overflow)?;
    debtor.obligations_out = debtor
        .obligations_out
        .checked_sub(claim)
        .ok_or(ObligoError::Overflow)?;

    // One default, however many creditors turn up to collect on it. A merchant can only become
    // insolvent again by first being reinstated, which needs it to be solvent — so the transition
    // here is the whole event.
    if debtor.status != MerchantStatus::Defaulted {
        debtor.status = MerchantStatus::Defaulted;
        debtor.defaults = debtor
            .defaults
            .checked_add(1)
            .ok_or(ObligoError::Overflow)?;
    }

    let collateral_remaining = debtor.collateral;
    let obligations_remaining = debtor.obligations_out;
    let debtor_key = debtor.key();

    let creditor = &mut ctx.accounts.creditor;
    creditor.obligations_in = creditor
        .obligations_in
        .checked_sub(claim)
        .ok_or(ObligoError::Overflow)?;
    creditor.collateral = creditor
        .collateral
        .checked_add(paid)
        .ok_or(ObligoError::Overflow)?;
    let creditor_key = creditor.key();

    ctx.accounts.edge.amount = 0;

    emit!(Liquidated {
        debtor: debtor_key,
        creditor: creditor_key,
        claim,
        paid,
        written_off,
        collateral_remaining,
        obligations_remaining,
    });

    Ok(())
}
