use anchor_lang::prelude::*;

use crate::constants::{MAX_RATE_BPS, MERCHANT_SEED, MIN_RATE_BPS, OFFER_SEED};
use crate::error::ObligoError;
use crate::events::OfferPosted;
use crate::state::{AcceptanceOffer, Merchant};

/// The acceptor's own authority. An offer is a commitment of the acceptor's shelves; nobody else
/// may make it, and the merchant PDA is derived from the signer, so nobody else can name it.
#[derive(Accounts)]
pub struct PostOffer<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [MERCHANT_SEED, authority.key().as_ref()],
        bump = acceptor.bump,
        has_one = authority,
    )]
    pub acceptor: Account<'info, Merchant>,

    /// The merchant whose points are being bid for. Only its key is read: an offer is a statement
    /// about somebody else's liability, and it needs no permission from them.
    #[account(
        seeds = [MERCHANT_SEED, issuer.authority.as_ref()],
        bump = issuer.bump,
    )]
    pub issuer: Account<'info, Merchant>,

    #[account(
        init_if_needed,
        payer = authority,
        space = 8 + AcceptanceOffer::INIT_SPACE,
        seeds = [OFFER_SEED, acceptor.key().as_ref(), issuer.key().as_ref()],
        bump
    )]
    pub offer: Account<'info, AcceptanceOffer>,

    pub system_program: Program<'info, System>,
}

/// Bid for another merchant's points.
///
/// Three numbers, and the third is the one that matters. `rate_bps` prices the goods. `expires_at`
/// bounds the promise in time. `capacity` bounds it in money: it is the face value this offer will
/// absorb before it stops, and it is the acceptor's customer-acquisition budget expressed somewhere
/// it cannot quietly overrun. A marketing budget that lives in a spreadsheet is a hope; this one is
/// checked on every redemption, against the acceptor's own till.
///
/// Posting again replaces the offer outright — a changed bid is a new offer, and its budget starts
/// again with it. Nothing here consults the issuer, and nothing here can be vetoed by it.
pub(crate) fn handler(
    ctx: Context<PostOffer>,
    rate_bps: u16,
    capacity: u64,
    expires_at: i64,
) -> Result<()> {
    require!(
        (MIN_RATE_BPS..=MAX_RATE_BPS).contains(&rate_bps),
        ObligoError::InvalidRate
    );
    require!(capacity > 0, ObligoError::InvalidAmount);

    let acceptor = ctx.accounts.acceptor.key();
    let issuer = ctx.accounts.issuer.key();

    // An acceptor honouring its own points is issuing a refund, not accepting a liability. It
    // would book itself a debt it also holds: a self-loop in the graph that settlement would net
    // to nothing and cycle clearing would have to special-case. It is not a bid.
    require!(acceptor != issuer, ObligoError::SelfOffer);

    let now = Clock::get()?.unix_timestamp;
    require!(expires_at > now, ObligoError::OfferExpired);

    ctx.accounts.offer.set_inner(AcceptanceOffer {
        acceptor,
        issuer,
        rate_bps,
        capacity,
        consumed: 0,
        expires_at,
        bump: ctx.bumps.offer,
    });

    emit!(OfferPosted {
        acceptor,
        issuer,
        rate_bps,
        capacity,
        expires_at,
    });

    Ok(())
}
