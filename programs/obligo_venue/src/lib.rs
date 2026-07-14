//! Obligo Venue — a worked example of a third-party point-of-sale that composes with Obligo.
//!
//! Obligo is a clearing house for loyalty liabilities. Its promise is that *any* dApp can become a
//! redemption venue permissionlessly, just by calling `redeem` on the core: the points are spent,
//! the issuer's obligation is created, and the venue's own sale is recorded — atomically, in one
//! transaction, or not at all.
//!
//! This program is that claim, made concrete. It is a minimal till: `checkout` records a `Receipt`
//! of the sale in a PDA the venue owns, emits a `Sale` event for its own off-chain systems, and in
//! the same instruction **CPIs `redeem` on the Obligo core** so the loyalty points are burned and
//! the cross-merchant obligation is booked. The venue never touches the customer's points itself and
//! knows nothing of Obligo's reserve maths — it forwards the accounts and the core enforces every
//! invariant. That separation is the point: a venue integrates with a wire format, not a codebase.
//!
//! **Why the CPI is hand-rolled.** The obvious way — `obligo = { features = ["cpi"] }` and
//! `obligo::cpi::redeem(...)` — turns on `no-entrypoint`, and cargo unifies features across the
//! workspace, so a plain `cargo-build-sbf` would then strip the entrypoint out of the *core's* own
//! `.so`. It would deploy, resolve, and fail silently at runtime. So the venue speaks to the core by
//! its wire format: an 8-byte Anchor discriminator (`sha256("global:redeem")[..8]`) plus the borsh
//! `points: u64`, invoked with the exact account metas the core's `Redeem` accounts struct declares.
//! `tests/venue.rs` pins that discriminator and that account order against the core crate itself, so
//! a change over there turns a test red here rather than quietly breaking every checkout.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    instruction::{AccountMeta, Instruction},
    program::invoke,
};

declare_id!("7wNEAuRAECFeN2Y9bSdeXCxJAwvazbhEPsXhgRNFHD96");

/// The Obligo core program this venue redeems against. Pinned so a checkout can only ever drive the
/// real clearing house, never a look-alike a caller slipped into the `obligo_program` slot.
pub const OBLIGO_CORE_ID: Pubkey = pubkey!("3YV8p2MhPQpSZS8fp3D7VoQYPy8GwxV221koWpsptRpN");

/// `sha256("global:redeem")[..8]` — the core's Anchor discriminator for `redeem`. Pinned against the
/// core crate in `tests/venue.rs`, so a rename there is a red test here, not a dead till.
pub const REDEEM_DISCRIMINATOR: [u8; 8] = [184, 12, 86, 149, 70, 196, 97, 225];

pub const RECEIPT_SEED: &[u8] = b"receipt";

#[program]
pub mod obligo_venue {
    use super::*;

