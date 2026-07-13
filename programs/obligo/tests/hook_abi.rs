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
    assert_eq!(
        obligo::hook_cpi::GRANT_PERMIT,
        obligo_hook::instruction::GrantPermit::DISCRIMINATOR,
    );
}

#[test]
fn the_hook_seeds_we_hardcode_are_the_hook_s_own() {
    assert_eq!(
        obligo::hook_cpi::EXTRA_ACCOUNT_METAS_SEED,
        obligo_hook::EXTRA_ACCOUNT_METAS_SEED,
    );
    assert_eq!(obligo::hook_cpi::PERMIT_SEED, obligo_hook::PERMIT_SEED);
}

/// A permit's `kind` is a bare byte on the wire. If the hook ever renumbers them, a redemption
/// would still be authorised — as a gift, or an expiry — and nothing would complain.
#[test]
fn the_permit_kinds_we_hardcode_are_the_hook_s_own() {
    assert_eq!(
        obligo::hook_cpi::PERMIT_KIND_REDEEM,
        obligo_hook::PERMIT_KIND_REDEEM,
    );
    assert_eq!(
        obligo::hook_cpi::PERMIT_KIND_GIFT,
        obligo_hook::PERMIT_KIND_GIFT,
    );
    assert_eq!(
        obligo::hook_cpi::PERMIT_KIND_EXPIRE,
        obligo_hook::PERMIT_KIND_EXPIRE,
    );
}

/// `grant_permit` is called by hand-built `Instruction`, so its argument order is a wire contract
/// too: `kind: u8` then `amount: u64`. Anchor's own borsh encoding of the call is the reference.
#[test]
fn the_grant_permit_arguments_we_encode_are_in_the_hook_s_order() {
    use anchor_lang::InstructionData;

    let anchors = obligo_hook::instruction::GrantPermit {
        kind: obligo_hook::PERMIT_KIND_REDEEM,
        amount: 7_000_000_000_000_000_042,
    }
    .data();

    let mut ours = obligo::hook_cpi::GRANT_PERMIT.to_vec();
    ours.push(obligo::hook_cpi::PERMIT_KIND_REDEEM);
    ours.extend_from_slice(&7_000_000_000_000_000_042u64.to_le_bytes());

    assert_eq!(ours, anchors);
}
