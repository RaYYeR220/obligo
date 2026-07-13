//! Expiry and breakage: the points nobody ever spends.
//!
//! Breakage is the oldest revenue line in retail. Somewhere between 10% and 30% of loyalty points
//! are never redeemed, and the liability behind them is quietly written back to income in a back
//! office, on a schedule the people holding the points never see. It is the single largest reason
//! loyalty programmes are profitable, and the single least visible thing about them.
//!
//! Here it is an instruction. Anyone may call it. It cannot be called a second before the deadline
//! the merchant itself published on chain. It emits an event that says exactly how much of a promise
//! was cancelled and what it was worth. And the collateral it frees is freed by arithmetic, not by
//! policy: `points_outstanding` falls, so `required_collateral` falls, so the merchant may now
//! withdraw money that a minute ago it could not.
//!
//! The mechanism is worth reading closely. Token-2022 does **not** invoke a transfer hook on `Burn`,
//! so burning a customer's points where they sit would route around the one component that makes a
//! point unmovable-by-default — the protocol quietly exempting itself from its own rule. So expiry
//! *moves* the points, through a real transfer, with the hook on the critical path and a permit of
//! kind `Expire`, and only then burns them out of an account the protocol owns.

mod common;

use common::*;
use obligo::events::Breakage;
use obligo::state::MerchantStatus;
use solana_keypair::Keypair;
use solana_signer::Signer;

/// A day is the TTL `Env::issuer` declares. Aurora prints 1000 points against exactly the reserve
/// they require: $10.00 of face, 30%, $3.00 in the vault.
fn dormant_customer(env: &mut Env) -> (MerchantHandle, Keypair) {
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 3 * DOLLAR);
    let customer = Keypair::new();
    env.issue(&aurora, &customer.pubkey(), 1000).expect("issue");
    (aurora, customer)
}

/// The deadline is the merchant's own, published, and the chain holds it to it in both directions.
/// One second early is early.
#[test]
fn points_cannot_be_expired_before_their_time() {
    let mut env = Env::new();
    let (aurora, customer) = dormant_customer(&mut env);

    let err = env
        .expire(&aurora, &customer.pubkey())
        .expect_err("the TTL has not run");
    assert_custom_error(err, E_NOT_YET_EXPIRED);

    // One second short of a day, and still not.
    env.warp(86_399);
    let err = env
        .expire(&aurora, &customer.pubkey())
        .expect_err("one second short is short");
    assert_custom_error(err, E_NOT_YET_EXPIRED);

    assert_eq!(env.merchant_state(&aurora).points_outstanding, 1000);
    assert_eq!(env.points_balance(&aurora, &customer.pubkey()), 1000);
    assert_eq!(env.points_supply(&aurora), 1000);
}

/// And then it is not early. Anyone may turn the crank — the customer does not sign, the merchant
/// does not sign, a stranger does — and the promise is cancelled in public.
#[test]
fn after_the_ttl_anyone_may_expire_a_dormant_customers_points() {
    let mut env = Env::new();
    let (aurora, customer) = dormant_customer(&mut env);

    env.warp(86_400);

    let stranger = env.stranger();
    assert_ne!(stranger.pubkey(), aurora.authority.pubkey());
    assert_ne!(stranger.pubkey(), customer.pubkey());

    let meta = env
        .expire_as(&stranger, &aurora, &customer.pubkey())
        .expect("expiry needs nobody's permission, least of all the customer's");

    // The points are gone from the customer's wallet and gone from the supply. Not parked, not
    // frozen, not "marked expired" while sitting in a wallet the merchant cannot reach — gone.
    assert_eq!(env.points_balance(&aurora, &customer.pubkey()), 0);
    assert_eq!(env.points_supply(&aurora), 0);
    assert_eq!(env.escrow_balance(&aurora), 0, "the escrow is a turnstile");
    assert_eq!(env.batch_state(&aurora, &customer.pubkey()).amount, 0);

    let a = env.merchant_state(&aurora);
    assert_eq!(a.points_outstanding, 0);
    assert_eq!(a.total_expired, 1000);
    assert_eq!(a.total_issued, 1000);
    assert_eq!(a.total_redeemed, 0);
    assert_eq!(a.collateral, 3 * DOLLAR, "not one cent of USDC moved");

    let event = decode_event::<Breakage>(&meta);
    assert_eq!(event.merchant, aurora.merchant);
    assert_eq!(event.customer, customer.pubkey());
    assert_eq!(event.points, 1000);
    assert_eq!(
        event.face_value,
        10 * DOLLAR,
        "the promise that was cancelled"
    );
}

