//! Bilateral settlement: two merchants, two directed edges, and one number that actually moves.
//!
//! Redemption is deliberately cheap — it creates debt and moves nothing. Settlement is where the
//! debt is finally paid, and the whole point of it is *how little* has to be paid. Two merchants
//! that owe each other $10 and $8 do not need $18 of liquidity between them. They need $2. The
//! other $16 is cancelled against itself and never touches a vault.
//!
//! Nobody is asked for permission. Neither party signs. It is a crank, and it is a public good:
//! the debtor's health improves, the creditor gets paid, and the caller gets nothing but the
//! satisfaction of having netted two numbers. That is on purpose — a settlement that only the
//! creditor could trigger would be a settlement the creditor could sit on.

mod common;

use common::*;
use obligo::events::Settled;
use obligo::state::MerchantStatus;
use solana_signer::Signer;

/// Two merchants, $20 of collateral each, and a mutual debt: Aurora owes Belmont $10 and Belmont
/// owes Aurora $8. $18 of gross. $2 of net.
fn mutual_debt(env: &mut Env) -> (MerchantHandle, MerchantHandle) {
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 20 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 20 * DOLLAR);

    env.owe(&aurora, &belmont, 10 * DOLLAR);
    env.owe(&belmont, &aurora, 8 * DOLLAR);

    (aurora, belmont)
}

/// The claim: eighteen dollars of debt, two dollars of money.
#[test]
fn eighteen_dollars_of_debt_are_settled_by_moving_two() {
    let mut env = Env::new();
    let (aurora, belmont) = mutual_debt(&mut env);

    assert_eq!(env.owed(&aurora, &belmont), 10 * DOLLAR);
    assert_eq!(env.owed(&belmont, &aurora), 8 * DOLLAR);

    let aurora_vault = env.token_balance(&aurora.vault);
    let belmont_vault = env.token_balance(&belmont.vault);
    assert_eq!(aurora_vault, 20 * DOLLAR);
    assert_eq!(belmont_vault, 20 * DOLLAR);

    let meta = env.settle(&aurora, &belmont).expect("settle");

    // Exactly $2 of USDC crossed, and it crossed in the right direction.
    assert_eq!(
        env.token_balance(&aurora.vault) as i128 - aurora_vault as i128,
        -2 * DOLLAR as i128,
        "the net debtor pays the net, and only the net"
    );
    assert_eq!(
        env.token_balance(&belmont.vault) as i128 - belmont_vault as i128,
        2 * DOLLAR as i128
    );

    // The books say the same thing as the vaults.
    let a = env.merchant_state(&aurora);
    let b = env.merchant_state(&belmont);
    assert_eq!(a.collateral, 18 * DOLLAR);
    assert_eq!(b.collateral, 22 * DOLLAR);

    // Aurora's gross obligation fell by the whole $10, not by the $2 it paid: $8 of it was
    // cancelled against what Belmont owed back.
    assert_eq!(a.obligations_out, 0);
    assert_eq!(a.obligations_in, 0);
    assert_eq!(b.obligations_out, 0);
    assert_eq!(b.obligations_in, 0);

    // And both edges are extinguished.
    assert_eq!(env.owed(&aurora, &belmont), 0);
    assert_eq!(env.owed(&belmont, &aurora), 0);

    let event = decode_event::<Settled>(&meta);
    assert_eq!(event.debtor, aurora.merchant);
    assert_eq!(event.creditor, belmont.merchant);
    assert_eq!(event.offset, 8 * DOLLAR, "cancelled without money");
    assert_eq!(event.paid, 2 * DOLLAR, "and this is all the money there was");
    assert_eq!(event.residual, 0);
}

