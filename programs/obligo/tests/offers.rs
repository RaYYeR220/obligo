//! The acceptance auction.
//!
//! An offer is a merchant saying, in public and in advance: *I will honour that merchant's points,
//! at this rate, up to this much face value, until this date.* One number carries two meanings.
//! Above 10_000 the acceptor is handing the customer more than face and eating the difference as
//! customer acquisition; below 10_000 it is discounting the issuer's credit. Either way the
//! acceptor claims face — 100% — from the issuer. The rate only ever prices the goods.
//!
//! `capacity` is the budget line. It is the one number here that stops being marketing and starts
//! being a constraint the chain enforces.

mod common;

use common::*;
use solana_keypair::Keypair;
use solana_signer::Signer;

/// $0.01 points, 30% reserve, $3.00 posted — the shape every test in this file starts from.
fn merchants(env: &mut Env) -> (MerchantHandle, MerchantHandle) {
    let issuer = env.register_merchant("Cafe Aurora", 10_000, 3000, 86_400);
    let acceptor = env.register_merchant("Bodega Belmont", 10_000, 3000, 86_400);
    (issuer, acceptor)
}

#[test]
fn an_offer_is_a_rate_and_a_budget() {
    let mut env = Env::new();
    let (issuer, acceptor) = merchants(&mut env);
    let expires_at = env.now() + 30 * 86_400;

    env.post_offer(&acceptor, &issuer, 11_000, 250 * DOLLAR, expires_at)
        .expect("post_offer");

    let offer = env.offer_state(&acceptor, &issuer);
    assert_eq!(offer.acceptor, acceptor.merchant);
    assert_eq!(offer.issuer, issuer.merchant);
    // 110%: the acceptor is bidding for footfall and will hand over $1.10 of goods per $1.00 of
    // face it claims back. The 10% is its advertising budget, priced by it, not by us.
    assert_eq!(offer.rate_bps, 11_000);
    assert_eq!(offer.capacity, 250 * DOLLAR);
    assert_eq!(offer.consumed, 0);
    assert_eq!(offer.expires_at, expires_at);
    assert_ne!(offer.bump, 0);
}

/// The other half of the auction: an acceptor that thinks the issuer is a bad credit takes the
/// points at a discount. Nothing in the program prefers one to the other.
#[test]
fn an_acceptor_may_discount_an_issuers_credit() {
    let mut env = Env::new();
    let (issuer, acceptor) = merchants(&mut env);
    let expires_at = env.now() + 30 * 86_400;

    env.post_offer(&acceptor, &issuer, 8_500, 100 * DOLLAR, expires_at)
        .expect("85% is a perfectly good bid");
    assert_eq!(env.offer_state(&acceptor, &issuer).rate_bps, 8_500);
}

/// Re-posting replaces the offer outright, budget included. An acceptor changing its bid is
/// making a new offer, not amending an old one, and its budget starts again with it.
#[test]
fn re_posting_replaces_the_offer() {
    let mut env = Env::new();
    let (issuer, acceptor) = merchants(&mut env);
    let expires_at = env.now() + 30 * 86_400;

    env.post_offer(&acceptor, &issuer, 11_000, 250 * DOLLAR, expires_at)
        .expect("first bid");
    env.post_offer(&acceptor, &issuer, 9_000, 10 * DOLLAR, expires_at)
        .expect("second thoughts");

    let offer = env.offer_state(&acceptor, &issuer);
    assert_eq!(offer.rate_bps, 9_000);
    assert_eq!(offer.capacity, 10 * DOLLAR);
    assert_eq!(offer.consumed, 0);
}

#[test]
fn a_rate_outside_the_band_is_refused() {
    let mut env = Env::new();
    let (issuer, acceptor) = merchants(&mut env);
    let expires_at = env.now() + 30 * 86_400;

    // Zero would be an acceptor taking points and giving nothing back.
    let err = env
        .post_offer(&acceptor, &issuer, 0, 250 * DOLLAR, expires_at)
        .expect_err("a rate of nothing is not a bid");
    assert_custom_error(err, E_INVALID_RATE);

    // And 200% is where we stop entertaining fat-fingered decimals.
    let err = env
        .post_offer(&acceptor, &issuer, 20_001, 250 * DOLLAR, expires_at)
        .expect_err("beyond 200% is a typo, not a strategy");
    assert_custom_error(err, E_INVALID_RATE);

    env.post_offer(&acceptor, &issuer, 20_000, 250 * DOLLAR, expires_at)
        .expect("200% exactly is still a bid");
    assert_eq!(env.offer_state(&acceptor, &issuer).rate_bps, 20_000);
}

