use anchor_lang::prelude::*;

use crate::constants::MERCHANT_SEED;
use crate::error::ObligoError;
use crate::events::Reinstated;
use crate::math::is_solvent;
use crate::state::{Merchant, MerchantStatus};

/// Permissionless, like everything else that only makes the graph smaller. See the handler.
#[derive(Accounts)]
pub struct Reinstate<'info> {
    pub cranker: Signer<'info>,

    #[account(
        mut,
        seeds = [MERCHANT_SEED, merchant.authority.as_ref()],
        bump = merchant.bump,
    )]
    pub merchant: Account<'info, Merchant>,
}

/// Bring a defaulted merchant that can pay its debts again back to `Active`.
///
/// Without this instruction `Defaulted` would be a trap door, and not only for the merchant. A
/// defaulted merchant cannot be `settle`d — its collateral is an estate and belongs to all of its
/// creditors in proportion, so the first creditor to reach the crank must not be able to take it.
/// But `liquidate` only fires while the merchant is *insolvent*. There is a state between those
/// two — defaulted, and solvent again, because a ring got cleared or somebody topped its vault up —
/// where a creditor could not settle and could not liquidate, and its perfectly good claim would sit
/// there forever waiting on a merchant that had no particular reason to act.
///
/// So the door is opened by the *books*, not by the merchant: **anyone may reinstate a merchant that
/// is solvent.** The stranded creditor turns the crank itself and then settles. Nobody is harmed by
/// it — reinstatement lets the merchant issue points (still gated by the reserve invariant) and lets
/// its creditors take its money (which is what they are owed) — and letting the merchant hold the
/// key to it would be handing a debtor a veto over its own creditors.
///
/// Solvency, not health, is the threshold, because solvency is what liquidation is keyed to and a
/// door that swings shut at a different line than it opens at is not a door.
///
/// What reinstatement does *not* do is erase the record. `defaults` stays where it is.
pub(crate) fn handler(ctx: Context<Reinstate>) -> Result<()> {
    let merchant = &mut ctx.accounts.merchant;

    require!(
        merchant.status == MerchantStatus::Defaulted,
        ObligoError::NotDefaulted
    );
    require!(
        is_solvent(merchant.collateral, merchant.obligations_out),
        ObligoError::StillInsolvent
    );

    merchant.status = MerchantStatus::Active;

    emit!(Reinstated {
        merchant: merchant.key(),
        collateral: merchant.collateral,
        obligations_out: merchant.obligations_out,
        defaults: merchant.defaults,
    });

    Ok(())
}
