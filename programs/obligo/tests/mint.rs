//! The points mint: Token-2022, transfer-hooked, self-describing, and un-repointable.

mod common;

use anchor_spl::token_2022::spl_token_2022::{
    extension::{
        metadata_pointer::MetadataPointer, transfer_hook::TransferHook, BaseStateWithExtensions,
        ExtensionType, StateWithExtensions,
    },
    state::Mint as MintState,
};
use anchor_spl::token_2022_extensions::spl_token_metadata_interface::state::TokenMetadata;
use common::*;
use spl_tlv_account_resolution::state::ExtraAccountMetaList;

/// The whole security story of this mint is in what it *cannot* do.
#[test]
fn the_points_mint_is_hooked_and_the_hook_can_never_be_repointed() {
    let mut env = Env::new();
    let m = env.register_merchant("Cafe Aurora", 10_000, 3000, 86_400);

    env.create_points_mint(&m, "Aurora Points", "AURA", "https://aurora.example/points.json")
        .expect("create_points_mint");

    let raw = env.svm.get_account(&m.points_mint).expect("mint exists");
    assert_eq!(raw.owner, TOKEN_2022_ID);

    let state = StateWithExtensions::<MintState>::unpack(&raw.data).expect("token-2022 mint");

    // Points are whole things a cashier can count.
    assert_eq!(state.base.decimals, 0);
    assert_eq!(state.base.supply, 0);

    // Only the merchant PDA may mint, and this program is the only thing that can sign for it.
    assert_eq!(
        state.base.mint_authority,
        Some(m.merchant).try_into().unwrap()
    );
    // No freeze authority: a merchant that could freeze a customer's points would be a custodian
    // of them. Points stop moving because the hook says so, not because an issuer sulked.
    assert_eq!(state.base.freeze_authority, None.try_into().unwrap());

    let hook = state.get_extension::<TransferHook>().expect("TransferHook");
    assert_eq!(
        Option::<anchor_lang::prelude::Pubkey>::from(hook.program_id),
        Some(obligo_hook::ID)
    );
    // The rug that this closes: an authority here could repoint the mint at a hook that waves
    // everything through, and every point in circulation would stop being a backed liability.
    assert_eq!(
        Option::<anchor_lang::prelude::Pubkey>::from(hook.authority),
        None,
        "the transfer-hook authority must be None, forever"
    );

    let pointer = state
        .get_extension::<MetadataPointer>()
        .expect("MetadataPointer");
    assert_eq!(
        Option::<anchor_lang::prelude::Pubkey>::from(pointer.metadata_address),
        Some(m.points_mint),
        "the mint describes itself; no Metaplex account to go stale"
    );

    let metadata: TokenMetadata = state
        .get_variable_len_extension::<TokenMetadata>()
        .expect("TokenMetadata");
    assert_eq!(metadata.name, "Aurora Points");
    assert_eq!(metadata.symbol, "AURA");
    assert_eq!(metadata.uri, "https://aurora.example/points.json");
    assert_eq!(metadata.mint, m.points_mint);

    // NonTransferable is deliberately absent: combined with TransferHook it bricks every
    // transfer with error 37 and the hook never runs at all. Non-transferability comes from the
    // hook, which is programmable.
    let extensions = state.get_extension_types().unwrap();
    assert!(!extensions.contains(&ExtensionType::NonTransferable));

    // The merchant's books now know where its points live.
    let state = env.merchant_state(&m);
    assert_eq!(state.points_mint, m.points_mint);
    assert_ne!(state.mint_bump, 0);
}

/// A mint without its ExtraAccountMetaList is a mint whose points cannot move, and whose EAML
/// the next person to come along gets to write. Both are created by the same instruction, so
/// neither state is reachable.
#[test]
fn the_mint_cannot_exist_without_its_extra_account_meta_list() {
    let mut env = Env::new();
    let m = env.register_merchant("Cafe Aurora", 10_000, 3000, 86_400);

    let eaml = eaml_address(&m.points_mint);
    assert!(env.svm.get_account(&eaml).is_none(), "no mint, no EAML");

    env.create_points_mint(&m, "Aurora Points", "AURA", "https://aurora.example/points.json")
        .expect("create_points_mint");

    let raw = env.svm.get_account(&eaml).expect("EAML exists");
    assert_eq!(raw.owner, obligo_hook::ID);

    // One extra account: the permit PDA for whichever source account is sending.
    assert_eq!(raw.data.len(), ExtraAccountMetaList::size_of(1).unwrap());
}

#[test]
fn a_merchant_gets_exactly_one_points_mint() {
    let mut env = Env::new();
    let m = env.register_merchant("Cafe Aurora", 10_000, 3000, 86_400);

    env.create_points_mint(&m, "Aurora Points", "AURA", "https://aurora.example/points.json")
        .expect("first");

    env.create_points_mint(&m, "Aurora Points", "AURA", "https://aurora.example/points.json")
        .expect_err("a second mint would orphan every point issued from the first");
}
