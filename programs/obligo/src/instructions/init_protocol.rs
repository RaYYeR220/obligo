use anchor_lang::prelude::*;
use anchor_spl::token_interface::Mint;

use crate::constants::{AUTHORITY_SEED, PROTOCOL_SEED};
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
    #[account(executable)]
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
