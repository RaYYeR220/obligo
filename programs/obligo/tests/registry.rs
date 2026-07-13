//! Protocol config, the permissionless merchant registry, and the collateral vault.

mod common;

use common::*;
use obligo::math::required_collateral;
use obligo::state::MerchantStatus;
use solana_keypair::Keypair;
use solana_signer::Signer;

#[test]
fn genesis_fixes_the_settlement_asset_and_the_hook() {
    let env = Env::new();
    let protocol = env.protocol_state();

    assert_eq!(protocol.authority, env.protocol_authority.pubkey());
    assert_eq!(protocol.usdc_mint, env.usdc_mint);
    assert_eq!(protocol.hook_program, obligo_hook::ID);
    assert_eq!(protocol.merchant_count, 0);
}

#[test]
fn anyone_may_register_as_a_merchant() {
    let mut env = Env::new();

    let a = env.register_merchant("Cafe Aurora", 10_000, 3000, 86_400);
    let b = env.register_merchant("Bodega Belmont", 5_000, 10_000, 86_400);

    let state = env.merchant_state(&a);
    assert_eq!(state.name, "Cafe Aurora");
    assert_eq!(state.authority, a.authority.pubkey());
    assert_eq!(state.vault, a.vault);
    assert_eq!(state.usdc_per_point, 10_000);
    assert_eq!(state.reserve_bps, 3000);
    assert_eq!(state.collateral, 0);
    assert_eq!(state.points_outstanding, 0);
    assert_eq!(state.status, MerchantStatus::Active);
    // No mint until create_points_mint; nothing can be issued before then.
    assert_eq!(state.points_mint, Default::default());

    assert_eq!(env.merchant_state(&b).reserve_bps, 10_000);
    assert_eq!(env.protocol_state().merchant_count, 2);

    // The vault exists, is empty, and is owned by the merchant PDA.
    assert_eq!(env.token_balance(&a.vault), 0);
}

#[test]
fn terms_outside_the_permitted_range_are_refused() {
    let mut env = Env::new();
    let authority = Keypair::new();
    env.svm
        .airdrop(&authority.pubkey(), 1_000_000_000_000)
        .unwrap();

    // A reserve above 100% is not a reserve, it is a mistake.
    let err = env
        .try_register(&authority, "Overcollateralised", 10_000, 10_001, 86_400)
        .expect_err("reserve_bps > 10_000");
    assert_custom_error(err, E_INVALID_TERMS);

    // A point worth nothing is not a liability, and the whole protocol prices liabilities.
    let err = env
        .try_register(&authority, "Worthless", 0, 3000, 86_400)
        .expect_err("usdc_per_point == 0");
    assert_custom_error(err, E_INVALID_TERMS);

    let err = env
        .try_register(&authority, "Eternal", 10_000, 3000, 0)
        .expect_err("point_ttl == 0");
    assert_custom_error(err, E_INVALID_TERMS);
}

#[test]
fn collateral_can_be_deposited_by_anyone_and_withdrawn_by_the_merchant() {
    let mut env = Env::new();
    let m = env.register_merchant("Cafe Aurora", 10_000, 3000, 86_400);

    env.deposit(&m, 3 * DOLLAR).expect("deposit");
    assert_eq!(env.merchant_state(&m).collateral, 3 * DOLLAR);
    assert_eq!(env.token_balance(&m.vault), 3 * DOLLAR);

    // A merchant with no points and no debts requires nothing, so all of it comes back out.
    env.withdraw(&m, 3 * DOLLAR).expect("withdraw");
    assert_eq!(env.merchant_state(&m).collateral, 0);
    assert_eq!(env.token_balance(&m.vault), 0);

    // And it cannot withdraw what it never had.
    let err = env.withdraw(&m, 1).expect_err("empty vault");
    assert_custom_error(err, E_INSUFFICIENT_COLLATERAL);
}