/// Settlement is a crank, and a crank that only the interested parties can turn is not a crank.
/// A stranger who holds no points, has no merchant account and stands to gain nothing calls it,
/// and the two merchants' books are settled anyway.
#[test]
fn settlement_is_a_crank_any_stranger_may_turn() {
    let mut env = Env::new();
    let (aurora, belmont) = mutual_debt(&mut env);

    let stranger = env.stranger();
    assert_ne!(stranger.pubkey(), aurora.authority.pubkey());
    assert_ne!(stranger.pubkey(), belmont.authority.pubkey());

    env.settle_as(&stranger, &aurora, &belmont)
        .expect("a settlement needs nobody's permission");

    assert_eq!(env.token_balance(&aurora.vault), 18 * DOLLAR);
    assert_eq!(env.token_balance(&belmont.vault), 22 * DOLLAR);
    assert_eq!(env.merchant_state(&aurora).obligations_out, 0);
    assert_eq!(env.merchant_state(&belmont).obligations_in, 0);
}

/// Which of the two is the debtor is a fact about the edges, not about the order of the arguments.
/// Calling it the other way round settles the identical thing, and the money still runs from
/// Aurora to Belmont.
#[test]
fn the_direction_of_payment_is_read_from_the_graph_not_from_the_caller() {
    let mut env = Env::new();
    let (aurora, belmont) = mutual_debt(&mut env);

    env.settle(&belmont, &aurora)
        .expect("named the other way round");

    assert_eq!(env.token_balance(&aurora.vault), 18 * DOLLAR);
    assert_eq!(env.token_balance(&belmont.vault), 22 * DOLLAR);
}

/// The reverse edge does not have to exist. Two merchants that have only ever traded one way have
/// no `B -> A` account at all, and there is nothing to net: the debtor simply pays.
///
/// It has to be *present in the transaction* all the same, and that is not a formality. If a
/// settlement could be told "there is no reverse edge" and take the caller's word for it, anyone
/// could hide a live counter-claim and force the debtor to pay gross — draining collateral that
/// other creditors have a claim on, and pushing a solvent merchant under. So the account is
/// created, at the caller's expense, at the one address the seeds allow.
#[test]
fn a_one_way_debt_settles_against_a_reverse_edge_that_never_existed() {
    let mut env = Env::new();
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 20 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 20 * DOLLAR);

    env.owe(&aurora, &belmont, 10 * DOLLAR);
    assert!(
        !env.account_exists(&obligation_address(&belmont.merchant, &aurora.merchant)),
        "Belmont has never owed Aurora a cent"
    );

    env.settle(&aurora, &belmont).expect("settle");

    assert_eq!(env.token_balance(&aurora.vault), 10 * DOLLAR);
    assert_eq!(env.token_balance(&belmont.vault), 30 * DOLLAR);
    assert_eq!(env.merchant_state(&aurora).obligations_out, 0);
    assert_eq!(env.merchant_state(&belmont).obligations_in, 0);

    // The empty edge now exists, pinned to the only address its seeds permit, and says zero.
    let reverse = env.obligation_state(&belmont, &aurora);
    assert_eq!(reverse.debtor, belmont.merchant);
    assert_eq!(reverse.creditor, aurora.merchant);
    assert_eq!(reverse.amount, 0);
}

/// `paid = min(net, collateral)`. A debtor that cannot cover the net pays every cent it has, and
/// the rest of the debt stays exactly where it was — on the edge, with the creditor's name on it.
/// It is not written off, and the debtor is not let off: it is now visibly insolvent, and anyone
/// may liquidate it.
#[test]
fn a_debtor_that_cannot_cover_the_net_pays_what_it_has_and_stays_on_the_hook() {
    let mut env = Env::new();
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 20 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 20 * DOLLAR);

    // Aurora issued $30 of points against $20 of collateral — a 30% reserve permitted exactly
    // that — and every one of them came home.
    env.owe(&aurora, &belmont, 30 * DOLLAR);
    env.owe(&belmont, &aurora, 8 * DOLLAR);

    env.settle(&aurora, &belmont).expect("settle");

    // $8 cancelled, $20 paid, $2 left owing.
    assert_eq!(env.token_balance(&aurora.vault), 0);
    assert_eq!(env.token_balance(&belmont.vault), 40 * DOLLAR);

    let a = env.merchant_state(&aurora);
    assert_eq!(a.collateral, 0);
    assert_eq!(a.obligations_out, 2 * DOLLAR, "the residual stays on the edge");
    assert_eq!(a.obligations_in, 0);
    assert_eq!(env.owed(&aurora, &belmont), 2 * DOLLAR);
    assert_eq!(env.owed(&belmont, &aurora), 0);

    let b = env.merchant_state(&belmont);
    assert_eq!(b.obligations_in, 2 * DOLLAR);
    assert_eq!(b.obligations_out, 0);

    // Nothing was forgiven and nothing was invented: Aurora is simply, publicly, short.
    assert!(
        !obligo::math::is_solvent(a.collateral, a.obligations_out),
        "$0.00 against $2.00 owed"
    );
    assert_eq!(env.health_bps(&aurora), 0);
}

