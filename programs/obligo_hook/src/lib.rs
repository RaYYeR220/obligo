//! Token-2022 transfer hook for Obligo loyalty points.
//!
//! Points are a liability, not a currency. They may only move when the clearing house
//! (the `obligo` core program) has stated, in the same transaction, that it authorises
//! this exact source account to move this exact amount for this exact reason.
//!
//! The hook is deliberately dumb: it cannot read the clearing graph and must not try.
//! It reads a one-shot `Permit` PDA that only the core's `[b"authority"]` signer PDA can
//! create, and burns it down as points move. Absent a live permit, nothing moves — which
//! is what makes `points_outstanding` a collateral-backed number rather than a wish.
//!
//! Token-2022 does not invoke a hook on `MintTo` or `Burn`, so issuance and redemption
//! accounting lives in the core. The hook exclusively gates *movement*.

use anchor_lang::prelude::*;
use anchor_spl::token_2022::spl_token_2022::{
    extension::{
        transfer_hook::TransferHookAccount, BaseStateWithExtensions, PodStateWithExtensions,
    },
    pod::PodAccount,
};
use anchor_spl::token_interface::{Mint, TokenAccount};
use spl_discriminator::SplDiscriminate;
use spl_tlv_account_resolution::{
    account::ExtraAccountMeta, seeds::Seed, state::ExtraAccountMetaList,
};
use spl_transfer_hook_interface::instruction::ExecuteInstruction;

declare_id!("AtDpNdzKVRxMwK5bTotfmjxQdVU854RopJccgYRP8wQ7");

/// The clearing house. Its `[b"authority"]` PDA is the only account that may grant a permit.
pub const CORE_PROGRAM_ID: Pubkey = pubkey!("3YV8p2MhPQpSZS8fp3D7VoQYPy8GwxV221koWpsptRpN");

pub const PERMIT_SEED: &[u8] = b"permit";
pub const CORE_AUTHORITY_SEED: &[u8] = b"authority";
pub const EXTRA_ACCOUNT_METAS_SEED: &[u8] = b"extra-account-metas";

/// Why a movement was authorised. The core decides; the hook records. Numbered densely from zero so
/// `kind <= PERMIT_KIND_MAX` leaves no valid-looking hole a future desync could slip through.
pub const PERMIT_KIND_REDEEM: u8 = 0;
pub const PERMIT_KIND_EXPIRE: u8 = 1;
pub const PERMIT_KIND_MAX: u8 = PERMIT_KIND_EXPIRE;

#[program]
pub mod obligo_hook {
    use super::*;

    /// Publish the account list Token-2022 must resolve and hand to `Execute`.
    ///
    /// Execute's account order is fixed by the interface: 0=source, 1=mint, 2=destination,
    /// 3=authority, 4=EAML, resolved extras from 5. Our single extra is the permit PDA for
    /// *this* source account, writable so the hook can consume it.
    pub fn initialize_extra_account_meta_list(
        ctx: Context<InitializeExtraAccountMetaList>,
    ) -> Result<()> {
        let metas = vec![ExtraAccountMeta::new_with_seeds(
            &[
                Seed::Literal {
                    bytes: PERMIT_SEED.to_vec(),
                },
                Seed::AccountKey { index: 0 },
            ],
            false, // is_signer — a hook is never handed a signer
            true,  // is_writable — we burn the permit down
        )?];

        ExtraAccountMetaList::init::<ExecuteInstruction>(
            &mut ctx.accounts.extra_account_meta_list.try_borrow_mut_data()?,
            &metas,
        )?;
        Ok(())
    }

    /// Authorise `amount` points to leave `source_token`. Creates or overwrites the permit.
    ///
    /// Gated on a signature from the core program's `[b"authority"]` PDA, so only an
    /// instruction the core itself built can ever put points in motion.
    pub fn grant_permit(ctx: Context<GrantPermit>, kind: u8, amount: u64) -> Result<()> {
        require!(kind <= PERMIT_KIND_MAX, HookError::BadPermitKind);

        ctx.accounts.permit.set_inner(Permit {
            source: ctx.accounts.source_token.key(),
            kind,
            amount,
            bump: ctx.bumps.permit,
        });
        Ok(())
    }

