use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};

use crate::constants::MERCHANT_SEED;
use crate::error::ObligoError;
use crate::state::Merchant;

/// Permissionless. Anyone may top up any merchant's vault — a merchant's backers, its franchisor,
/// or a creditor with an interest in keeping it solvent. Only the merchant can ever take it out.
#[derive(Accounts)]
pub struct DepositCollateral<'info> {
    pub depositor: Signer<'info>,

    #[account(
        mut,
        seeds = [MERCHANT_SEED, merchant.authority.as_ref()],
        bump = merchant.bump,
        has_one = vault,
    )]
    pub merchant: Account<'info, Merchant>,

    #[account(mut)]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    #[account(address = vault.mint)]
    pub usdc_mint: InterfaceAccount<'info, Mint>,

    #[account(mut, token::mint = usdc_mint, token::authority = depositor)]
    pub from: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub(crate) fn handler(ctx: Context<DepositCollateral>, amount: u64) -> Result<()> {
    require!(amount > 0, ObligoError::InvalidAmount);

    transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            TransferChecked {
                from: ctx.accounts.from.to_account_info(),
                mint: ctx.accounts.usdc_mint.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
                authority: ctx.accounts.depositor.to_account_info(),
            },
        ),
        amount,
        ctx.accounts.usdc_mint.decimals,
    )?;

    let merchant = &mut ctx.accounts.merchant;
    merchant.collateral = merchant
        .collateral
        .checked_add(amount)
        .ok_or(ObligoError::Overflow)?;

    Ok(())
}