/// Settling the same pair twice is not an error the second time because the first one was wrong;
/// it is an error because there is nothing there. A crank that could be re-run for a fee on an
/// empty pair is a crank someone will re-run for a fee on an empty pair.
#[test]
fn settling_a_pair_that_owes_nothing_is_refused() {
    let mut env = Env::new();
    let (aurora, belmont) = mutual_debt(&mut env);

    env.settle(&aurora, &belmont).expect("first");

    let err = env
        .settle(&aurora, &belmont)
        .expect_err("and there is nothing left to do");
    assert_custom_error(err, E_NOTHING_TO_SETTLE);

    assert_eq!(env.token_balance(&aurora.vault), 18 * DOLLAR);
    assert_eq!(env.token_balance(&belmont.vault), 22 * DOLLAR);
}

/// A defaulted merchant's collateral is an estate. It belongs to *all* of its creditors, in
/// proportion, and `liquidate` is the instruction that says so. If settlement could still run, the
/// first creditor to crank it would take the whole vault up to its own claim and everyone behind
/// it would find the cupboard bare. So the debtor's default closes this door.
///
/// Note which merchant the guard is read from: the one the graph says is paying. Belmont, the
/// creditor, may be in whatever state it likes — money arriving in a defaulted merchant's vault
/// only helps the people it owes.
#[test]
fn a_defaulted_debtor_cannot_be_settled_ahead_of_its_other_creditors() {
    let mut env = Env::new();
    let (aurora, belmont) = mutual_debt(&mut env);

    env.set_status(&aurora, MerchantStatus::Defaulted);

    let err = env
        .settle(&aurora, &belmont)
        .expect_err("Aurora's estate is not first-come-first-served");
    assert_custom_error(err, E_MERCHANT_DEFAULTED);

    assert_eq!(env.token_balance(&aurora.vault), 20 * DOLLAR);
    assert_eq!(env.token_balance(&belmont.vault), 20 * DOLLAR);
    assert_eq!(env.owed(&aurora, &belmont), 10 * DOLLAR);

    // Flip it: Belmont, the *creditor*, defaults. Aurora still owes it, and still pays.
    env.set_status(&aurora, MerchantStatus::Active);
    env.set_status(&belmont, MerchantStatus::Defaulted);

    env.settle(&aurora, &belmont)
        .expect("paying a defaulted creditor is paying its creditors");
    assert_eq!(env.token_balance(&belmont.vault), 22 * DOLLAR);
}

/// Settlement pays down real debt, so it raises the debtor's health — and it never touches the
/// creditor's, because being owed less money is not a solvency event.
#[test]
fn settlement_leaves_the_debtor_healthier_than_it_found_it() {
    let mut env = Env::new();
    let (aurora, belmont) = mutual_debt(&mut env);

    // $20 of collateral against $10 owed.
    let before = env.health_bps(&aurora);
    assert_eq!(before, 20_000);

    env.settle(&aurora, &belmont).expect("settle");

    // $18 of collateral against nothing owed at all.
    assert_eq!(env.health_bps(&aurora), u64::MAX);
    assert_eq!(env.health_bps(&belmont), u64::MAX);
}
