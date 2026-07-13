//! Calls into `obligo_hook`, encoded by hand.
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

/// `sha256("global:initialize_extra_account_meta_list")[..8]`
pub const INITIALIZE_EXTRA_ACCOUNT_META_LIST: [u8; 8] = [92, 197, 174, 197, 41, 124, 19, 3];

pub const EXTRA_ACCOUNT_METAS_SEED: &[u8] = b"extra-account-metas";

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
