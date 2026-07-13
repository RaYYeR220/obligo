//! Test fixture. Never deployed.
//!
//! `obligo_hook::grant_permit` demands a signature from the core program's `[b"authority"]` PDA,
//! and only the program that owns a PDA can sign for it. So the hook's tests cannot reach the
//! happy path from a plain keypair, and hand-writing permit accounts would test nothing.
//!
//! This crate carries the core program's id and does exactly one thing: CPI into the hook with
//! that PDA's signature, the way the real core does inside `redeem` and `expire_points`.

use anchor_lang::prelude::*;
use obligo_hook::cpi::accounts::GrantPermit;
use obligo_hook::program::ObligoHook;

declare_id!("3YV8p2MhPQpSZS8fp3D7VoQYPy8GwxV221koWpsptRpN");

#[program]
pub mod mock_core {
    use super::*;

    #[instruction(discriminator = [0])]
    pub fn grant(ctx: Context<Grant>, kind: u8, amount: u64) -> Result<()> {
        let bump = ctx.bumps.core_authority;
        let seeds: &[&[u8]] = &[b"authority", &[bump]];

        obligo_hook::cpi::grant_permit(
            CpiContext::new_with_signer(
                ctx.accounts.hook_program.key(),
                GrantPermit {
                    payer: ctx.accounts.payer.to_account_info(),
                    core_authority: ctx.accounts.core_authority.to_account_info(),
                    source_token: ctx.accounts.source_token.to_account_info(),
                    permit: ctx.accounts.permit.to_account_info(),
                    system_program: ctx.accounts.system_program.to_account_info(),
                },
                &[seeds],
            ),
            kind,
            amount,
        )
    }
}

#[derive(Accounts)]
pub struct Grant<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    /// CHECK: the core's signer PDA; the runtime signs for it via `invoke_signed`.
    #[account(seeds = [b"authority"], bump)]
    pub core_authority: UncheckedAccount<'info>,

    /// CHECK: forwarded to the hook, which validates it.
    pub source_token: UncheckedAccount<'info>,

    /// CHECK: forwarded to the hook, which owns and validates it.
    #[account(mut)]
    pub permit: UncheckedAccount<'info>,

    pub hook_program: Program<'info, ObligoHook>,
    pub system_program: Program<'info, System>,
}
