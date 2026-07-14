use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};

use crate::constants::MERCHANT_SEED;
use crate::error::ObligoError;
use crate::state::Merchant;
use crate::yield_adapter::YieldAdapter;

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

pub(crate) fn handler<'info>(
    ctx: Context<'info, DepositCollateral<'info>>,
    amount: u64,
) -> Result<()> {
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

    // Route the freshly-deposited USDC through the yield seam. With the default NullAdapter this is
    // a no-op — the USDC rests in the vault exactly as before — so the on-chain effect is byte-for
    // -byte today's. The merchant PDA is the vault's authority; the Kamino path signs KLend CPIs
    // with its seeds, the Null path ignores them.
    let vault = ctx.accounts.vault.to_account_info();
    let owner = ctx.accounts.merchant.to_account_info();
    let authority = ctx.accounts.merchant.authority;
    let bump = ctx.accounts.merchant.bump;
    let seeds: &[&[u8]] = &[MERCHANT_SEED, authority.as_ref(), &[bump]];
    let adapter =
        crate::yield_adapter::vault_adapter(&vault, ctx.remaining_accounts, &owner, seeds)?;
    adapter.deposit(amount)?;

    let merchant = &mut ctx.accounts.merchant;
    merchant.collateral = merchant
        .collateral
        .checked_add(amount)
        .ok_or(ObligoError::Overflow)?;

    Ok(())
}
