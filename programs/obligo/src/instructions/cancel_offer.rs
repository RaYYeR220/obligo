use anchor_lang::prelude::*;

use crate::constants::{MERCHANT_SEED, OFFER_SEED};
use crate::events::OfferCancelled;
use crate::state::{AcceptanceOffer, Merchant};

/// Withdraw a bid.
///
/// The only account that can reach this instruction is the acceptor's own authority: the acceptor
/// PDA is derived from the signer, and the offer PDA is derived from the acceptor. An issuer that
/// dislikes being priced at 85% cannot make the offer go away, and neither can the protocol
/// authority — it is not a party to this instruction at all.
///
/// Redemptions already made against the offer are untouched. Cancelling stops the next one; it
/// does not unwind the last one.
#[derive(Accounts)]
pub struct CancelOffer<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [MERCHANT_SEED, authority.key().as_ref()],
        bump = acceptor.bump,
        has_one = authority,
    )]
    pub acceptor: Account<'info, Merchant>,

    /// The issuer is not an account here — the offer remembers who it was for, and the rent goes
    /// back to the acceptor who paid it.
    #[account(
        mut,
        close = authority,
        seeds = [OFFER_SEED, acceptor.key().as_ref(), offer.issuer.as_ref()],
        bump = offer.bump,
        has_one = acceptor,
    )]
    pub offer: Account<'info, AcceptanceOffer>,
}

pub(crate) fn handler(ctx: Context<CancelOffer>) -> Result<()> {
    emit!(OfferCancelled {
        acceptor: ctx.accounts.offer.acceptor,
        issuer: ctx.accounts.offer.issuer,
        consumed: ctx.accounts.offer.consumed,
    });
    Ok(())
}