/// Expiry has to go through the hook, and this is the assertion that proves it did.
///
/// The core grants a permit of kind `Expire` for exactly the points being burned; Token-2022 fires
/// the hook during the transfer into the escrow; the hook spends the permit down to nothing. If the
/// implementation had taken the shortcut — burning the customer's points where they sat, which
/// Token-2022 would have allowed without ever calling the hook — there would be no permit here at
/// all, and the protocol would have exempted itself from the one rule it enforces on everybody else.
#[test]
fn expiry_moves_the_points_through_the_hook_like_everything_else() {
    let mut env = Env::new();
    let (aurora, customer) = dormant_customer(&mut env);
    let source = env.points_account(&aurora, &customer.pubkey());

    // Issuance is a `MintTo`, which never touches the hook. Nothing has ever authorised these
    // points to move anywhere.
    assert!(env.permit_state(&source).is_none());

    env.warp(86_400);
    env.expire(&aurora, &customer.pubkey()).expect("expire");

    let permit = env.permit_state(&source).expect("the expiry granted one");
    assert_eq!(permit.source, source, "bound to this account alone");
    assert_eq!(permit.kind, 1, "kind = Expire");
    assert_eq!(
        permit.amount, 0,
        "the hook ran, and consumed every point it was given"
    );
}

/// Why expiry is worth an instruction at all: it gives the merchant its money back.
///
/// Before the TTL, Aurora's $3.00 is locked — it is the 30% reserve behind 1000 live points, and
/// `withdraw_collateral` will not release a cent of it. After the TTL and the crank, those points
/// are not a liability any more, the reserve behind them is nothing, and the same withdrawal that
/// was refused a moment ago goes through for the full amount. That is breakage recognised as
/// revenue, done in the open.
#[test]
fn expiry_frees_the_reserve_the_points_were_holding_hostage() {
    let mut env = Env::new();
    let (aurora, customer) = dormant_customer(&mut env);

    assert_eq!(env.required_collateral(&aurora), 3 * DOLLAR);
    let err = env
        .withdraw(&aurora, 1)
        .expect_err("every cent of it is spoken for");
    assert_custom_error(err, E_RESERVE_BREACHED);

    env.warp(86_400);
    env.expire(&aurora, &customer.pubkey()).expect("expire");

    assert_eq!(
        env.required_collateral(&aurora),
        0,
        "nothing is owed to anyone"
    );
    assert_eq!(env.health_bps(&aurora), u64::MAX);

    env.withdraw(&aurora, 3 * DOLLAR)
        .expect("the reserve those points were holding is Aurora's again");

    assert_eq!(env.merchant_state(&aurora).collateral, 0);
    assert_eq!(env.token_balance(&aurora.vault), 0);
}

/// A lapsed point cannot be spent, before or after anybody gets around to burning it. The clock
/// kills the point; the crank only tidies up after it.
#[test]
fn expired_points_cannot_be_redeemed() {
    let mut env = Env::new();
    let (aurora, customer) = dormant_customer(&mut env);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 3 * DOLLAR);

    env.warp(86_400);

    let expires_at = env.now() + 30 * 86_400;
    env.post_offer(&belmont, &aurora, 11_000, 250 * DOLLAR, expires_at)
        .expect("Belmont's bid is perfectly live");

    // Before the crank. The books still say 1000 points are outstanding, the wallet still holds
    // them, and they still buy nothing.
    assert_eq!(env.merchant_state(&aurora).points_outstanding, 1000);
    let err = env
        .redeem(&aurora, &belmont, &customer, 500)
        .expect_err("the points are dead whether or not anyone has noticed");
    assert_custom_error(err, E_POINTS_EXPIRED);

    // After it, they are dead and gone.
    env.expire(&aurora, &customer.pubkey()).expect("expire");
    let err = env
        .redeem(&aurora, &belmont, &customer, 500)
        .expect_err("and now there is nothing there at all");
    assert_custom_error(err, E_POINTS_EXPIRED);

    assert_eq!(env.merchant_state(&aurora).obligations_out, 0);
    assert_eq!(env.merchant_state(&belmont).obligations_in, 0);
    assert_eq!(env.offer_state(&belmont, &aurora).consumed, 0);
}

