//! Everything the core has to say to the hook, encoded by hand — and the one call to Token-2022
//! that has to carry the hook along with it.
//!
//! The obvious way to write this is `obligo_hook = { path = "...", features = ["cpi"] }` and
//! `obligo_hook::cpi::*`. Do not. The `cpi` feature turns on `no-entrypoint`, cargo unifies
//! features across the workspace, and the `obligo_hook.so` that comes out of a workspace-wide
//! `cargo-build-sbf` is then compiled *without its entrypoint*. It deploys, it resolves, and
//! every transfer fails — silently, and only at runtime.
//!
//! So the core knows the hook by its wire format instead of by its types. The wire format is
//! eight bytes of Anchor discriminator plus borsh args, and `tests/hook_abi.rs` pins every
//! constant below against the hook crate itself, so a rename over there is a red test here
//! rather than a dead protocol.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::{instruction::Instruction, program::invoke_signed};
use anchor_spl::token_2022::spl_token_2022;

/// `sha256("global:initialize_extra_account_meta_list")[..8]`
pub const INITIALIZE_EXTRA_ACCOUNT_META_LIST: [u8; 8] = [92, 197, 174, 197, 41, 124, 19, 3];

/// `sha256("global:grant_permit")[..8]`
pub const GRANT_PERMIT: [u8; 8] = [170, 94, 187, 22, 42, 224, 162, 203];

pub const EXTRA_ACCOUNT_METAS_SEED: &[u8] = b"extra-account-metas";
pub const PERMIT_SEED: &[u8] = b"permit";

/// Why the clearing house is letting these points move. The hook records it; nothing else in the
/// protocol may put a point in motion at all.
pub const PERMIT_KIND_REDEEM: u8 = 0;
pub const PERMIT_KIND_GIFT: u8 = 1;
pub const PERMIT_KIND_EXPIRE: u8 = 2;

/// Publish the account list Token-2022 hands to the hook on every transfer of this mint.
///
/// Called from `create_points_mint`, in the same instruction that creates the mint, because a
/// mint whose `ExtraAccountMetaList` does not exist yet is a mint whose points cannot move and
/// whose EAML anyone else may then front-run.
pub fn initialize_extra_account_meta_list<'info>(
    hook_program: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    extra_account_meta_list: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let ix = Instruction {
        program_id: hook_program.key(),
        accounts: vec![
            AccountMeta::new(payer.key(), true),
            AccountMeta::new_readonly(mint.key(), false),
            AccountMeta::new(extra_account_meta_list.key(), false),
            AccountMeta::new_readonly(system_program.key(), false),
        ],
        data: INITIALIZE_EXTRA_ACCOUNT_META_LIST.to_vec(),
    };

    invoke_signed(
        &ix,
        &[
            payer.clone(),
            mint.clone(),
            extra_account_meta_list.clone(),
            system_program.clone(),
        ],
        signer_seeds,
    )
    .map_err(Into::into)
}

/// Authorise exactly `amount` points to leave exactly `source_token`, for exactly this reason.
///
/// This is the only door. The hook will not let a point move without a permit, and it will not
/// accept a permit that was not signed for by the core's `[b"authority"]` PDA — an address no
/// keypair on earth can produce a signature for, and that only this program can sign with. So the
/// sentence "points move only when the clearing house says so" is not a policy we are promising to
/// enforce; it is a fact about which private keys exist.
///
/// `signer_seeds` must therefore carry `[b"authority", &[protocol.authority_bump]]`.
///
/// The permit is one-shot: the hook decrements it as the points move, and a redemption that
/// authorises `n` and moves `n` leaves nothing behind to replay.
#[allow(clippy::too_many_arguments)]
pub fn grant_permit<'info>(
    hook_program: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    core_authority: &AccountInfo<'info>,
    source_token: &AccountInfo<'info>,
    permit: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    kind: u8,
    amount: u64,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    // Eight bytes of discriminator, then borsh: `kind: u8`, `amount: u64` little-endian.
    let mut data = Vec::with_capacity(GRANT_PERMIT.len() + 9);
    data.extend_from_slice(&GRANT_PERMIT);
    data.push(kind);
    data.extend_from_slice(&amount.to_le_bytes());

    let ix = Instruction {
        program_id: hook_program.key(),
        accounts: vec![
            AccountMeta::new(payer.key(), true),
            AccountMeta::new_readonly(core_authority.key(), true),
            AccountMeta::new_readonly(source_token.key(), false),
            AccountMeta::new(permit.key(), false),
            AccountMeta::new_readonly(system_program.key(), false),
        ],
        data,
    };

    invoke_signed(
        &ix,
        &[
            payer.clone(),
            core_authority.clone(),
            source_token.clone(),
            permit.clone(),
            system_program.clone(),
        ],
        signer_seeds,
    )
    .map_err(Into::into)
}

/// Move points, with the hook in the loop.
///
/// This is a Token-2022 `TransferChecked` and nothing more exotic, and it is written out by hand
/// because `anchor_spl`'s version cannot do it. Look at `anchor_spl::token_2022::transfer_checked`:
/// it builds a four-account instruction — source, mint, destination, authority — and invokes it
/// with exactly those four `AccountInfo`s. `CpiContext::with_remaining_accounts` has no effect on
/// it; the extra accounts are dropped on the floor.
///
/// For an ordinary mint that is correct. For a hooked mint it cannot work at all: Token-2022 has to
/// call the hook, and the only place it can find the hook program, the mint's `ExtraAccountMetaList`
/// and the accounts that list resolves to is *the account list of the transfer instruction itself*.
/// Hand it four accounts and it gets as far as `Unknown program <hook>` and returns `MissingAccount`.
///
/// So we append them. Token-2022 matches them up by key, resolves the EAML, and invokes `Execute` —
/// which is the moment the protocol's one hard rule is enforced: no permit, no movement.
///
/// `signer_seeds` is empty for a redemption, where the customer signs the outer transaction for
/// their own points, and carries the merchant PDA's seeds for an expiry, where nobody signs for the
/// customer at all and the merchant PDA moves the points as the mint's permanent delegate. The hook
/// does not know or care which: it asks for a permit either way, and the permanent delegate has no
/// more right to move a point without one than the customer does.
#[allow(clippy::too_many_arguments)]
pub fn transfer_points<'info>(
    token_program: &AccountInfo<'info>,
    source: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    destination: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
    hook_program: &AccountInfo<'info>,
    extra_account_meta_list: &AccountInfo<'info>,
    permit: &AccountInfo<'info>,
    amount: u64,
    decimals: u8,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let mut ix = spl_token_2022::instruction::transfer_checked(
        token_program.key,
        source.key,
        mint.key,
        destination.key,
        authority.key,
        &[],
        amount,
        decimals,
    )?;

    ix.accounts
        .push(AccountMeta::new_readonly(hook_program.key(), false));
    ix.accounts.push(AccountMeta::new_readonly(
        extra_account_meta_list.key(),
        false,
    ));
    // Writable: the hook spends the permit down as the points move.
    ix.accounts.push(AccountMeta::new(permit.key(), false));

    invoke_signed(
        &ix,
        &[
            source.clone(),
            mint.clone(),
            destination.clone(),
            authority.clone(),
            hook_program.clone(),
            extra_account_meta_list.clone(),
            permit.clone(),
        ],
        signer_seeds,
    )
    .map_err(Into::into)
}