    /// Ring up a sale that a customer pays for, in part or whole, with Obligo loyalty points.
    ///
    /// `receipt_id` is the store's own order number (its receipts are `[b"receipt", store, id]`),
    /// `points` is how many of the issuer's points the customer is spending here, and `price` is the
    /// sale's sticker value in the venue's own units — a plain recorded number, the venue's business.
    ///
    /// The venue does two things of its own and one thing through Obligo, and all three land together
    /// or none do:
    ///
    ///   1. writes a `Receipt` PDA recording `{customer, store, issuer, points, price, timestamp}`;
    ///   2. emits a `Sale` event for its off-chain systems;
    ///   3. **CPIs `redeem` on the core**, spending the points and booking the issuer's obligation.
    ///
    /// If the core refuses the redemption — the offer is exhausted, the points have lapsed, the
    /// issuer has defaulted — the whole transaction reverts and no receipt is written. A sale paid
    /// with points that could not actually be spent never happened.
    pub fn checkout(
        ctx: Context<Checkout>,
        receipt_id: u64,
        points: u64,
        price: u64,
    ) -> Result<()> {
        require!(points > 0, VenueError::NothingToRedeem);

        // ---- compose with Obligo: spend the points, book the obligation --------------------------
        //
        // The account order and flags below are the core's `Redeem` accounts struct, verbatim. The
        // customer and the payer signed the outer transaction, so their signatures carry into the
        // CPI; the venue itself signs for nothing here — an integrator does not get, and does not
        // need, any authority over a customer's points.
        let accounts = vec![
            AccountMeta::new(ctx.accounts.payer.key(), true),
            AccountMeta::new_readonly(ctx.accounts.customer.key(), true),
            AccountMeta::new_readonly(ctx.accounts.protocol.key(), false),
            AccountMeta::new(ctx.accounts.issuer.key(), false),
            AccountMeta::new(ctx.accounts.acceptor.key(), false),
            AccountMeta::new(ctx.accounts.offer.key(), false),
            AccountMeta::new(ctx.accounts.obligation.key(), false),
            AccountMeta::new(ctx.accounts.points_mint.key(), false),
            AccountMeta::new(ctx.accounts.customer_points.key(), false),
            AccountMeta::new(ctx.accounts.redemption_escrow.key(), false),
            AccountMeta::new(ctx.accounts.batch.key(), false),
            AccountMeta::new_readonly(ctx.accounts.core_authority.key(), false),
            AccountMeta::new(ctx.accounts.permit.key(), false),
            AccountMeta::new_readonly(ctx.accounts.extra_account_meta_list.key(), false),
            AccountMeta::new_readonly(ctx.accounts.hook_program.key(), false),
            AccountMeta::new_readonly(ctx.accounts.token_program.key(), false),
            AccountMeta::new_readonly(ctx.accounts.associated_token_program.key(), false),
            AccountMeta::new_readonly(ctx.accounts.system_program.key(), false),
        ];

        // 8-byte discriminator, then borsh: `points: u64` little-endian.
        let mut data = Vec::with_capacity(REDEEM_DISCRIMINATOR.len() + 8);
        data.extend_from_slice(&REDEEM_DISCRIMINATOR);
        data.extend_from_slice(&points.to_le_bytes());

        let ix = Instruction {
            program_id: ctx.accounts.obligo_program.key(),
            accounts,
            data,
        };

        invoke(
            &ix,
            &[
                ctx.accounts.payer.to_account_info(),
                ctx.accounts.customer.to_account_info(),
                ctx.accounts.protocol.to_account_info(),
                ctx.accounts.issuer.to_account_info(),
                ctx.accounts.acceptor.to_account_info(),
                ctx.accounts.offer.to_account_info(),
                ctx.accounts.obligation.to_account_info(),
                ctx.accounts.points_mint.to_account_info(),
                ctx.accounts.customer_points.to_account_info(),
                ctx.accounts.redemption_escrow.to_account_info(),
                ctx.accounts.batch.to_account_info(),
                ctx.accounts.core_authority.to_account_info(),
                ctx.accounts.permit.to_account_info(),
                ctx.accounts.extra_account_meta_list.to_account_info(),
                ctx.accounts.hook_program.to_account_info(),
                ctx.accounts.token_program.to_account_info(),
                ctx.accounts.associated_token_program.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // ---- the venue's own books ---------------------------------------------------------------
        let now = Clock::get()?.unix_timestamp;
        let receipt = &mut ctx.accounts.receipt;
        receipt.set_inner(Receipt {
            customer: ctx.accounts.customer.key(),
            store: ctx.accounts.acceptor.key(),
            issuer: ctx.accounts.issuer.key(),
            receipt_id,
            points,
            price,
            timestamp: now,
            bump: ctx.bumps.receipt,
        });

        emit!(Sale {
            store: ctx.accounts.acceptor.key(),
            customer: ctx.accounts.customer.key(),
            issuer: ctx.accounts.issuer.key(),
            receipt_id,
            points,
            price,
            timestamp: now,
        });

        Ok(())
    }
}

/// `checkout`'s accounts: the venue's own two (`payer`, `receipt`) plus, forwarded untouched, exactly
/// the accounts the core's `redeem` needs. The forwarded ones are `UncheckedAccount`s on purpose —
/// the venue validates none of them, because the core validates all of them, and a redemption that
/// the core would reject is a redemption this instruction reverts on. The `mut`/signer flags here are
/// load-bearing: they set the privileges the CPI is then allowed to pass through.
#[derive(Accounts)]
#[instruction(receipt_id: u64)]
pub struct Checkout<'info> {
    /// Pays the receipt's rent and is forwarded as the redemption's `payer` — so a till or a relayer
    /// can carry the cost for a customer who has never held a lamport.
    #[account(mut)]
    pub payer: Signer<'info>,

    /// The customer spending their own points. Signs the outer transaction; that signature is what
    /// authorises the movement inside the core.
    pub customer: Signer<'info>,

    /// CHECK: the issuer whose points are spent. Forwarded to and validated by the core.
    #[account(mut)]
    pub issuer: UncheckedAccount<'info>,

    /// CHECK: the acceptor honouring the points — this store. Forwarded to and validated by the core,
    /// and the anchor of this receipt's address.
    #[account(mut)]
    pub acceptor: UncheckedAccount<'info>,

    /// The venue's permanent record of the sale. One per `(store, receipt_id)`; a repeat id is a
    /// duplicate order number and the `init` refuses it.
    #[account(
        init,
        payer = payer,
        space = 8 + Receipt::INIT_SPACE,
        seeds = [RECEIPT_SEED, acceptor.key().as_ref(), &receipt_id.to_le_bytes()],
        bump
    )]
    pub receipt: Account<'info, Receipt>,

    /// CHECK: the Obligo core, invoked below. Pinned to its known program id and required executable,
    /// so a checkout can only ever drive the real clearing house.
    #[account(executable, address = OBLIGO_CORE_ID)]
    pub obligo_program: UncheckedAccount<'info>,

    // ---- forwarded verbatim to `obligo::redeem` ----
    /// CHECK: forwarded to the core.
    pub protocol: UncheckedAccount<'info>,
    /// CHECK: forwarded to the core.
    #[account(mut)]
    pub offer: UncheckedAccount<'info>,
    /// CHECK: forwarded to the core.
    #[account(mut)]
    pub obligation: UncheckedAccount<'info>,
    /// CHECK: forwarded to the core.
    #[account(mut)]
    pub points_mint: UncheckedAccount<'info>,
    /// CHECK: forwarded to the core.
    #[account(mut)]
    pub customer_points: UncheckedAccount<'info>,
    /// CHECK: forwarded to the core.
    #[account(mut)]
    pub redemption_escrow: UncheckedAccount<'info>,
    /// CHECK: forwarded to the core.
    #[account(mut)]
    pub batch: UncheckedAccount<'info>,
    /// CHECK: forwarded to the core.
    pub core_authority: UncheckedAccount<'info>,
    /// CHECK: forwarded to the core.
    #[account(mut)]
    pub permit: UncheckedAccount<'info>,
    /// CHECK: forwarded to the core.
    pub extra_account_meta_list: UncheckedAccount<'info>,
    /// CHECK: forwarded to the core.
    pub hook_program: UncheckedAccount<'info>,
    /// CHECK: forwarded to the core.
    pub token_program: UncheckedAccount<'info>,
    /// CHECK: forwarded to the core.
    pub associated_token_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

/// One sale, recorded by the venue. Obligo's books say what the *protocol* owes; this says what the
/// store handed across the counter and which points paid for it.
#[account]
#[derive(InitSpace)]
pub struct Receipt {
    pub customer: Pubkey,
    /// The store that made the sale — the acceptor in Obligo's terms.
    pub store: Pubkey,
    /// The merchant whose points were spent.
    pub issuer: Pubkey,
    pub receipt_id: u64,
    /// Loyalty points redeemed on this sale.
    pub points: u64,
    /// The sale's price in the venue's own units.
    pub price: u64,
    pub timestamp: i64,
    pub bump: u8,
}

#[event]
pub struct Sale {
    pub store: Pubkey,
    pub customer: Pubkey,
    pub issuer: Pubkey,
    pub receipt_id: u64,
    pub points: u64,
    pub price: u64,
    pub timestamp: i64,
}

#[error_code]
pub enum VenueError {
    #[msg("a checkout must redeem at least one point")]
    NothingToRedeem,
}
