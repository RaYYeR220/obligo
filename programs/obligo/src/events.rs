//! What the protocol tells the world.

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
