//! Cross-merchant redemption: the whole protocol in one instruction.
//!
//! A customer walks into Bodega Belmont holding Cafe Aurora's points. Belmont has a standing bid
//! for them. The points move — through the hook, which is the only thing in the system that can
//! say no — into Aurora's escrow, and are burned. Belmont hands over goods and is owed face by
//! Aurora. Not one cent of USDC moves.
//!
//! What that does to Aurora is the point. Before the redemption it held a *reserve* against points
//! at large: 30% of face, a fraction, because a loyalty point is a probabilistic liability and
//! most of them are never spent. After it, those points have been spent — the liability is real,
//! it is due to a named creditor, and it is owed in full. Aurora's health falls, and it is
//! supposed to. A protocol that refused the redemption to protect the issuer's ratio would be
//! protecting the issuer from its own promise.

mod common;

use common::*;
use obligo::state::MerchantStatus;
use solana_keypair::Keypair;
use solana_signer::Signer;

/// Aurora issues $0.01 points against a 30% reserve, and posts exactly enough collateral for the
/// 1000 points it is about to print ($10.00 of face, $3.00 of reserve). Belmont bids 110%.
fn scene(env: &mut Env) -> (MerchantHandle, MerchantHandle, Keypair) {
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 3 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 3 * DOLLAR);
    let customer = Keypair::new();

    env.issue(&aurora, &customer.pubkey(), 1000).expect("issue");

    let expires_at = env.now() + 30 * 86_400;
    env.post_offer(&belmont, &aurora, 11_000, 250 * DOLLAR, expires_at)
        .expect("post_offer");

    (aurora, belmont, customer)
}

#[test]
fn a_redemption_moves_a_liability_and_not_one_cent_of_usdc() {
    let mut env = Env::new();
    let (aurora, belmont, customer) = scene(&mut env);

    let aurora_vault = env.token_balance(&aurora.vault);
    let belmont_vault = env.token_balance(&belmont.vault);

    let meta = env
        .redeem(&aurora, &belmont, &customer, 500)
        .expect("redeem 500 of Aurora's points at Belmont");

    // The customer keeps what they did not spend, and the 500 they did spend are gone from the
    // supply — not parked somewhere, gone.
    assert_eq!(env.points_balance(&aurora, &customer.pubkey()), 500);
    assert_eq!(env.points_supply(&aurora), 500);
    assert_eq!(env.batch_state(&aurora, &customer.pubkey()).amount, 500);
    assert_eq!(
        env.token_balance(&associated_token_address(&aurora.merchant, &aurora.points_mint)),
        0,
        "the escrow is a turnstile, not a vault"
    );

    // Aurora: half its points came home, and $5.00 of fractional reserve became $5.00 of debt.
    let a = env.merchant_state(&aurora);
    assert_eq!(a.points_outstanding, 500);
    assert_eq!(a.total_redeemed, 500);
    assert_eq!(a.total_issued, 1000);
    assert_eq!(a.obligations_out, 5 * DOLLAR);
    assert_eq!(a.obligations_in, 0);

    // Belmont is owed it.
    let b = env.merchant_state(&belmont);
    assert_eq!(b.obligations_in, 5 * DOLLAR);
    assert_eq!(b.obligations_out, 0);

    // And the debt is a named edge in the graph, not an aggregate.
    let edge = env.obligation_state(&aurora, &belmont);
    assert_eq!(edge.debtor, aurora.merchant);
    assert_eq!(edge.creditor, belmont.merchant);
    assert_eq!(edge.amount, 5 * DOLLAR);
    assert_ne!(edge.bump, 0);

    // Belmont's budget for Aurora's customers is drawn down by the FACE it will claim, not by the
    // goods it gave away. Its acquisition cost is the difference, and it chose it.
    let offer = env.offer_state(&belmont, &aurora);
    assert_eq!(offer.consumed, 5 * DOLLAR);
    assert_eq!(offer.capacity, 250 * DOLLAR);

    // What the till is told: claim $5.00 from Aurora, hand the customer $5.50 of goods.
    let event = decode_redeemed(&meta);
    assert_eq!(event.issuer, aurora.merchant);
    assert_eq!(event.acceptor, belmont.merchant);
    assert_eq!(event.customer, customer.pubkey());
    assert_eq!(event.points, 500);
    assert_eq!(event.value_face, 5_000_000);
    assert_eq!(event.goods_value, 5_500_000);
    assert_eq!(event.rate_bps, 11_000);
    assert_eq!(event.obligation, 5 * DOLLAR);

    // And the money. There isn't any. Both vaults are exactly where they were: a redemption is a
    // transfer of *liability*, and settling it in cash is a separate decision, made later, by
    // whoever cares to crank it.
    assert_eq!(env.token_balance(&aurora.vault), aurora_vault);
    assert_eq!(env.token_balance(&belmont.vault), belmont_vault);
    assert_eq!(env.merchant_state(&aurora).collateral, 3 * DOLLAR);
    assert_eq!(env.merchant_state(&belmont).collateral, 3 * DOLLAR);
}

