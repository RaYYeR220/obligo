//! Liquidation: what happens when a fractional reserve turns out to have been a fraction.
//!
//! A merchant running a 30% reserve has issued points it cannot cover in full. That is not a bug,
//! it is the product — most loyalty points are never spent, and forcing every issuer to lock up
//! 100% of a liability that will mostly evaporate is why nobody does this on chain today. But the
//! moment enough of those points come home, the promise stops being probabilistic and starts being
//! a debt to a named creditor, in full, today. When the debts exceed the vault, the merchant is
//! insolvent, and there is nothing left to wait for.
//!
//! So anyone may liquidate it. Every creditor gets its share of what is there, pro rata with every
//! other creditor's claim, and the shares add up to exactly what was in the vault — never a cent
//! more, however the rounding falls. The rest is written off, on the acceptor that chose to take
//! the risk, at a rate it set itself, up to a budget it capped itself, having read the issuer's
//! health on chain before it did any of it.
//!
//! And afterwards the merchant is not dead. It is defaulted — which is a fact, recorded permanently,
//! that anyone deciding whether to honour its points can read. `reinstate` lets it trade again once
//! it can pay its way. It does not let it pretend.

mod common;

use common::*;
use obligo::events::{Liquidated, Reinstated};
use obligo::state::MerchantStatus;
use solana_keypair::Keypair;
use solana_signer::Signer;

/// Aurora, insolvent, owing $6.00 to Belmont and $6.00 to Cordoba against $3.00 of collateral.
///
/// Built the only way the protocol permits. Aurora declares a 25% reserve, so $3.00 of collateral
/// backs $12.00 of face value — and it must print all $12.00 *before* any of it comes home, because
/// each redemption converts a 25% reserve into a 100% debt and raises the bar for the next
/// issuance. That is the invariant working, and it is why building this state needs no surgery.
fn insolvent(env: &mut Env) -> (MerchantHandle, MerchantHandle, MerchantHandle) {
    let aurora = env.issuer("Cafe Aurora", 10_000, 2500, 3 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 10 * DOLLAR);
    let cordoba = env.issuer("Cordoba Books", 10_000, 3000, 10 * DOLLAR);

    let c1 = Keypair::new();
    let c2 = Keypair::new();
    env.issue(&aurora, &c1.pubkey(), 600).expect("issue");
    env.issue(&aurora, &c2.pubkey(), 600).expect("issue");
    assert_eq!(env.merchant_state(&aurora).points_outstanding, 1200);
    assert_eq!(
        env.health_bps(&aurora),
        10_000,
        "exactly reserved, to the cent"
    );

    let expires_at = env.now() + 30 * 86_400;
    env.post_offer(&belmont, &aurora, 10_000, 6 * DOLLAR, expires_at)
        .expect("post_offer");
    env.post_offer(&cordoba, &aurora, 10_000, 6 * DOLLAR, expires_at)
        .expect("post_offer");

    env.redeem(&aurora, &belmont, &c1, 600).expect("redeem");
    env.redeem(&aurora, &cordoba, &c2, 600).expect("redeem");

    let a = env.merchant_state(&aurora);
    assert_eq!(a.obligations_out, 12 * DOLLAR);
    assert_eq!(a.collateral, 3 * DOLLAR);
    assert_eq!(a.status, MerchantStatus::Active, "not yet");

    (aurora, belmont, cordoba)
}

