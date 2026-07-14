use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};

use crate::constants::MERCHANT_SEED;
use crate::error::ObligoError;
use crate::math::required_collateral;
use crate::state::Merchant;
use crate::yield_adapter::YieldAdapter;

/// The merchant's own authority, and nobody else's.
///
/// The merchant PDA is derived from this signer, so there is no way to name someone else's
/// merchant account here: a different signer derives a different PDA and the seeds check fails
/// before a single lamport moves. The protocol authority has no privileged path in — it is not
/// a party to this instruction at all, which is the difference between a protocol and a
/// custodian.
#[derive(Accounts)]
pub struct WithdrawCollateral<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [MERCHANT_SEED, authority.key().as_ref()],
        bump = merchant.bump,
        has_one = authority,
        has_one = vault,
    )]
    pub merchant: Account<'info, Merchant>,

    #[account(mut)]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    #[account(address = vault.mint)]
    pub usdc_mint: InterfaceAccount<'info, Mint>,

    #[account(mut, token::mint = usdc_mint)]
    pub destination: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub(crate) fn handler<'info>(
    ctx: Context<'info, WithdrawCollateral<'info>>,
    amount: u64,
) -> Result<()> {
    require!(amount > 0, ObligoError::InvalidAmount);

    let remaining = ctx
        .accounts
        .merchant
        .collateral
        .checked_sub(amount)
        .ok_or(ObligoError::InsufficientCollateral)?;

    // The invariant is re-checked against the books as they would stand AFTER the withdrawal.
    // Checking it beforehand would let a merchant walk out with the reserve backing every point
    // it has issued. It is measured on principal — `collateral` — and never on principal + yield,
    // so a yield-earning vault can never issue against interest it has not realised.
    let required = required_collateral(
        ctx.accounts.merchant.obligations_out,
        ctx.accounts.merchant.points_outstanding,
        ctx.accounts.merchant.usdc_per_point,
        ctx.accounts.merchant.reserve_bps,
    )?;
    require!(remaining >= required, ObligoError::ReserveBreached);

    ctx.accounts.merchant.collateral = remaining;

    let authority = ctx.accounts.merchant.authority;
    let bump = ctx.accounts.merchant.bump;
    let seeds: &[&[u8]] = &[MERCHANT_SEED, authority.as_ref(), &[bump]];

    // Bring the principal back into the vault before it leaves it. NullAdapter passthrough: it is
    // already there. Kamino: redeem the cTokens KLend is holding back into USDC first, which is
    // where the accrued interest is realised.
    let vault = ctx.accounts.vault.to_account_info();
    let owner = ctx.accounts.merchant.to_account_info();
    let adapter =
        crate::yield_adapter::vault_adapter(&vault, ctx.remaining_accounts, &owner, seeds)?;
    adapter.withdraw(amount)?;

    transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            TransferChecked {
                from: ctx.accounts.vault.to_account_info(),
                mint: ctx.accounts.usdc_mint.to_account_info(),
                to: ctx.accounts.destination.to_account_info(),
                authority: ctx.accounts.merchant.to_account_info(),
            },
            &[seeds],
        ),
        amount,
        ctx.accounts.usdc_mint.decimals,
    )?;

    Ok(())
}