/// Redemption is deliberately not gated on the issuer's collateral, and this is what that costs
/// the issuer. Aurora was exactly reserved before — health 10_000, one to one. Afterwards it owes
/// $5.00 outright and still reserves 30% against the 500 points still out there: $6.50 required
/// against $3.00 held. It is now insolvent, in public, and anyone can see it.
///
/// That is not a bug being tolerated. It is the product. A point that could not turn into a real
/// debt would not be a liability at all, and there would be nothing to clear.
#[test]
fn redemption_lowers_the_issuers_health() {
    let mut env = Env::new();
    let (aurora, belmont, customer) = scene(&mut env);

    let before = env.health_bps(&aurora);
    assert_eq!(before, 10_000, "exactly reserved: $3.00 against $3.00 required");

    env.redeem(&aurora, &belmont, &customer, 500).expect("redeem");

    let after = env.health_bps(&aurora);
    assert!(
        after < before,
        "redemption must lower health: {after} is not below {before}"
    );
    // $3.00 * 10_000 / $6.50 = 4615 bps.
    assert_eq!(after, 4_615);

    // Belmont, holding the other side of it, is not harmed by carrying the claim.
    assert_eq!(env.health_bps(&belmont), u64::MAX);
}

/// The hook is on the critical path of every redemption, and it is on it exactly once.
///
/// The core grants a permit for precisely the points being redeemed; Token-2022 fires the hook
/// during the transfer; the hook spends the permit down to nothing. What is left behind is a
/// permit for zero — which is the same thing, to the hook, as no permit at all. There is no
/// residue to replay, and the next redemption has to ask again.
#[test]
fn the_permit_is_spent_to_zero_by_the_redemption() {
    let mut env = Env::new();
    let (aurora, belmont, customer) = scene(&mut env);
    let source = env.points_account(&aurora, &customer.pubkey());

    // Issuance does not go through the hook, so nothing has ever authorised these points to move.
    assert!(
        env.permit_state(&source).is_none(),
        "no permit exists until the clearing house grants one"
    );

    env.redeem(&aurora, &belmont, &customer, 500).expect("redeem");

    let permit = env.permit_state(&source).expect("the redemption granted one");
    assert_eq!(permit.source, source, "and bound it to this account alone");
    assert_eq!(permit.kind, 0, "kind = Redeem");
    assert_eq!(
        permit.amount, 0,
        "the hook ran, and consumed every point it was given"
    );

    // A fresh redemption gets a fresh permit — and spends that one to zero too.
    env.redeem(&aurora, &belmont, &customer, 300)
        .expect("second redemption");
    assert_eq!(env.permit_state(&source).unwrap().amount, 0);

    assert_eq!(env.points_balance(&aurora, &customer.pubkey()), 200);
    assert_eq!(env.merchant_state(&aurora).points_outstanding, 200);
    assert_eq!(env.merchant_state(&aurora).obligations_out, 8 * DOLLAR);
    assert_eq!(env.obligation_state(&aurora, &belmont).amount, 8 * DOLLAR);
}

