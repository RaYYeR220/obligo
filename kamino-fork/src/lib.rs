//! A throwaway program that exists only to drive obligo's real [`KaminoAdapter`] against real
//! mainnet KLend in `tests/fork.rs`.
//!
//! The adapter's deposit and withdraw are `invoke_signed` CPIs, so they can only run inside a
//! program — a client cannot sign for the vault PDA. This probe is that program, and nothing more:
//! it owns a vault PDA, and each instruction builds the adapter through obligo's own
//! `yield_adapter::vault_adapter` factory — the identical entry point `deposit_collateral` and
//! `withdraw_collateral` use — and calls it. There is no yield logic here; all of it lives in
//! `obligo::yield_adapter`, which is what the test is proving.

use anchor_lang::prelude::*;
use obligo::yield_adapter::{vault_adapter, YieldAdapter};

declare_id!("7U7wtcqdmFTXVTQGAL4mTVH6E5rt3eW7RETYySB3ywe6");

/// The vault PDA seed. This PDA is the authority of the USDC and cToken accounts, and the signer of
/// every KLend CPI the adapter makes — exactly as the merchant PDA is in the core.
pub const VAULT_SEED: &[u8] = b"vault";

#[program]
pub mod kamino_probe {
    use super::*;

    /// Route `amount` USDC from the vault into KLend via the real adapter. The KLend accounts arrive
    /// in `remaining_accounts` in the order `KaminoAdapter::from_remaining` expects.
    pub fn yield_deposit<'info>(
        ctx: Context<'info, YieldOp<'info>>,
        amount: u64,
    ) -> Result<()> {
        with_adapter(&ctx, |adapter| {
            adapter.deposit(amount)?;
            msg!("deposited {}", amount);
            Ok(())
        })
    }

    /// Redeem the vault's whole cToken position back to USDC and log what it realized. The test reads
    /// that number and asserts it exceeds what `yield_deposit` put in.
    pub fn yield_withdraw<'info>(
        ctx: Context<'info, YieldOp<'info>>,
        principal_out: u64,
    ) -> Result<()> {
        with_adapter(&ctx, |adapter| {
            let realized = adapter.withdraw(principal_out)?;
            msg!("realized {}", realized);
            Ok(())
        })
    }

    /// Report the position's principal + accrued value through the adapter's `total_assets`, so the
    /// test can cross-check the reserve-derived valuation against the USDC a redemption actually pays.
    pub fn yield_report<'info>(ctx: Context<'info, YieldOp<'info>>) -> Result<()> {
        with_adapter(&ctx, |adapter| {
            msg!("total_assets {}", adapter.total_assets()?);
            Ok(())
        })
    }
}

fn with_adapter<'info>(
    ctx: &Context<'info, YieldOp<'info>>,
    f: impl FnOnce(&dyn YieldAdapter) -> Result<()>,
) -> Result<()> {
    let vault = ctx.accounts.vault_liquidity.to_account_info();
    let owner = ctx.accounts.vault_authority.to_account_info();
    let bump = ctx.bumps.vault_authority;
    let seeds: &[&[u8]] = &[VAULT_SEED, &[bump]];
    let adapter = vault_adapter(&vault, ctx.remaining_accounts, &owner, seeds)?;
    f(&adapter)
}

#[derive(Accounts)]
pub struct YieldOp<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    /// CHECK: PDA authority over the vault token accounts; signs the KLend CPIs. Derived, never read.
    #[account(seeds = [VAULT_SEED], bump)]
    pub vault_authority: UncheckedAccount<'info>,

    /// CHECK: the vault's USDC (user-liquidity) token account. The adapter reads its balance and
    /// KLend moves USDC through it.
    #[account(mut)]
    pub vault_liquidity: UncheckedAccount<'info>,
}
