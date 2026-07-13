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
    /// Cancelled against the counter-claim. No liquidity required for this part, ever.
    pub offset: u64,
    /// USDC moved, debtor's vault to creditor's. `min(net, collateral)`.
    pub paid: u64,
    /// Still owed on the edge afterwards. Non-zero only when the debtor ran out of collateral —
    /// in which case it is now insolvent, and anyone may liquidate it.
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