/// Health below 1.0 is not a licence to liquidate. A merchant with a fractional reserve is *meant*
/// to look under-collateralised against the face value of points still at large — that is what the
/// fraction means. The line that matters is a harder one: can it pay the debts it has actually
/// incurred? Aurora can, so nobody may touch it.
#[test]
fn a_solvent_merchant_cannot_be_liquidated() {
    let mut env = Env::new();
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 3 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 3 * DOLLAR);
    let customer = Keypair::new();

    env.issue(&aurora, &customer.pubkey(), 1000).expect("issue");
    let expires_at = env.now() + 30 * 86_400;
    env.post_offer(&belmont, &aurora, 10_000, 250 * DOLLAR, expires_at)
        .expect("post_offer");
    env.redeem(&aurora, &belmont, &customer, 200)
        .expect("redeem");

    // $3.00 of collateral against $2.00 of real debt — but $6.40 of *required* collateral, once the
    // 30% reserve against the 800 points still out there is counted. Health is 4,687 bps: well
    // under water by the ratio, and perfectly able to pay everything it owes.
    let a = env.merchant_state(&aurora);
    assert_eq!(a.obligations_out, 2 * DOLLAR);
    assert_eq!(a.collateral, 3 * DOLLAR);
    assert!(env.health_bps(&aurora) < 10_000, "the ratio looks bad");
    assert!(env.is_solvent(&aurora), "and the merchant is fine");

    let err = env
        .liquidate(&aurora, &belmont)
        .expect_err("a ratio is not a default");
    assert_custom_error(err, E_NOT_LIQUIDATABLE);

    assert_eq!(env.merchant_state(&aurora).status, MerchantStatus::Active);
    assert_eq!(env.token_balance(&aurora.vault), 3 * DOLLAR);
    assert_eq!(env.token_balance(&belmont.vault), 3 * DOLLAR);
}

/// The money shot. $3.00 in the vault, $12.00 of claims against it, two creditors with identical
/// claims: each gets exactly half of what is there, and the halves add up to all of it.
#[test]
fn an_insolvent_issuer_pays_every_creditor_its_share_and_not_a_cent_more() {
    let mut env = Env::new();
    let (aurora, belmont, cordoba) = insolvent(&mut env);

    let belmont_before = env.token_balance(&belmont.vault);
    let cordoba_before = env.token_balance(&cordoba.vault);

    // $6.00 of a $12.00 claim pool, against a $3.00 estate: $1.50.
    let meta = env.liquidate(&aurora, &belmont).expect("liquidate");

    let event = decode_event::<Liquidated>(&meta);
    assert_eq!(event.debtor, aurora.merchant);
    assert_eq!(event.creditor, belmont.merchant);
    assert_eq!(event.claim, 6 * DOLLAR, "what Belmont was owed");
    assert_eq!(event.paid, 1_500_000, "what the estate could find");
    assert_eq!(event.written_off, 4_500_000, "and what Belmont lost");
    assert_eq!(event.collateral_remaining, 1_500_000);
    assert_eq!(event.obligations_remaining, 6 * DOLLAR);

    assert_eq!(
        env.token_balance(&belmont.vault) - belmont_before,
        1_500_000
    );
    assert_eq!(env.token_balance(&aurora.vault), 1_500_000);

    let a = env.merchant_state(&aurora);
    assert_eq!(a.status, MerchantStatus::Defaulted);
    assert_eq!(a.defaults, 1, "on the record, and it stays there");
    assert_eq!(a.collateral, 1_500_000);
    assert_eq!(a.obligations_out, 6 * DOLLAR, "Cordoba is still owed");
    assert_eq!(
        env.owed(&aurora, &belmont),
        0,
        "Belmont's claim is discharged"
    );

    let b = env.merchant_state(&belmont);
    assert_eq!(b.obligations_in, 0);
    assert_eq!(b.collateral, belmont_before + 1_500_000);

    // Cordoba, arriving second, is not punished for it: its $6.00 claim is now the whole claim
    // pool, and the remaining $1.50 is the whole estate. Identical claim, identical recovery.
    env.liquidate(&aurora, &cordoba).expect("liquidate");
    assert_eq!(
        env.token_balance(&cordoba.vault) - cordoba_before,
        1_500_000
    );

    let a = env.merchant_state(&aurora);
    assert_eq!(a.collateral, 0, "the estate is empty");
    assert_eq!(a.obligations_out, 0, "and there is nothing left to claim");
    assert_eq!(a.defaults, 1, "one default, two creditors");
    assert_eq!(env.token_balance(&aurora.vault), 0);
    assert_eq!(env.owed(&aurora, &cordoba), 0);
    assert_eq!(env.merchant_state(&cordoba).obligations_in, 0);

    // Everything that left the estate went to a creditor. Nothing evaporated, nothing was minted.
    assert_eq!(
        (env.token_balance(&belmont.vault) - belmont_before)
            + (env.token_balance(&cordoba.vault) - cordoba_before),
        3 * DOLLAR
    );
}

