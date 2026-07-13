//! The points mint: Token-2022, transfer-hooked, self-describing, and un-repointable.

mod common;

use anchor_spl::token_2022::spl_token_2022::{
    extension::{
        metadata_pointer::MetadataPointer, permanent_delegate::PermanentDelegate,
        transfer_hook::TransferHook, BaseStateWithExtensions, ExtensionType, StateWithExtensions,
    },
    state::Mint as MintState,
};
use anchor_spl::token_2022_extensions::spl_token_metadata_interface::state::TokenMetadata;
use common::*;
use solana_signer::Signer;
use spl_tlv_account_resolution::state::ExtraAccountMetaList;

/// The whole security story of this mint is in what it *cannot* do.
#[test]
fn the_points_mint_is_hooked_and_the_hook_can_never_be_repointed() {
    let mut env = Env::new();
    let m = env.register_merchant("Cafe Aurora", 10_000, 3000, 86_400);

    env.create_points_mint(
        &m,
        "Aurora Points",
        "AURA",
        "https://aurora.example/points.json",
    )
    .expect("create_points_mint");

    let raw = env.svm.get_account(&m.points_mint).expect("mint exists");
    assert_eq!(raw.owner, TOKEN_2022_ID);

    let state = StateWithExtensions::<MintState>::unpack(&raw.data).expect("token-2022 mint");

    // Points are whole things a cashier can count.
    assert_eq!(state.base.decimals, 0);
    assert_eq!(state.base.supply, 0);

    // Only the merchant PDA may mint, and this program is the only thing that can sign for it.
    assert_eq!(state.base.mint_authority, Some(m.merchant).into());
    // No freeze authority: a merchant that could freeze a customer's points would be a custodian
    // of them. Points stop moving because the hook says so, not because an issuer sulked.
    assert_eq!(
        state.base.freeze_authority,
        Option::<anchor_lang::prelude::Pubkey>::None.into()
    );

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

/// The extension everybody warns you about, and why it is here.
///
/// A permanent delegate can move any holder's tokens without their signature. `expire_points` needs
/// exactly that — a lapsed point has to be burned out of a customer's account by a crank the
/// customer will never sign — and every alternative is worse: `approve` needs the customer's
/// signature at issuance and can be revoked the next block, and an "expiry" that only edits the
/// books while the tokens sit in the wallet is a lie the mint's own supply would contradict.
///
/// What makes it safe is *who the delegate is*. Not the merchant's keypair — the merchant **PDA**,
/// which no private key on earth can sign for. Only this program can, and it uses it as a transfer
/// authority in exactly one instruction, behind the TTL. And even then the movement still goes
/// through the hook and still needs a permit.
#[test]
fn the_permanent_delegate_is_a_pda_that_only_the_program_can_sign_for() {
    let mut env = Env::new();
    let m = env.register_merchant("Cafe Aurora", 10_000, 3000, 86_400);

    env.create_points_mint(
        &m,
        "Aurora Points",
        "AURA",
        "https://aurora.example/points.json",
    )
    .expect("create_points_mint");

    let raw = env.svm.get_account(&m.points_mint).expect("mint exists");
    let state = StateWithExtensions::<MintState>::unpack(&raw.data).expect("token-2022 mint");

    let delegate = state
        .get_extension::<PermanentDelegate>()
        .expect("PermanentDelegate");
    assert_eq!(
        Option::<anchor_lang::prelude::Pubkey>::from(delegate.delegate),
        Some(m.merchant),
        "the delegate is the merchant PDA, not the merchant's keypair"
    );

    // And that PDA is ours: derived from `[b"merchant", authority]` under this program, which is
    // the only program that can produce a signature for it. The merchant's own keypair cannot.
    assert_eq!(merchant_address(&m.authority.pubkey()), m.merchant);
    assert_ne!(m.merchant, m.authority.pubkey());
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

    env.create_points_mint(
        &m,
        "Aurora Points",
        "AURA",
        "https://aurora.example/points.json",
    )
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

    env.create_points_mint(
        &m,
        "Aurora Points",
        "AURA",
        "https://aurora.example/points.json",
    )
    .expect("first");

    env.create_points_mint(
        &m,
        "Aurora Points",
        "AURA",
        "https://aurora.example/points.json",
    )
    .expect_err("a second mint would orphan every point issued from the first");
}
