use anchor_lang::prelude::*;

use crate::constants::{MAX_POINT_TTL, MERCHANT_SEED};
use crate::error::ObligoError;
use crate::math::{required_collateral, BPS};
use crate::state::Merchant;

pub fn validate_terms(usdc_per_point: u64, reserve_bps: u16, point_ttl: i64) -> Result<()> {
    require!(usdc_per_point > 0, ObligoError::InvalidTerms);
    require!(reserve_bps as u128 <= BPS, ObligoError::InvalidTerms);
    require!(
        point_ttl > 0 && point_ttl <= MAX_POINT_TTL,
        ObligoError::InvalidTerms
    );
    Ok(())
}

#[derive(Accounts)]
pub struct SetTerms<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [MERCHANT_SEED, authority.key().as_ref()],
        bump = merchant.bump,
        has_one = authority,
    )]
    pub merchant: Account<'info, Merchant>,
}

/// A merchant may re-price its own risk, not its own liabilities.
///
/// `reserve_bps` and `point_ttl` are free to move — tightening them is only ever safer, and
/// loosening them is caught below by the invariant. `usdc_per_point` is not: it is the promise
/// printed on every point already in a customer's pocket, and a merchant that could rewrite it
/// downwards could inflate its way out of its own liabilities. So it is frozen for as long as
/// anyone is holding a point or is owed a dollar.
pub(crate) fn handler(
    ctx: Context<SetTerms>,
    usdc_per_point: u64,
    reserve_bps: u16,
    point_ttl: i64,
) -> Result<()> {
    validate_terms(usdc_per_point, reserve_bps, point_ttl)?;

    let merchant = &mut ctx.accounts.merchant;

    if usdc_per_point != merchant.usdc_per_point {
        require!(
            merchant.points_outstanding == 0 && merchant.obligations_out == 0,
            ObligoError::TermsLocked
        );
        merchant.usdc_per_point = usdc_per_point;
    }
    merchant.reserve_bps = reserve_bps;
    merchant.point_ttl = point_ttl;

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

    Ok(())
}