    /// The SPL `Execute` entrypoint. Token-2022 calls this inside every `transfer_checked`
    /// on a points mint, using the SPL discriminator rather than an Anchor one.
    ///
    /// `#[interface(spl_transfer_hook_interface::execute)]` was removed in Anchor 1.0, so the
    /// discriminator is declared explicitly.
    #[instruction(discriminator = ExecuteInstruction::SPL_DISCRIMINATOR_SLICE)]
    pub fn transfer_hook(ctx: Context<TransferHook>, amount: u64) -> Result<()> {
        // Token-2022 raises `transferring` only for the duration of a real transfer. Without
        // this check anyone may invoke Execute directly, with a forged amount and forged extra
        // accounts, and drain every permit in the protocol.
        require_transferring(&ctx.accounts.source_token)?;
        require_transferring(&ctx.accounts.destination_token)?;

        // Confidential transfers hand the hook `u64::MAX` instead of a real amount. Points are
        // a public liability and are not confidential; refuse rather than under-count.
        require!(amount != u64::MAX, HookError::MovementNotAuthorized);

        let source = ctx.accounts.source_token.key();
        let permit_info = &ctx.accounts.permit;

        // A source account that was never granted a permit has no permit PDA at all: the account
        // is empty and still owned by the system program. That is not an edge case to apologise
        // for — it is the default, and the default is: points do not move.
        require_keys_eq!(
            *permit_info.owner,
            crate::ID,
            HookError::MovementNotAuthorized
        );

        let mut permit: Permit = {
            let data = permit_info.try_borrow_data()?;
            Permit::try_deserialize(&mut data.as_ref())
                .map_err(|_| error!(HookError::MovementNotAuthorized))?
        };

        // Re-derive from the bump we stored. `create_program_address` costs ~1.5k CU;
        // `find_program_address` would cost 12,136.
        let expected = Pubkey::create_program_address(
            &[PERMIT_SEED, source.as_ref(), &[permit.bump]],
            &crate::ID,
        )
        .map_err(|_| error!(HookError::MovementNotAuthorized))?;
        require_keys_eq!(
            expected,
            permit_info.key(),
            HookError::MovementNotAuthorized
        );
        require_keys_eq!(permit.source, source, HookError::MovementNotAuthorized);

        require!(permit.amount > 0, HookError::MovementNotAuthorized);
        require!(amount <= permit.amount, HookError::AmountExceedsPermit);

        // One-shot. A permit authorises a movement, not a standing right to move.
        permit.amount = permit
            .amount
            .checked_sub(amount)
            .ok_or(HookError::Overflow)?;

        let mut data = permit_info.try_borrow_mut_data()?;
        let mut cursor: &mut [u8] = &mut data;
        permit.try_serialize(&mut cursor)?;

        Ok(())
    }
}

/// The token account must be mid-transfer. This is the whole defence against a forged
/// `Execute`: the flag lives in Token-2022's extension data and only Token-2022 can raise it.
fn require_transferring(account: &InterfaceAccount<TokenAccount>) -> Result<()> {
    let info = account.to_account_info();
    let data = info.try_borrow_data()?;
    let state = PodStateWithExtensions::<PodAccount>::unpack(&data)
        .map_err(|_| error!(HookError::NotTransferring))?;
    let extension = state
        .get_extension::<TransferHookAccount>()
        .map_err(|_| error!(HookError::NotTransferring))?;

    require!(
        bool::from(extension.transferring),
        HookError::NotTransferring
    );
    Ok(())
}

/// A single authorised movement out of one token account.
#[account]
#[derive(InitSpace)]
pub struct Permit {
    /// The token account allowed to send. Bound so a permit cannot be replayed elsewhere.
    pub source: Pubkey,
    /// 0 = redeem, 1 = expire.
    pub kind: u8,
    /// Points still authorised to move. Decremented by the hook, never refilled by it.
    pub amount: u64,
    pub bump: u8,
}

#[derive(Accounts)]
pub struct InitializeExtraAccountMetaList<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    pub mint: InterfaceAccount<'info, Mint>,

    /// CHECK: raw TLV data, address pinned by seeds, written by `ExtraAccountMetaList::init`.
    #[account(
        init,
        payer = payer,
        space = ExtraAccountMetaList::size_of(1)?,
        seeds = [EXTRA_ACCOUNT_METAS_SEED, mint.key().as_ref()],
        bump
    )]
    pub extra_account_meta_list: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct GrantPermit<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    /// CHECK: the core program's signer PDA, pinned by seeds against `CORE_PROGRAM_ID`.
    /// The only account in existence that may authorise a point movement.
    #[account(
        signer,
        seeds = [CORE_AUTHORITY_SEED],
        bump,
        seeds::program = CORE_PROGRAM_ID,
    )]
    pub core_authority: UncheckedAccount<'info>,

    pub source_token: InterfaceAccount<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = payer,
        space = 8 + Permit::INIT_SPACE,
        seeds = [PERMIT_SEED, source_token.key().as_ref()],
        bump
    )]
    pub permit: Account<'info, Permit>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TransferHook<'info> {
    #[account(token::mint = mint)]
    pub source_token: InterfaceAccount<'info, TokenAccount>,

    pub mint: InterfaceAccount<'info, Mint>,

    #[account(token::mint = mint)]
    pub destination_token: InterfaceAccount<'info, TokenAccount>,

    /// CHECK: the transfer authority. Token-2022 strips its signature before calling us, so it
    /// arrives read-only and non-signer, and we never trust it.
    pub owner: UncheckedAccount<'info>,

    /// CHECK: our own TLV account, address pinned by seeds.
    #[account(
        seeds = [EXTRA_ACCOUNT_METAS_SEED, mint.key().as_ref()],
        bump
    )]
    pub extra_account_meta_list: UncheckedAccount<'info>,

    /// CHECK: resolved by Token-2022 from the EAML above, so during a real transfer its address
    /// is always `[b"permit", source]`. Loaded and re-derived by hand in the handler because a
    /// never-granted permit does not exist yet, and "does not exist" is a valid answer.
    #[account(mut)]
    pub permit: UncheckedAccount<'info>,
}

#[error_code]
pub enum HookError {
    #[msg("points may only move when the clearing house has authorised the movement")]
    MovementNotAuthorized,
    #[msg("movement exceeds the authorised amount")]
    AmountExceedsPermit,
    #[msg("hook invoked outside a real token-2022 transfer")]
    NotTransferring,
    #[msg("unknown permit kind")]
    BadPermitKind,
    #[msg("arithmetic overflow")]
    Overflow,
}
