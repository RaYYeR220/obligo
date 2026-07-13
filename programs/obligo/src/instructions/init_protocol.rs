use anchor_lang::prelude::*;
use anchor_spl::token_interface::Mint;

use crate::constants::{AUTHORITY_SEED, OBLIGO_HOOK_ID, PROTOCOL_SEED};
use crate::state::Protocol;

#[derive(Accounts)]
pub struct InitProtocol<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + Protocol::INIT_SPACE,
        seeds = [PROTOCOL_SEED],
        bump
    )]
    pub protocol: Account<'info, Protocol>,

    /// The settlement asset. Collateral, obligations and face values are all denominated in it.
    pub usdc_mint: InterfaceAccount<'info, Mint>,

    /// CHECK: the transfer-hook program every points mint will be permanently bound to. Fixed
    /// here, at genesis, and never updatable: there is no instruction that rewrites this field,
    /// because a protocol that can repoint its own hook is a protocol that can rug its points.
    ///
    /// Pinned to `OBLIGO_HOOK_ID` so genesis itself cannot be front-run onto a malicious hook. Absent
    /// this, whoever won the race to call `init_protocol` could bind every future mint to a hook of
    /// their choosing — and the binding is permanent. Still `executable`, so the id must resolve to a
    /// real program, not merely match by address.
    #[account(executable, address = OBLIGO_HOOK_ID)]
    pub hook_program: UncheckedAccount<'info>,

    /// CHECK: the program's signer PDA. It holds nothing and is never written; it exists only so
    /// the hook has exactly one account whose signature means "the clearing house said so".
    #[account(seeds = [AUTHORITY_SEED], bump)]
    pub protocol_authority: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub(crate) fn handler(ctx: Context<InitProtocol>) -> Result<()> {
    ctx.accounts.protocol.set_inner(Protocol {
        authority: ctx.accounts.authority.key(),
        usdc_mint: ctx.accounts.usdc_mint.key(),
        hook_program: ctx.accounts.hook_program.key(),
        merchant_count: 0,
        bump: ctx.bumps.protocol,
        authority_bump: ctx.bumps.protocol_authority,
    });
    Ok(())
}