/// No bid, no acceptance. A merchant that has never said it will honour Aurora's points cannot be
/// made to: there is no offer account, and the instruction has nowhere to go.
#[test]
fn points_cannot_be_redeemed_where_there_is_no_offer() {
    let mut env = Env::new();
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 3 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 3 * DOLLAR);
    let customer = Keypair::new();
    env.issue(&aurora, &customer.pubkey(), 1000).expect("issue");

    let err = env
        .redeem(&aurora, &belmont, &customer, 500)
        .expect_err("Belmont never bid for these");
    assert_custom_error(err, E_ACCOUNT_NOT_INITIALIZED);

    // Nothing happened to anybody.
    assert_eq!(env.merchant_state(&aurora).points_outstanding, 1000);
    assert_eq!(env.merchant_state(&aurora).obligations_out, 0);
    assert_eq!(env.points_balance(&aurora, &customer.pubkey()), 1000);
}

/// A bid with a date on it stops being a bid on that date.
#[test]
fn an_expired_offer_cannot_be_redeemed_against() {
    let mut env = Env::new();
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 3 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 3 * DOLLAR);
    let customer = Keypair::new();
    env.issue(&aurora, &customer.pubkey(), 1000).expect("issue");

    let expires_at = env.now() + 3_600;
    env.post_offer(&belmont, &aurora, 11_000, 250 * DOLLAR, expires_at)
        .expect("post_offer");

    env.redeem(&aurora, &belmont, &customer, 100)
        .expect("inside the hour, the bid stands");

    env.warp(3_601);

    let err = env
        .redeem(&aurora, &belmont, &customer, 100)
        .expect_err("outside it, it does not");
    assert_custom_error(err, E_OFFER_EXPIRED);

    assert_eq!(env.merchant_state(&aurora).points_outstanding, 900);
    assert_eq!(env.offer_state(&belmont, &aurora).consumed, DOLLAR);
}

/// `capacity` is the acceptor's customer-acquisition budget, and it is the one line of an offer
/// the chain will not let it overrun — not by mistake, not by a bug in its own till, not by a
/// customer arriving with more points than it bargained for.
#[test]
fn a_redemption_beyond_the_offers_capacity_is_refused() {
    let mut env = Env::new();
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 3 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 3 * DOLLAR);
    let customer = Keypair::new();
    env.issue(&aurora, &customer.pubkey(), 1000).expect("issue");

    // Belmont will absorb $3.00 of Aurora's face value and not a cent more.
    let expires_at = env.now() + 30 * 86_400;
    env.post_offer(&belmont, &aurora, 11_000, 3 * DOLLAR, expires_at)
        .expect("post_offer");

    // 500 points is $5.00 of face. Belmont budgeted three.
    let err = env
        .redeem(&aurora, &belmont, &customer, 500)
        .expect_err("$5.00 of face against a $3.00 budget");
    assert_custom_error(err, E_OFFER_EXHAUSTED);

    // The budget is spent to the last cent, and then it is spent.
    env.redeem(&aurora, &belmont, &customer, 300)
        .expect("$3.00 of face fits exactly");
    assert_eq!(env.offer_state(&belmont, &aurora).consumed, 3 * DOLLAR);

    let err = env
        .redeem(&aurora, &belmont, &customer, 1)
        .expect_err("one more point is one point too many");
    assert_custom_error(err, E_OFFER_EXHAUSTED);

    // Belmont's exposure to Aurora is exactly what Belmont said it was willing to take.
    assert_eq!(env.merchant_state(&belmont).obligations_in, 3 * DOLLAR);
    assert_eq!(env.obligation_state(&aurora, &belmont).amount, 3 * DOLLAR);
    assert_eq!(env.points_balance(&aurora, &customer.pubkey()), 700);
}

