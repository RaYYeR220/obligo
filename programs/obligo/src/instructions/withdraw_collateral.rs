use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};

use crate::constants::MERCHANT_SEED;
use crate::error::ObligoError;
use crate::math::required_collateral;
use crate::state::Merchant;

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

pub(crate) fn handler(ctx: Context<WithdrawCollateral>, amount: u64) -> Result<()> {
    require!(amount > 0, ObligoError::InvalidAmount);

    let merchant = &mut ctx.accounts.merchant;

    let remaining = merchant
        .collateral
        .checked_sub(amount)
        .ok_or(ObligoError::InsufficientCollateral)?;

    // The invariant is re-checked against the books as they would stand AFTER the withdrawal.
    // Checking it beforehand would let a merchant walk out with the reserve backing every point
    // it has issued.
    let required = required_collateral(
        merchant.obligations_out,
        merchant.points_outstanding,
        merchant.usdc_per_point,
        merchant.reserve_bps,
    )?;
    require!(remaining >= required, ObligoError::ReserveBreached);

    merchant.collateral = remaining;

    let authority = merchant.authority;
    let bump = merchant.bump;
    let seeds: &[&[u8]] = &[MERCHANT_SEED, authority.as_ref(), &[bump]];

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