/// The property an auditor looks for first: **the payouts can never exceed the estate.**
///
/// The numbers here are chosen to make the division ugly. Three creditors holding $3.33 each against
/// a $9.99 claim pool and a $1.00 vault: no share divides evenly, every one of them rounds down, and
/// the rounding dust must stay in the vault rather than being conjured out of it. After each step
/// the vault holds exactly what has not yet been paid out, and the running total never once passes
/// the dollar that was there to begin with.
#[test]
fn the_payouts_can_never_exceed_the_estate_however_the_rounding_falls() {
    let mut env = Env::new();

    // 10% reserve: $1.00 backs $10.00 of face.
    let aurora = env.issuer("Cafe Aurora", 10_000, 1000, DOLLAR);
    let creditors: Vec<MerchantHandle> = (0..3)
        .map(|i| env.issuer(&format!("Creditor {i}"), 10_000, 3000, 5 * DOLLAR))
        .collect();

    let customers: Vec<Keypair> = (0..3).map(|_| Keypair::new()).collect();
    for c in &customers {
        env.issue(&aurora, &c.pubkey(), 333).expect("issue");
    }
    assert_eq!(env.merchant_state(&aurora).points_outstanding, 999);

    let expires_at = env.now() + 30 * 86_400;
    for (creditor, customer) in creditors.iter().zip(customers.iter()) {
        env.post_offer(creditor, &aurora, 10_000, 10 * DOLLAR, expires_at)
            .expect("post_offer");
        env.redeem(&aurora, creditor, customer, 333)
            .expect("redeem");
    }

    // $9.99 of claims against $1.00.
    assert_eq!(env.merchant_state(&aurora).obligations_out, 9_990_000);
    assert_eq!(env.merchant_state(&aurora).collateral, DOLLAR);
    assert!(!env.is_solvent(&aurora));

    let estate = DOLLAR;
    let before: Vec<u64> = creditors
        .iter()
        .map(|c| env.token_balance(&c.vault))
        .collect();
    let mut paid_out: u64 = 0;

    for (i, creditor) in creditors.iter().enumerate() {
        env.liquidate(&aurora, creditor).expect("liquidate");

        let received = env.token_balance(&creditor.vault) - before[i];
        paid_out += received;

        let a = env.merchant_state(&aurora);

        // The claim on the books is the money in the vault, always.
        assert_eq!(
            a.collateral,
            env.token_balance(&aurora.vault),
            "the books and the vault disagree after payout {i}"
        );
        // Not one cent more than there ever was.
        assert!(
            paid_out <= estate,
            "paid out {paid_out} of an estate of {estate} after {i} liquidations"
        );
        // And the dust is not lost either: what went out plus what is left is what was there.
        assert_eq!(
            paid_out + a.collateral,
            estate,
            "the estate does not add up after payout {i}"
        );
    }

    // 333,333 + 333,333 + 333,334. The floor takes a cent off the first two shares and the last
    // creditor sweeps what it left behind — because its claim is, by then, the entire claim pool.
    assert_eq!(
        paid_out, estate,
        "the estate is distributed to the last cent"
    );
    assert_eq!(env.merchant_state(&aurora).collateral, 0);
    assert_eq!(env.merchant_state(&aurora).obligations_out, 0);

    let received: Vec<u64> = creditors
        .iter()
        .enumerate()
        .map(|(i, c)| env.token_balance(&c.vault) - before[i])
        .collect();
    assert_eq!(received, vec![333_333, 333_333, 333_334]);
}

/// A liquidation nobody may call is a liquidation nobody calls. A stranger with no merchant account,
/// no points and nothing to gain turns the crank, and two merchants who have never heard of them
/// are paid.
#[test]
fn liquidation_is_a_crank_any_stranger_may_turn() {
    let mut env = Env::new();
    let (aurora, belmont, _cordoba) = insolvent(&mut env);

    let stranger = env.stranger();
    assert_ne!(stranger.pubkey(), aurora.authority.pubkey());
    assert_ne!(stranger.pubkey(), belmont.authority.pubkey());

    let before = env.token_balance(&belmont.vault);
    env.liquidate_as(&stranger, &aurora, &belmont)
        .expect("an insolvency needs nobody's permission");

    assert_eq!(env.token_balance(&belmont.vault) - before, 1_500_000);
    assert_eq!(
        env.merchant_state(&aurora).status,
        MerchantStatus::Defaulted
    );
}

