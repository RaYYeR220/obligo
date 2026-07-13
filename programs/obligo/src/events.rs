//! What the protocol tells the world.
//!
//! The books on chain record what the *protocol* owes and is owed. They do not, and cannot,
//! record what a shopkeeper handed across a counter. `Redeemed` is the bridge: it carries the
//! face value the acceptor will claim from the issuer and, next to it, the goods value the
//! acceptor's own posted rate obliges it to hand over. A till reads the second number.

use anchor_lang::prelude::*;

#[event]
pub struct OfferPosted {
    pub acceptor: Pubkey,
    pub issuer: Pubkey,
    pub rate_bps: u16,
    pub capacity: u64,
    pub expires_at: i64,
}

#[event]
pub struct OfferCancelled {
    pub acceptor: Pubkey,
    pub issuer: Pubkey,
    /// Face value the offer had already absorbed before it was withdrawn.
    pub consumed: u64,
}

#[event]
pub struct Redeemed {
    pub issuer: Pubkey,
    pub acceptor: Pubkey,
    pub customer: Pubkey,
    pub points: u64,
    /// USDC micro the acceptor may now claim from the issuer. Always 100% of face.
    pub value_face: u64,
    /// USDC micro of goods the acceptor's posted rate obliges it to hand the customer. Above
    /// `value_face` the difference is the acceptor's acquisition cost; below it, its discount on
    /// the issuer's credit.
    pub goods_value: u64,
    pub rate_bps: u16,
    /// The running total on the `issuer -> acceptor` edge after this redemption.
    pub obligation: u64,
}

/// Two merchants' mutual debt, resolved. Read `offset` and `paid` side by side: the first is the
/// debt that cancelled against debt, the second is all the money the pair actually needed to find.
#[event]
pub struct Settled {
    /// The merchant the graph says owes more. Not whichever one the caller named first.
    pub debtor: Pubkey,
    pub creditor: Pubkey,
    /// Cancelled against the counter-claim. No liquidity required for this part, ever, so it is
    /// applied whatever the debtor's solvency.
    pub offset: u64,
    /// USDC moved, debtor's vault to creditor's. `min(net, collateral)` while the debtor is solvent,
    /// and `0` once it is not: an insolvent merchant's vault is an estate that belongs to all of its
    /// creditors pro rata, and `liquidate` — not whoever cranks `settle` first — is what distributes it.
    pub paid: u64,
    /// Still owed on the edge afterwards. Non-zero whenever the debtor could not cover the net in
    /// cash — because it ran its collateral to zero, or because it was already insolvent and paid
    /// nothing. Either way it is now liquidatable.
    pub residual: u64,
}

/// A ring of debt, cancelled.
///
/// `usdc_moved` is in the event because it is the claim, and because it is always zero. Every
/// merchant in the cycle owes `amount_cleared` less and is owed `amount_cleared` less, no vault
/// was opened, and no counterparty had to find the liquidity to make it happen. This is the line
/// that separates a clearing house from a payment network.
#[event]
pub struct CycleCleared {
    pub cycle_len: u8,
    /// The smallest edge in the ring — the most that could be cancelled all the way round it.
    pub amount_cleared: u64,
    pub usdc_moved: u64,
}

/// One creditor's share of an insolvent issuer's estate.
///
/// `claim` and `paid` are both here because they are not the same number, and pretending otherwise
/// is how a protocol lies about a default. The creditor's claim is discharged in full; what it
/// actually recovers is its share of what is left, pro rata with everyone else's claim. The
/// difference is `written_off`, and somebody really did lose it.
#[event]
pub struct Liquidated {
    pub debtor: Pubkey,
    pub creditor: Pubkey,
    /// What the creditor was owed, and what this instruction discharges.
    pub claim: u64,
    /// USDC moved out of the estate: `claim * collateral / obligations_out`, rounded down.
    pub paid: u64,
    /// The part of the claim the estate could not cover. The loss this creditor underwrote when
    /// it posted the offer, having read the issuer's health first.
    pub written_off: u64,
    /// What is left in the estate for the debtor's remaining creditors.
    pub collateral_remaining: u64,
    /// And what those creditors are still owed.
    pub obligations_remaining: u64,
}

/// A defaulted merchant that can cover its debts again.
#[event]
pub struct Reinstated {
    pub merchant: Pubkey,
    pub collateral: u64,
    pub obligations_out: u64,
    /// Defaults on the record. Coming back does not take one off.
    pub defaults: u32,
}

/// Points that were never spent.
///
/// Breakage is the oldest revenue line in retail and it is normally recognised in a back office,
/// on a schedule the people holding the points never see. Here it is an instruction anybody may
/// call, on a deadline the merchant published, and it is the moment the reserve behind those
/// points stops being locked up. That is the honest version of the same accounting.
#[event]
pub struct Breakage {
    pub merchant: Pubkey,
    pub customer: Pubkey,
    pub points: u64,
    /// The liability the merchant is released from — and, at `reserve_bps` of it, the collateral
    /// it may now withdraw.
    pub face_value: u64,
}