/// Points expire. A customer cannot redeem a promise that has already lapsed, and — importantly —
/// this holds before anybody has cranked the expiry instruction. The books are not what makes the
/// points dead; the clock is.
#[test]
fn points_past_their_ttl_cannot_be_redeemed() {
    let mut env = Env::new();
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 3 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 3 * DOLLAR);
    let customer = Keypair::new();
    env.issue(&aurora, &customer.pubkey(), 1000).expect("issue");

    // A day and a second: the TTL Aurora declared is 86_400.
    env.warp(86_401);

    let expires_at = env.now() + 30 * 86_400;
    env.post_offer(&belmont, &aurora, 11_000, 250 * DOLLAR, expires_at)
        .expect("Belmont's bid is live");

    let err = env
        .redeem(&aurora, &belmont, &customer, 500)
        .expect_err("the points are not");
    assert_custom_error(err, E_POINTS_EXPIRED);

    // Still on Aurora's books as outstanding — expiring them is somebody else's crank, and until
    // it is turned the reserve stays posted.
    assert_eq!(env.merchant_state(&aurora).points_outstanding, 1000);
    assert_eq!(env.merchant_state(&aurora).obligations_out, 0);
}

/// A defaulted issuer's points are not a currency any more. They are a claim in an estate, and
/// letting an acceptor take one at face would be letting it jump the queue ahead of the creditors
/// already waiting on the same collateral.
#[test]
fn a_defaulted_issuers_points_cannot_be_redeemed() {
    let mut env = Env::new();
    let (aurora, belmont, customer) = scene(&mut env);

    env.set_status(&aurora, MerchantStatus::Defaulted);

    let err = env
        .redeem(&aurora, &belmont, &customer, 500)
        .expect_err("Aurora has defaulted");
    assert_custom_error(err, E_ISSUER_DEFAULTED);

    assert_eq!(env.merchant_state(&aurora).obligations_out, 0);
    assert_eq!(env.merchant_state(&belmont).obligations_in, 0);
    assert_eq!(env.points_balance(&aurora, &customer.pubkey()), 1000);
}

#[test]
fn a_customer_cannot_redeem_points_it_does_not_hold() {
    let mut env = Env::new();
    let (aurora, belmont, customer) = scene(&mut env);

    let err = env
        .redeem(&aurora, &belmont, &customer, 1001)
        .expect_err("1000 issued, 1001 claimed");
    assert_custom_error(err, E_INSUFFICIENT_POINTS);

    let err = env
        .redeem(&aurora, &belmont, &customer, 0)
        .expect_err("nor nothing at all");
    assert_custom_error(err, E_INVALID_AMOUNT);

    assert_eq!(env.merchant_state(&aurora).points_outstanding, 1000);
}

/// A redemption is four cross-program calls deep — grant the permit, transfer, the hook Token-2022
/// fires inside the transfer, burn — plus, the first time two merchants ever meet, three accounts
/// created from nothing. None of the tests above raise the compute budget, and this one says why
/// out loud: it fits in the 200,000 units a transaction is given for free. A cashier's terminal
/// should not have to reason about compute budgets.
#[test]
fn a_redemption_fits_in_the_default_compute_budget() {
    let mut env = Env::new();
    let (aurora, belmont, customer) = scene(&mut env);

    // The expensive one: creates the escrow, the permit and the obligation edge.
    let first = env.redeem(&aurora, &belmont, &customer, 500).expect("first");
    // And the ordinary one, with all three already there.
    let steady = env.redeem(&aurora, &belmont, &customer, 100).expect("steady");

    assert!(
        first.compute_units_consumed < 200_000,
        "cold redemption burned {} CU",
        first.compute_units_consumed
    );
    assert!(
        steady.compute_units_consumed < first.compute_units_consumed,
        "a warm redemption should be cheaper than the one that built the accounts"
    );
}
