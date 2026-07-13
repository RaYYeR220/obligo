use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::constants::{MAX_NAME_LEN, MERCHANT_SEED, PROTOCOL_SEED, VAULT_SEED};
use crate::error::ObligoError;
use crate::state::{Merchant, MerchantStatus, Protocol};

/// Permissionless. Anyone may become an issuer; nobody approves them. What restrains a merchant
/// is not a whitelist, it is the reserve invariant and the fact that its health is public.
#[derive(Accounts)]
pub struct RegisterMerchant<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut, seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Account<'info, Protocol>,

    #[account(
        init,
        payer = authority,
        space = 8 + Merchant::INIT_SPACE,
        seeds = [MERCHANT_SEED, authority.key().as_ref()],
        bump
    )]
    pub merchant: Account<'info, Merchant>,

    #[account(address = protocol.usdc_mint)]
    pub usdc_mint: InterfaceAccount<'info, Mint>,

    /// The collateral vault. Owned by the merchant PDA, so the only signature that can ever move
    /// USDC out of it is one this program produces on the merchant's behalf.
    #[account(
        init,
        payer = authority,
        seeds = [VAULT_SEED, merchant.key().as_ref()],
        bump,
        token::mint = usdc_mint,
        token::authority = merchant,
        token::token_program = token_program,
    )]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub(crate) fn handler(
    ctx: Context<RegisterMerchant>,
    name: String,
    usdc_per_point: u64,
    reserve_bps: u16,
    point_ttl: i64,
) -> Result<()> {
    require!(name.len() <= MAX_NAME_LEN, ObligoError::NameTooLong);
    crate::instructions::set_terms::validate_terms(usdc_per_point, reserve_bps, point_ttl)?;

    let merchant = &mut ctx.accounts.merchant;
    merchant.set_inner(Merchant {
        authority: ctx.accounts.authority.key(),
        // No mint yet. `create_points_mint` fills this in, and `issue_points` is unreachable
        // until it does.
        points_mint: Pubkey::default(),
        vault: ctx.accounts.vault.key(),
        name,
        usdc_per_point,
        reserve_bps,
        point_ttl,
        collateral: 0,
        points_outstanding: 0,
        obligations_out: 0,
        obligations_in: 0,
        total_issued: 0,
        total_redeemed: 0,
        total_expired: 0,
        status: MerchantStatus::Active,
        defaults: 0,
        bump: ctx.bumps.merchant,
        vault_bump: ctx.bumps.vault,
        mint_bump: 0,
    });

    let protocol = &mut ctx.accounts.protocol;
    protocol.merchant_count = protocol
        .merchant_count
        .checked_add(1)
        .ok_or(ObligoError::Overflow)?;

    Ok(())
}