#[test]
fn an_offer_needs_a_budget_and_a_future() {
    let mut env = Env::new();
    let (issuer, acceptor) = merchants(&mut env);
    let now = env.now();

    let err = env
        .post_offer(&acceptor, &issuer, 11_000, 0, now + 30 * 86_400)
        .expect_err("an offer that will absorb nothing is not an offer");
    assert_custom_error(err, E_INVALID_AMOUNT);

    let err = env
        .post_offer(&acceptor, &issuer, 11_000, 250 * DOLLAR, now)
        .expect_err("an offer that has already expired is not an offer");
    assert_custom_error(err, E_OFFER_EXPIRED);
}

/// A merchant honouring its own points is not an acceptance, it is a refund — and it would book
/// itself a debt it also owns, an edge from a node to itself, and a cycle of length one.
#[test]
fn a_merchant_cannot_bid_for_its_own_points() {
    let mut env = Env::new();
    let (_, acceptor) = merchants(&mut env);
    let expires_at = env.now() + 30 * 86_400;

    let err = env
        .post_offer(&acceptor, &acceptor, 11_000, 250 * DOLLAR, expires_at)
        .expect_err("an obligation to oneself is not an obligation");
    assert_custom_error(err, E_SELF_OFFER);
}

/// The rent goes back where it came from, and the offer stops being a promise anyone can rely on.
#[test]
fn cancelling_an_offer_returns_its_rent_to_the_acceptor() {
    let mut env = Env::new();
    let (issuer, acceptor) = merchants(&mut env);
    let expires_at = env.now() + 30 * 86_400;

    let before = env.lamports(&acceptor.authority.pubkey());

    env.post_offer(&acceptor, &issuer, 11_000, 250 * DOLLAR, expires_at)
        .expect("post_offer");
    assert!(env.offer_is_live(&acceptor, &issuer));
    assert!(
        env.lamports(&acceptor.authority.pubkey()) < before,
        "the acceptor paid the rent"
    );

    env.cancel_offer(&acceptor, &issuer).expect("cancel_offer");

    assert!(!env.offer_is_live(&acceptor, &issuer));
    assert_eq!(
        env.lamports(&acceptor.authority.pubkey()),
        before,
        "and got every lamport of it back"
    );
}

/// The offer is the acceptor's own commitment. Nobody else gets to withdraw it — not the issuer
/// whose credit is being priced, not another merchant, not the protocol authority.
#[test]
fn only_the_acceptor_may_cancel_its_own_offer() {
    let mut env = Env::new();
    let (issuer, acceptor) = merchants(&mut env);
    let interloper = env.register_merchant("Third Party", 10_000, 3000, 86_400);
    let expires_at = env.now() + 30 * 86_400;

    env.post_offer(&acceptor, &issuer, 11_000, 250 * DOLLAR, expires_at)
        .expect("post_offer");

    // The issuer would love this offer to go away; it cannot make it.
    let err = env
        .cancel_offer_as(&issuer.authority, acceptor.merchant, issuer.merchant)
        .expect_err("the issuer does not own the acceptor's bid");
    assert_custom_error(err, E_CONSTRAINT_SEEDS);

    // Nor may an unrelated merchant, naming the acceptor's account and signing with its own key.
    let err = env
        .cancel_offer_as(&interloper.authority, acceptor.merchant, issuer.merchant)
        .expect_err("a different signer derives a different merchant");
    assert_custom_error(err, E_CONSTRAINT_SEEDS);

    // Nor a passing stranger with no merchant at all.
    let stranger = Keypair::new();
    env.svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();
    let err = env
        .cancel_offer_as(&stranger, acceptor.merchant, issuer.merchant)
        .expect_err("and certainly not a stranger");
    assert_custom_error(err, E_CONSTRAINT_SEEDS);

    // The offer is untouched, and the acceptor can still withdraw it whenever it likes.
    assert!(env.offer_is_live(&acceptor, &issuer));
    env.cancel_offer(&acceptor, &issuer)
        .expect("the acceptor, and only the acceptor");
    assert!(!env.offer_is_live(&acceptor, &issuer));
}