/// The TTL runs from the customer's *last activity*, not from a point's birthday — which is how
/// every real loyalty programme states it, and which means a customer who keeps shopping never
/// loses the points they earned first.
#[test]
fn shopping_again_resets_the_clock_on_everything_the_customer_holds() {
    let mut env = Env::new();
    // $5.00 of collateral, so there is room to print the second batch: 1100 points at a 30% reserve
    // need $3.30, and the invariant would refuse the top-up against Aurora's usual $3.00.
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 5 * DOLLAR);
    let customer = Keypair::new();
    env.issue(&aurora, &customer.pubkey(), 1000).expect("issue");

    // Twelve hours pass, and the customer comes back.
    env.warp(43_200);
    env.issue(&aurora, &customer.pubkey(), 100).expect("issue");
    assert_eq!(env.batch_state(&aurora, &customer.pubkey()).amount, 1100);

    // The original day is up, but the clock restarted when they walked in.
    env.warp(43_200);
    let err = env
        .expire(&aurora, &customer.pubkey())
        .expect_err("this customer is not dormant");
    assert_custom_error(err, E_NOT_YET_EXPIRED);
    assert_eq!(env.points_balance(&aurora, &customer.pubkey()), 1100);

    // A day of silence from the *last* visit, and the whole batch goes together.
    env.warp(43_201);
    env.expire(&aurora, &customer.pubkey()).expect("expire");
    assert_eq!(env.merchant_state(&aurora).total_expired, 1100);
    assert_eq!(env.points_supply(&aurora), 0);
}

/// An empty batch is not a payday. A crank that could be re-run on nothing is a crank somebody will
/// re-run on nothing.
#[test]
fn a_batch_that_holds_no_points_cannot_be_expired() {
    let mut env = Env::new();
    let (aurora, customer) = dormant_customer(&mut env);

    env.warp(86_400);
    env.expire(&aurora, &customer.pubkey()).expect("first");

    let err = env
        .expire(&aurora, &customer.pubkey())
        .expect_err("and there is nothing left to burn");
    assert_custom_error(err, E_INSUFFICIENT_POINTS);

    assert_eq!(env.merchant_state(&aurora).total_expired, 1000);
    assert_eq!(env.points_supply(&aurora), 0);
}

/// A defaulted merchant's points can *only* be expired. Redemption is shut to them, so if expiry
/// were shut too, the dead liability would sit on the books of an estate that is trying to close.
#[test]
fn a_defaulted_merchants_points_can_still_be_expired() {
    let mut env = Env::new();

    // 25% reserve: $3.00 backs $12.00 of face. Aurora prints all of it — 1000 points to a spender
    // and 200 to a customer who will never come back.
    let aurora = env.issuer("Cafe Aurora", 10_000, 2500, 3 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 3 * DOLLAR);

    let spender = Keypair::new();
    let dormant = Keypair::new();
    env.issue(&aurora, &spender.pubkey(), 1000).expect("issue");
    env.issue(&aurora, &dormant.pubkey(), 200).expect("issue");

    let expires_at = env.now() + 30 * 86_400;
    env.post_offer(&belmont, &aurora, 10_000, 250 * DOLLAR, expires_at)
        .expect("post_offer");
    env.redeem(&aurora, &belmont, &spender, 1000)
        .expect("redeem");

    env.liquidate(&aurora, &belmont).expect("liquidate");
    assert_eq!(
        env.merchant_state(&aurora).status,
        MerchantStatus::Defaulted
    );
    assert_eq!(env.merchant_state(&aurora).points_outstanding, 200);

    env.warp(86_400);
    env.expire(&aurora, &dormant.pubkey())
        .expect("a dead issuer's dead points are still expirable");

    let a = env.merchant_state(&aurora);
    assert_eq!(a.points_outstanding, 0);
    assert_eq!(a.total_expired, 200);
    assert_eq!(a.status, MerchantStatus::Defaulted, "and it is still dead");
    assert_eq!(env.points_supply(&aurora), 0);
}

/// Expiry drives four cross-program calls — grant the permit, transfer, the hook Token-2022 fires
/// inside the transfer, burn. Like a redemption, it fits in the compute budget a transaction is
/// given for free, which is what makes a keeper bot cranking a thousand of them a boring problem.
#[test]
fn an_expiry_fits_in_the_default_compute_budget() {
    let mut env = Env::new();
    let (aurora, customer) = dormant_customer(&mut env);

    env.warp(86_400);
    let meta = env.expire(&aurora, &customer.pubkey()).expect("expire");

    assert!(
        meta.compute_units_consumed < 200_000,
        "an expiry burned {} CU",
        meta.compute_units_consumed
    );
    println!("expire_points: {} CU", meta.compute_units_consumed);
}
