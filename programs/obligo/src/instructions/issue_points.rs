use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token_2022::{mint_to, MintTo, Token2022};
use anchor_spl::token_interface::{Mint, TokenAccount};

use crate::constants::{BATCH_SEED, MERCHANT_SEED, POINTS_SEED};
use crate::error::ObligoError;
use crate::math::required_collateral;
use crate::state::{Merchant, MerchantStatus, PointBatch};

#[derive(Accounts)]
pub struct IssuePoints<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [MERCHANT_SEED, authority.key().as_ref()],
        bump = merchant.bump,
        has_one = authority,
        has_one = points_mint,
    )]
    pub merchant: Account<'info, Merchant>,

    #[account(
        mut,
        seeds = [POINTS_SEED, merchant.key().as_ref()],
        bump = merchant.mint_bump,
        mint::token_program = token_program,
    )]
    pub points_mint: InterfaceAccount<'info, Mint>,

    /// CHECK: the customer. Nothing is read from it — its key derives the account the points land
    /// in and the batch that will one day expire them.
    pub customer: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = authority,
        associated_token::mint = points_mint,
        associated_token::authority = customer,
        associated_token::token_program = token_program,
    )]
    pub customer_points: InterfaceAccount<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = authority,
        space = 8 + PointBatch::INIT_SPACE,
        seeds = [BATCH_SEED, merchant.key().as_ref(), customer.key().as_ref()],
        bump
    )]
    pub batch: Account<'info, PointBatch>,

    pub token_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Issue `amount` points to a customer, and only if the merchant can still back them.
///
/// `MintTo` does not invoke the transfer hook — Token-2022 only calls it on a transfer. That is
/// exactly why the accounting has to happen here: if the core did not count the points, nothing
/// would, and `points_outstanding` would be a number nobody was defending.
pub(crate) fn handler(ctx: Context<IssuePoints>, amount: u64) -> Result<()> {
    require!(amount > 0, ObligoError::InvalidAmount);
    require!(
        ctx.accounts.merchant.status == MerchantStatus::Active,
        ObligoError::MerchantDefaulted
    );

    let merchant_key = ctx.accounts.merchant.key();
    let authority_key = ctx.accounts.authority.key();
    let merchant_bump = ctx.accounts.merchant.bump;
    let seeds: &[&[u8]] = &[MERCHANT_SEED, authority_key.as_ref(), &[merchant_bump]];

    mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            MintTo {
                mint: ctx.accounts.points_mint.to_account_info(),
                to: ctx.accounts.customer_points.to_account_info(),
                authority: ctx.accounts.merchant.to_account_info(),
            },
            &[seeds],
        ),
        amount,
    )?;

    let merchant = &mut ctx.accounts.merchant;
    merchant.points_outstanding = merchant
        .points_outstanding
        .checked_add(amount)
        .ok_or(ObligoError::Overflow)?;
    merchant.total_issued = merchant
        .total_issued
        .checked_add(amount)
        .ok_or(ObligoError::Overflow)?;

    // Checked against the books as they now stand, including the points we just printed. A
    // fractional reserve is a promise about a ratio; this is the line where the ratio is kept.
    let required = required_collateral(
        merchant.obligations_out,
        merchant.points_outstanding,
        merchant.usdc_per_point,
        merchant.reserve_bps,
    )?;
    require!(
        merchant.collateral >= required,
        ObligoError::ReserveBreached
    );

    let now = Clock::get()?.unix_timestamp;
    let batch = &mut ctx.accounts.batch;
    batch.merchant = merchant_key;
    batch.customer = ctx.accounts.customer.key();
    batch.amount = batch
        .amount
        .checked_add(amount)
        .ok_or(ObligoError::Overflow)?;
    // The TTL runs from the customer's last activity, not from a point's birthday. Expiry is a
    // rule about dormant customers; it should not punish an active one for having shopped early.
    batch.issued_at = now;
    batch.bump = ctx.bumps.batch;

    Ok(())
}
