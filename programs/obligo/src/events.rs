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
