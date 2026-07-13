//! Issuance under the reserve invariant.
//!
//! A merchant may promise more than it holds — that is what a fractional reserve *is* — but only
//! by exactly the multiple it declared, and not one point further.

mod common;

use anchor_spl::token_2022::spl_token_2022::{
    extension::{BaseStateWithExtensions, ExtensionType, StateWithExtensions},
    state::Account as TokenAccountState,
};
use common::*;
use solana_keypair::Keypair;
use solana_signer::Signer;

/// The boundary, to the point.
///
/// $3.00 posted, a 30% reserve, points worth $0.01 each. $3.00 backs $10.00 of face value, which
/// is 1000 points. The 1001st point would need $3.003 of reserve against $3.00 posted, and the
/// protocol would rather refuse the merchant than lie to the customer.
#[test]
fn the_reserve_invariant_is_a_hard_boundary() {
    let mut env = Env::new();
    let m = env.issuer("Cafe Aurora", 10_000, 3000, 3 * DOLLAR);
    let customer = Keypair::new().pubkey();

    env.issue(&m, &customer, 1000).expect("1000 points is exactly backed");

    let state = env.merchant_state(&m);
    assert_eq!(state.points_outstanding, 1000);
    assert_eq!(state.total_issued, 1000);
    assert_eq!(state.collateral, 3 * DOLLAR);
    assert_eq!(env.points_balance(&m, &customer), 1000);
    assert_eq!(env.points_supply(&m), 1000);

    // One more point. Not ten, not a hundred — one.
    let err = env
        .issue(&m, &customer, 1)
        .expect_err("the 1001st point is not backed and must not exist");
    assert_custom_error(err, E_RESERVE_BREACHED);

    // And nothing about the failure leaked into the books or into the customer's wallet.
    let state = env.merchant_state(&m);
    assert_eq!(state.points_outstanding, 1000);
    assert_eq!(state.total_issued, 1000);
    assert_eq!(env.points_balance(&m, &customer), 1000);
    assert_eq!(env.points_supply(&m), 1000);

    // The same boundary from a standing start: 1001 in one go is refused too.
    let n = env.issuer("Bodega Belmont", 10_000, 3000, 3 * DOLLAR);
    let err = env
        .issue(&n, &customer, 1001)
        .expect_err("1001 points at once is the same 1001 points");
    assert_custom_error(err, E_RESERVE_BREACHED);
    assert_eq!(env.merchant_state(&n).points_outstanding, 0);

    env.issue(&n, &customer, 1000).expect("1000 still fits");
    assert_eq!(env.merchant_state(&n).points_outstanding, 1000);
}

/// A full reserve is the degenerate case, and it had better behave: $3.00 buys 300 points, not
/// 1000. The invariant is one formula, not two code paths.
#[test]
fn a_fully_reserved_merchant_can_issue_only_what_it_holds() {
    let mut env = Env::new();
    let m = env.issuer("Bodega Belmont", 10_000, 10_000, 3 * DOLLAR);
    let customer = Keypair::new().pubkey();

    env.issue(&m, &customer, 300).expect("300 points = $3.00 face = $3.00 reserve");
    let err = env.issue(&m, &customer, 1).expect_err("nothing left to back it");
    assert_custom_error(err, E_RESERVE_BREACHED);
}

/// Token-2022 does not invoke a transfer hook on `MintTo`. That is not a gap we are papering
/// over — it is the reason issuance accounting lives in the core at all. No permit is granted,
/// none is needed, and none exists afterwards.
#[test]
fn minting_does_not_go_through_the_hook() {
    let mut env = Env::new();
    let m = env.issuer("Cafe Aurora", 10_000, 3000, 3 * DOLLAR);
    let customer = Keypair::new().pubkey();

    env.issue(&m, &customer, 500).expect("issue");

    let ata = env.points_account(&m, &customer);
    assert!(
        env.svm.get_account(&permit_address(&ata)).is_none(),
        "issuance grants no permit, because the hook never ran"
    );

    // The supply the token program believes in, and the liability we booked, are the same number.
    assert_eq!(env.points_supply(&m), env.merchant_state(&m).points_outstanding);
}

/// The customer's account is created by the associated-token program, not by us, and the hook
/// depends on it carrying `TransferHookAccount` — that extension is where Token-2022 raises the
/// `transferring` flag the hook checks. If the ATA came out without it, every redemption would
/// fail at the token layer and the hook would never see a thing.
#[test]
fn the_customers_account_is_born_ready_for_the_hook() {
    let mut env = Env::new();
    let m = env.issuer("Cafe Aurora", 10_000, 3000, 3 * DOLLAR);
    let customer = Keypair::new().pubkey();

    env.issue(&m, &customer, 100).expect("issue");

    let raw = env
        .svm
        .get_account(&env.points_account(&m, &customer))
        .expect("the ATA exists");
    let state = StateWithExtensions::<TokenAccountState>::unpack(&raw.data).expect("token account");

    assert!(state
        .get_extension_types()
        .unwrap()
        .contains(&ExtensionType::TransferHookAccount));
    assert_eq!(state.base.owner, customer);
}

#[test]
fn issuance_accumulates_into_the_customers_batch_and_resets_its_clock() {
    let mut env = Env::new();
    let m = env.issuer("Cafe Aurora", 10_000, 3000, 3 * DOLLAR);
    let customer = Keypair::new().pubkey();

    env.issue(&m, &customer, 400).expect("first visit");
    let first = env.batch_state(&m, &customer);
    assert_eq!(first.amount, 400);
    assert_eq!(first.merchant, m.merchant);
    assert_eq!(first.customer, customer);

    env.warp(60);
    env.issue(&m, &customer, 100).expect("second visit");

    let second = env.batch_state(&m, &customer);
    assert_eq!(second.amount, 500);
    // The TTL runs from last activity, which is how every loyalty programme on earth words it.
    assert!(second.issued_at > first.issued_at);

    // A different customer gets a different batch, not a share of this one.
    let other = Keypair::new().pubkey();
    env.issue(&m, &other, 100).expect("another customer");
    assert_eq!(env.batch_state(&m, &other).amount, 100);
    assert_eq!(env.batch_state(&m, &customer).amount, 500);
    assert_eq!(env.merchant_state(&m).points_outstanding, 600);
}

/// An issuer's promise is its own. It cannot be made by somebody else.
#[test]
fn a_merchant_cannot_mint_another_merchants_points() {
    let mut env = Env::new();
    let a = env.issuer("Cafe Aurora", 10_000, 3000, 3 * DOLLAR);
    let b = env.issuer("Bodega Belmont", 10_000, 3000, 3 * DOLLAR);
    let customer = Keypair::new().pubkey();

    // B signs, B's merchant account, but A's mint: the merchant does not own that mint.
    let err = env
        .try_issue(&b.authority, b.merchant, a.points_mint, &customer, 100)
        .expect_err("B may not print A's liabilities");
    assert_custom_error(err, E_CONSTRAINT_HAS_ONE);

    // B signs, but names A's merchant account: a different signer derives a different PDA.
    let err = env
        .try_issue(&b.authority, a.merchant, a.points_mint, &customer, 100)
        .expect_err("nor may B act as A");
    assert_custom_error(err, E_CONSTRAINT_SEEDS);

    assert_eq!(env.merchant_state(&a).points_outstanding, 0);
    assert_eq!(env.points_supply(&a), 0);
}