/// A merchant holding no claim on the estate cannot help itself to it. The edge is a PDA of the
/// pair, so there is no account it could substitute; the worst it can do is present a real, empty
/// edge — and an empty claim is worth an empty payout.
#[test]
fn a_merchant_with_no_claim_gets_nothing_from_the_estate() {
    let mut env = Env::new();
    let (aurora, belmont, _cordoba) = insolvent(&mut env);

    // Dorset has never honoured one of Aurora's points.
    let dorset = env.issuer("Dorset Deli", 10_000, 3000, 5 * DOLLAR);
    let before = env.token_balance(&dorset.vault);

    let err = env
        .liquidate(&aurora, &dorset)
        .expect_err("Dorset is owed nothing");
    // No edge account exists at all, and a non-existent PDA is not an absence the caller gets to
    // assert — it is an account Anchor refuses to deserialise.
    assert_custom_error(err, E_ACCOUNT_NOT_INITIALIZED);
    assert_eq!(env.token_balance(&dorset.vault), before);

    // And once Belmont has been paid, its own edge is empty. Cranking it again pays nothing.
    env.liquidate(&aurora, &belmont).expect("liquidate");
    let belmont_paid = env.token_balance(&belmont.vault);

    let err = env
        .liquidate(&aurora, &belmont)
        .expect_err("Belmont has already been paid what it is going to be paid");
    assert_custom_error(err, E_NO_CLAIM);
    assert_eq!(env.token_balance(&belmont.vault), belmont_paid);
}

/// A defaulted merchant may not print. It has already handed out promises it could not keep, and
/// the reserve invariant would not stop it printing more against a vault somebody else has just
/// topped up on its behalf.
#[test]
fn a_defaulted_merchant_cannot_issue_points() {
    let mut env = Env::new();
    let (aurora, belmont, cordoba) = insolvent(&mut env);

    env.liquidate(&aurora, &belmont).expect("liquidate");
    env.liquidate(&aurora, &cordoba).expect("liquidate");
    assert_eq!(
        env.merchant_state(&aurora).status,
        MerchantStatus::Defaulted
    );

    // Aurora's backer puts $50 in the vault. It changes nothing about the printing press.
    env.deposit(&aurora, 50 * DOLLAR).expect("deposit");
    assert_eq!(env.merchant_state(&aurora).collateral, 50 * DOLLAR);

    let customer = Keypair::new();
    let err = env
        .issue(&aurora, &customer.pubkey(), 1)
        .expect_err("collateral is not the same thing as standing");
    assert_custom_error(err, E_MERCHANT_DEFAULTED);

    assert_eq!(env.merchant_state(&aurora).points_outstanding, 0);
    assert_eq!(env.points_supply(&aurora), 0);
}

