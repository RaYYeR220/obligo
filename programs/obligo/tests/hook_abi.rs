//! The core calls the hook by wire format, not by type (see `src/hook_cpi.rs` for why).
//!
//! That trade buys a workspace whose `cargo-build-sbf` produces a hook binary that still has an
//! entrypoint. The bill it leaves is this file: if anyone renames an instruction over in the hook,
//! nothing in the compiler notices, and every points mint created afterwards would fail on-chain
//! with a bare `InvalidInstructionData`. So the constants are pinned to the hook crate itself.

use anchor_lang::Discriminator;

#[test]
fn the_hook_instruction_discriminators_we_hardcode_are_the_hook_s_own() {
    assert_eq!(
        obligo::hook_cpi::INITIALIZE_EXTRA_ACCOUNT_META_LIST,
        obligo_hook::instruction::InitializeExtraAccountMetaList::DISCRIMINATOR,
    );
}

#[test]
fn the_hook_seed_we_hardcode_is_the_hook_s_own() {
    assert_eq!(
        obligo::hook_cpi::EXTRA_ACCOUNT_METAS_SEED,
        obligo_hook::EXTRA_ACCOUNT_METAS_SEED,
    );
}
