//! Obligo — a permissionless clearing house for loyalty liabilities.
//!
//! A merchant posts partial USDC collateral and issues Token-2022 points against it under a
//! reserve invariant. Other merchants honour those points and accrue obligations against the
//! issuer. Obligations are netted bilaterally and cleared around cycles; issuers that cannot
//! cover the debts they have actually incurred are liquidated by anyone who cares to.
//!
//! Two rules run through everything here:
//!
//! - The `Protocol::authority` may change global parameters and nothing else. It cannot move a
//!   merchant's collateral, mint or burn a point, cancel an obligation or block a redemption.
//!   There is no instruction below that would let it, and that is not an accident.
//! - Token-2022 does not invoke a transfer hook on `MintTo` or `Burn`. So all issuance and
//!   redemption accounting lives here, in the core, and `obligo_hook` exclusively gates
//!   *movement*.

pub mod constants;
pub mod error;
pub mod events;
pub mod hook_cpi;
pub mod instructions;
pub mod math;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use events::*;
pub use instructions::*;
pub use state::*;

declare_id!("3YV8p2MhPQpSZS8fp3D7VoQYPy8GwxV221koWpsptRpN");

#[program]
pub mod obligo {
    use super::*;

    /// Genesis. Fixes the settlement asset and the transfer-hook program for all time.
    pub fn init_protocol(ctx: Context<InitProtocol>) -> Result<()> {
        instructions::init_protocol::handler(ctx)
    }

    /// Permissionless. Anyone may become an issuer.
    pub fn register_merchant(
        ctx: Context<RegisterMerchant>,
        name: String,
        usdc_per_point: u64,
        reserve_bps: u16,
        point_ttl: i64,
    ) -> Result<()> {
        instructions::register_merchant::handler(ctx, name, usdc_per_point, reserve_bps, point_ttl)
    }

    /// The merchant re-prices its own risk. It cannot re-price points already in the wild.
    pub fn set_terms(
        ctx: Context<SetTerms>,
        usdc_per_point: u64,
        reserve_bps: u16,
        point_ttl: i64,
    ) -> Result<()> {
        instructions::set_terms::handler(ctx, usdc_per_point, reserve_bps, point_ttl)
    }

    /// Permissionless: anyone may back a merchant.
    pub fn deposit_collateral(ctx: Context<DepositCollateral>, amount: u64) -> Result<()> {
        instructions::deposit_collateral::handler(ctx, amount)
    }

    /// Only the merchant, and only down to the reserve its outstanding points demand.
    pub fn withdraw_collateral(ctx: Context<WithdrawCollateral>, amount: u64) -> Result<()> {
        instructions::withdraw_collateral::handler(ctx, amount)
    }

    /// The merchant's Token-2022 points mint, hooked and un-repointable, with its
    /// `ExtraAccountMetaList` created in the same breath.
    pub fn create_points_mint(
        ctx: Context<CreatePointsMint>,
        name: String,
        symbol: String,
        uri: String,
    ) -> Result<()> {
        instructions::create_points_mint::handler(ctx, name, symbol, uri)
    }

    /// Mint points to a customer. Refused the moment the merchant can no longer back them.
    pub fn issue_points(ctx: Context<IssuePoints>, amount: u64) -> Result<()> {
        instructions::issue_points::handler(ctx, amount)
    }

    /// Bid to honour another merchant's points, at a rate and up to a budget of the bidder's
    /// choosing. Needs no permission from the issuer, and cannot be withdrawn by it.
    pub fn post_offer(
        ctx: Context<PostOffer>,
        rate_bps: u16,
        capacity: u64,
        expires_at: i64,
    ) -> Result<()> {
        instructions::post_offer::handler(ctx, rate_bps, capacity, expires_at)
    }

    /// Withdraw that bid. Only the acceptor may; the rent goes back to it.
    pub fn cancel_offer(ctx: Context<CancelOffer>) -> Result<()> {
        instructions::cancel_offer::handler(ctx)
    }

    /// A customer spends one merchant's points at another. The points move through the hook and
    /// are burned; the liability behind them becomes a debt between the two merchants; no USDC
    /// moves, and the issuer's health falls. That last clause is the protocol, not a side effect.
    pub fn redeem(ctx: Context<Redeem>, points: u64) -> Result<()> {
        instructions::redeem::handler(ctx, points)
    }

    /// Net two merchants' mutual debt and move only the difference. Permissionless: it makes the
    /// debtor healthier and the creditor wealthier and takes nothing from anyone, so there is
    /// nobody whose permission it could sensibly need — and a settlement only the creditor could
    /// trigger would be a settlement the creditor could sit on.
    pub fn settle(ctx: Context<Settle>) -> Result<()> {
        instructions::settle::handler(ctx)
    }
}