/// Reinstatement, and why it is not a favour to the merchant.
///
/// A defaulted merchant cannot be settled — its vault is an estate — and it can only be liquidated
/// while it is *insolvent*. Between those two lies a state where a creditor can do neither: the
/// merchant is defaulted and solvent, because a ring got cleared or somebody paid its vault up, and
/// the creditor's perfectly good claim sits there waiting on a merchant with no particular reason to
/// act. So the door is opened by the books rather than by the debtor: anyone may reinstate a
/// merchant that can pay.
#[test]
fn a_defaulted_merchant_that_can_pay_its_way_is_reinstated_by_anyone() {
    let mut env = Env::new();
    let (aurora, belmont, cordoba) = insolvent(&mut env);

    env.liquidate(&aurora, &belmont).expect("liquidate");
    env.liquidate(&aurora, &cordoba).expect("liquidate");

    // The estate is empty and so is the claim pool. Aurora owes nothing, and owns nothing.
    assert_eq!(env.merchant_state(&aurora).obligations_out, 0);
    assert_eq!(env.merchant_state(&aurora).collateral, 0);
    assert!(env.is_solvent(&aurora));

    let stranger = env.stranger();
    let meta = env
        .reinstate_as(&stranger, &aurora)
        .expect("solvency is a fact about the books, not a favour");

    let event = decode_event::<Reinstated>(&meta);
    assert_eq!(event.merchant, aurora.merchant);
    assert_eq!(event.obligations_out, 0);
    assert_eq!(
        event.defaults, 1,
        "coming back does not take it off the record"
    );

    let a = env.merchant_state(&aurora);
    assert_eq!(a.status, MerchantStatus::Active);
    assert_eq!(a.defaults, 1);

    // It can trade again — and only as far as its collateral will carry it. Nothing was forgiven
    // except the debt its creditors already took the loss on.
    let customer = Keypair::new();
    let err = env
        .issue(&aurora, &customer.pubkey(), 1)
        .expect_err("with an empty vault it can print exactly nothing");
    assert_custom_error(err, E_RESERVE_BREACHED);

    env.deposit(&aurora, 3 * DOLLAR).expect("deposit");
    env.issue(&aurora, &customer.pubkey(), 1200)
        .expect("and with $3.00 it may print $12.00 of face again, at a 25% reserve");
    assert_eq!(env.merchant_state(&aurora).points_outstanding, 1200);
}

/// The two ways reinstatement is refused, and they are the two that matter: a merchant that still
/// cannot pay, and a merchant that never stopped being able to.
#[test]
fn reinstatement_is_refused_to_the_insolvent_and_the_undefaulted() {
    let mut env = Env::new();
    let (aurora, belmont, cordoba) = insolvent(&mut env);

    // Aurora is not defaulted yet, however bad it looks.
    let err = env
        .reinstate(&aurora)
        .expect_err("there is nothing to reinstate it from");
    assert_custom_error(err, E_NOT_DEFAULTED);

    // Belmont takes its half. Aurora is now defaulted and still owes Cordoba $6.00 against $1.50.
    env.liquidate(&aurora, &belmont).expect("liquidate");
    assert!(!env.is_solvent(&aurora));

    let err = env
        .reinstate(&aurora)
        .expect_err("it still cannot pay Cordoba");
    assert_custom_error(err, E_STILL_INSOLVENT);
    assert_eq!(
        env.merchant_state(&aurora).status,
        MerchantStatus::Defaulted
    );

    // A backer puts in enough to cover Cordoba in full. Now it can pay, so now it may come back —
    // and Cordoba, whose $6.00 claim was stranded behind a merchant that could be neither settled
    // nor liquidated, can turn the crank itself and then be paid in full.
    env.deposit(&aurora, 5 * DOLLAR).expect("deposit");
    assert!(env.is_solvent(&aurora), "$6.50 against $6.00 owed");

    let err = env
        .liquidate(&aurora, &cordoba)
        .expect_err("a merchant that can pay is not liquidated");
    assert_custom_error(err, E_NOT_LIQUIDATABLE);

    let cordoba_authority = cordoba.authority.insecure_clone();
    env.reinstate_as(&cordoba_authority, &aurora)
        .expect("the stranded creditor opens the door itself");
    assert_eq!(env.merchant_state(&aurora).status, MerchantStatus::Active);

    let before = env.token_balance(&cordoba.vault);
    env.settle(&aurora, &cordoba).expect("and then gets paid");
    assert_eq!(env.token_balance(&cordoba.vault) - before, 6 * DOLLAR);
    assert_eq!(env.merchant_state(&aurora).obligations_out, 0);
    assert_eq!(env.merchant_state(&aurora).collateral, 500_000);
}

/// A liquidation is one transfer and a handful of writes. It should be cheap enough that a keeper
/// bot can crank a hundred of them without thinking about the compute budget, and it is.
#[test]
fn a_liquidation_fits_in_the_default_compute_budget() {
    let mut env = Env::new();
    let (aurora, belmont, _cordoba) = insolvent(&mut env);

    let meta = env.liquidate(&aurora, &belmont).expect("liquidate");
    assert!(
        meta.compute_units_consumed < 200_000,
        "a liquidation burned {} CU",
        meta.compute_units_consumed
    );
    println!("liquidate: {} CU", meta.compute_units_consumed);
}
