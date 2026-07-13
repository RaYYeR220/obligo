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
/// would still be authorised — as an expiry — and nothing would complain.
#[test]
fn the_permit_kinds_we_hardcode_are_the_hook_s_own() {
    assert_eq!(
        obligo::hook_cpi::PERMIT_KIND_REDEEM,
        obligo_hook::PERMIT_KIND_REDEEM,
    );
    assert_eq!(
        obligo::hook_cpi::PERMIT_KIND_EXPIRE,
        obligo_hook::PERMIT_KIND_EXPIRE,
    );
}

/// The core reads a permit's remaining `amount` straight out of the hook-owned account at a fixed
/// byte offset (`hook_cpi::permit_remaining`), to prove a movement drained it to zero. That read is
/// only correct if the hook lays the field out where the core expects it: `8 disc + 32 source + 1
/// kind = 41`. Pin the offset against the hook's real `Permit`, so a reordering over there turns this
/// red rather than making `redeem`/`expire_points` read the wrong eight bytes.
#[test]
fn the_permit_amount_offset_the_core_reads_is_the_hook_s_own() {
    use anchor_lang::prelude::Pubkey;
    use anchor_lang::AccountSerialize;

    let permit = obligo_hook::Permit {
        source: Pubkey::new_unique(),
        kind: obligo_hook::PERMIT_KIND_EXPIRE,
        amount: 0xA1A2_A3A4_A5A6_A7A8,
        bump: 253,
    };

    let mut buf = Vec::new();
    permit.try_serialize(&mut buf).unwrap();

    assert_eq!(
        &buf[41..49],
        &0xA1A2_A3A4_A5A6_A7A8u64.to_le_bytes(),
        "the core reads the remaining amount from bytes [41..49]"
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