/// The invariant is checked against the books as they would stand AFTER the withdrawal.
/// Checking it beforehand would let a merchant walk out with the reserve backing its points.
#[test]
fn withdrawal_below_the_reserve_is_refused() {
    let mut env = Env::new();
    let m = env.register_merchant("Cafe Aurora", 10_000, 3000, 86_400);

    env.deposit(&m, 5 * DOLLAR).expect("deposit");
    env.set_points_outstanding(&m, 1000);

    // 1000 points at $0.01 = $10.00 face; a 30% reserve pins $3.00 in place.
    let required = required_collateral(0, 1000, 10_000, 3000).unwrap();
    assert_eq!(required, 3 * DOLLAR);

    // $2.00 out of $5.00 leaves exactly the reserve. That is allowed.
    env.withdraw(&m, 2 * DOLLAR).expect("down to the reserve");
    assert_eq!(env.merchant_state(&m).collateral, 3 * DOLLAR);

    // One more micro-unit is not.
    let err = env
        .withdraw(&m, 1)
        .expect_err("the reserve is not the merchant's money");
    assert_custom_error(err, E_RESERVE_BREACHED);

    assert_eq!(env.merchant_state(&m).collateral, 3 * DOLLAR);
    assert_eq!(env.token_balance(&m.vault), 3 * DOLLAR);
}

/// This is the test that says we are a protocol and not a custodian.
///
/// The protocol authority funds genesis, owns the global config, and is the closest thing this
/// system has to an admin. It still cannot take a single micro-dollar out of a merchant's vault,
/// because the merchant PDA is derived from the merchant's own key: a different signer derives a
/// different PDA, and the seeds check fails before the transfer is ever built.
#[test]
fn the_protocol_authority_cannot_withdraw_a_merchants_collateral() {
    let mut env = Env::new();
    let m = env.register_merchant("Cafe Aurora", 10_000, 3000, 86_400);
    env.deposit(&m, 3 * DOLLAR).expect("deposit");

    let admin = env.protocol_authority.insecure_clone();
    let thief_account = env.usdc_account(&admin.pubkey(), 0);

    let err = env
        .withdraw_as(&admin, m.merchant, m.vault, thief_account, 3 * DOLLAR)
        .expect_err("the protocol authority has no claim on a merchant's collateral");
    assert_custom_error(err, E_CONSTRAINT_SEEDS);

    // An unrelated third party gets exactly as far.
    let outsider = Keypair::new();
    env.svm
        .airdrop(&outsider.pubkey(), 1_000_000_000_000)
        .unwrap();
    let outsider_account = env.usdc_account(&outsider.pubkey(), 0);
    let err = env
        .withdraw_as(&outsider, m.merchant, m.vault, outsider_account, 3 * DOLLAR)
        .expect_err("nor does anybody else");
    assert_custom_error(err, E_CONSTRAINT_SEEDS);

    assert_eq!(env.merchant_state(&m).collateral, 3 * DOLLAR);
    assert_eq!(env.token_balance(&m.vault), 3 * DOLLAR);
    assert_eq!(env.token_balance(&thief_account), 0);
}

#[test]
fn a_merchant_cannot_reprice_points_that_are_already_in_the_wild() {
    let mut env = Env::new();
    let m = env.register_merchant("Cafe Aurora", 10_000, 3000, 86_400);
    env.deposit(&m, 3 * DOLLAR).expect("deposit");

    // Free to re-price its own risk while nothing is outstanding.
    env.set_terms(&m, 20_000, 5000, 172_800)
        .expect("no points yet");
    assert_eq!(env.merchant_state(&m).usdc_per_point, 20_000);

    env.set_terms(&m, 10_000, 3000, 86_400).expect("back again");
    env.set_points_outstanding(&m, 1000);

    // Now the face value is a promise printed on a thousand points in customers' pockets.
    let err = env
        .set_terms(&m, 1, 3000, 86_400)
        .expect_err("a merchant may not inflate its way out of its own liabilities");
    assert_custom_error(err, E_TERMS_LOCKED);

    // Raising the reserve is always allowed: it only ever makes the merchant safer.
    env.set_terms(&m, 10_000, 3000, 86_400)
        .expect("unchanged terms");

    // Raising it beyond what the merchant can back is not — the invariant is re-checked.
    let err = env
        .set_terms(&m, 10_000, 4000, 86_400)
        .expect_err("$10.00 face at a 40% reserve needs $4.00, and only $3.00 is posted");
    assert_custom_error(err, E_RESERVE_BREACHED);
}
