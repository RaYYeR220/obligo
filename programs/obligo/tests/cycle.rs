//! Multilateral cycle clearing — the reason this protocol exists.
//!
//! Bilateral netting only helps two merchants who happen to owe each other. In a real acceptance
//! network the debt runs in rings: Aurora owes Belmont, Belmont owes Cordoba, Cordoba owes Aurora.
//! Nobody in that ring owes anybody *bilaterally*, so there is nothing to net, and every one of
//! them is carrying collateral against a gross number. Yet the ring as a whole owes nothing to
//! anyone. It is a closed loop of promises, and it can be walked around and cancelled — every edge
//! decremented by the smallest one — without a cent of money existing anywhere.
//!
//! That is what these tests are about, and it is why the verification below is written the way it
//! is. Cycle clearing hands the caller a very sharp knife: it lets an arbitrary stranger reach into
//! sixteen accounts and *reduce debts*. The only thing standing between that and a merchant simply
//! deleting what it owes is the proof that the ring it presented is real. So a forged ring must not
//! be rejected merely by policy. It must be **unrepresentable**: every edge is re-derived from the
//! two merchants it claims to connect, and an edge that does not derive is not an edge.

mod common;

use anchor_lang::prelude::Pubkey;
use common::*;
use obligo::events::CycleCleared;

/// The ring: Aurora owes Belmont $10, Belmont owes Cordoba $7, Cordoba owes Aurora $12.
///
/// Not one of those three pairs owes each other both ways, so bilateral settlement has nothing to
/// bite on. Between them they are carrying $29 of gross liability. $7 of it is a fiction.
fn ring_of_three(env: &mut Env) -> (MerchantHandle, MerchantHandle, MerchantHandle) {
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 20 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 20 * DOLLAR);
    let cordoba = env.issuer("Cordoba Books", 10_000, 3000, 20 * DOLLAR);

    env.owe(&aurora, &belmont, 10 * DOLLAR);
    env.owe(&belmont, &cordoba, 7 * DOLLAR);
    env.owe(&cordoba, &aurora, 12 * DOLLAR);

    (aurora, belmont, cordoba)
}

/// The money shot.
///
/// $7.00 of obligations extinguished. $0.00 of USDC moved. Every vault byte-identical, every
/// merchant healthier, and nobody paid anybody anything.
#[test]
fn a_ring_of_debt_is_extinguished_without_moving_a_cent() {
    let mut env = Env::new();
    let (aurora, belmont, cordoba) = ring_of_three(&mut env);

    assert_eq!(env.owed(&aurora, &belmont), 10 * DOLLAR);
    assert_eq!(env.owed(&belmont, &cordoba), 7 * DOLLAR);
    assert_eq!(env.owed(&cordoba, &aurora), 12 * DOLLAR);

    let vaults = [aurora.vault, belmont.vault, cordoba.vault];
    let before: Vec<Vec<u8>> = vaults.iter().map(|v| env.raw_data(v)).collect();

    let books_before = [
        env.merchant_state(&aurora),
        env.merchant_state(&belmont),
        env.merchant_state(&cordoba),
    ];
    let health_before = [
        env.health_bps(&aurora),
        env.health_bps(&belmont),
        env.health_bps(&cordoba),
    ];

    let meta = env
        .clear_cycle(&[&aurora, &belmont, &cordoba])
        .expect("the ring is real, and the program can prove it");

    // Every edge is down by the smallest edge in the ring, and the smallest edge is gone.
    assert_eq!(env.owed(&aurora, &belmont), 3 * DOLLAR);
    assert_eq!(env.owed(&belmont, &cordoba), 0);
    assert_eq!(env.owed(&cordoba, &aurora), 5 * DOLLAR);

    // Every merchant's gross debt AND gross credit fell by exactly $7 — which is the whole trick.
    // A merchant in a ring is both a debtor and a creditor for the same $7, and it was collateralising
    // one side of that while waiting on the other.
    let books_after = [
        env.merchant_state(&aurora),
        env.merchant_state(&belmont),
        env.merchant_state(&cordoba),
    ];
    for (before, after) in books_before.iter().zip(books_after.iter()) {
        assert_eq!(
            before.obligations_out - after.obligations_out,
            7 * DOLLAR,
            "every member of the ring owes $7.00 less"
        );
        assert_eq!(
            before.obligations_in - after.obligations_in,
            7 * DOLLAR,
            "and is owed $7.00 less"
        );
        assert_eq!(
            before.collateral, after.collateral,
            "and paid nothing for it"
        );
    }

    // The money. There is none. Not "the balances agree" — the vault accounts are byte-for-byte
    // the accounts they were before the instruction ran.
    for (vault, before) in vaults.iter().zip(before.iter()) {
        assert_eq!(&env.raw_data(vault), before, "a vault was touched");
    }

    // And everyone is healthier, because the collateral each of them was posting against that $7
    // was collateral against a debt that never really existed.
    assert!(env.health_bps(&aurora) > health_before[0]);
    assert!(env.health_bps(&belmont) > health_before[1]);
    assert!(env.health_bps(&cordoba) > health_before[2]);

    // $20 against $3 owed, $20 against nothing at all, $20 against $5 owed.
    assert_eq!(env.health_bps(&aurora), 66_666);
    assert_eq!(env.health_bps(&belmont), u64::MAX);
    assert_eq!(env.health_bps(&cordoba), 40_000);

    let event = decode_event::<CycleCleared>(&meta);
    assert_eq!(event.cycle_len, 3);
    assert_eq!(event.amount_cleared, 7 * DOLLAR);
    assert_eq!(event.usdc_moved, 0);
}

// ---- the ring has to be a real one ------------------------------------------------------------

/// The attack. Aurora presents a ring that is a lie: it names Belmont and Cordoba, but for the
/// edge that is supposed to close the loop it hands over a genuine, program-owned obligation
/// account belonging to an entirely different pair — Cordoba's real debt to Dalston.
///
/// If the program merely trusted the accounts it was handed, this would clear $7 of Aurora's debt
/// against a stranger's. The re-derivation is what makes it impossible: the account at index 2 has
/// to be the PDA of `[obligation, Cordoba, Aurora]`, and Cordoba's edge to Dalston is not, cannot
/// be, and can never be made to be.
#[test]
fn a_forged_edge_cannot_be_passed_off_as_part_of_the_ring() {
    let mut env = Env::new();
    let (aurora, belmont, cordoba) = ring_of_three(&mut env);
    let dalston = env.issuer("Dalston Deli", 10_000, 3000, 20 * DOLLAR);
    env.owe(&cordoba, &dalston, 9 * DOLLAR);

    let merchants = [aurora.merchant, belmont.merchant, cordoba.merchant];
    let edges = [
        obligation_address(&aurora.merchant, &belmont.merchant),
        obligation_address(&belmont.merchant, &cordoba.merchant),
        // A real edge. A real PDA. The wrong pair.
        obligation_address(&cordoba.merchant, &dalston.merchant),
    ];

    let err = env
        .clear_cycle_raw(3, &merchants, &edges, None)
        .expect_err("that edge does not close this ring");
    assert_custom_error(err, E_INVALID_CYCLE);

    // Nothing moved anywhere.
    assert_eq!(env.owed(&aurora, &belmont), 10 * DOLLAR);
    assert_eq!(env.owed(&belmont, &cordoba), 7 * DOLLAR);
    assert_eq!(env.owed(&cordoba, &aurora), 12 * DOLLAR);
    assert_eq!(env.owed(&cordoba, &dalston), 9 * DOLLAR);
}

/// An edge pointing the other way is still a real, program-owned, correctly-derived obligation
/// account — it is simply not the one the ring needs. The stored `debtor` and `creditor` are
/// checked against the merchants either side of it, so an edge cannot be walked backwards.
#[test]
fn an_edge_cannot_be_traversed_against_its_direction() {
    let mut env = Env::new();
    let (aurora, belmont, cordoba) = ring_of_three(&mut env);
    // Give the reverse edge a real existence, so the account genuinely resolves.
    env.owe(&belmont, &aurora, 4 * DOLLAR);

    let merchants = [aurora.merchant, belmont.merchant, cordoba.merchant];
    let edges = [
        // Aurora -> Belmont is what a ring starting at Aurora requires. This is Belmont -> Aurora.
        obligation_address(&belmont.merchant, &aurora.merchant),
        obligation_address(&belmont.merchant, &cordoba.merchant),
        obligation_address(&cordoba.merchant, &aurora.merchant),
    ];

    let err = env
        .clear_cycle_raw(3, &merchants, &edges, None)
        .expect_err("debt has a direction");
    assert_custom_error(err, E_INVALID_CYCLE);

    assert_eq!(env.owed(&belmont, &aurora), 4 * DOLLAR);
}

/// The cheapest forgery available: name the same merchant twice, so that some of the "ring" is a
/// merchant clearing debt against itself. Merchants in a cycle must be distinct, and the check is
/// the first thing that runs.
#[test]
fn a_ring_that_visits_a_merchant_twice_is_not_a_ring() {
    let mut env = Env::new();
    let (aurora, belmont, _cordoba) = ring_of_three(&mut env);
    env.owe(&belmont, &aurora, 4 * DOLLAR);

    // A -> B -> A -> ... : three slots, two merchants.
    let merchants = [aurora.merchant, belmont.merchant, aurora.merchant];
    let edges = [
        obligation_address(&aurora.merchant, &belmont.merchant),
        obligation_address(&belmont.merchant, &aurora.merchant),
        obligation_address(&aurora.merchant, &aurora.merchant),
    ];

    let err = env
        .clear_cycle_raw(3, &merchants, &edges, None)
        .expect_err("a merchant cannot appear twice in its own cycle");
    assert_custom_error(err, E_INVALID_CYCLE);

    assert_eq!(env.owed(&aurora, &belmont), 10 * DOLLAR);
    assert_eq!(env.owed(&belmont, &aurora), 4 * DOLLAR);
}

/// A cycle whose smallest edge is zero clears zero. It is a no-op that would still write to
/// sixteen accounts, and there is no reason to let anyone pay for that with somebody else's compute
/// budget. Clearing the ring a second time is precisely this case: the first pass took the minimum
/// edge to nothing.
#[test]
fn a_ring_with_a_dead_edge_is_refused() {
    let mut env = Env::new();
    let (aurora, belmont, cordoba) = ring_of_three(&mut env);

    env.clear_cycle(&[&aurora, &belmont, &cordoba])
        .expect("the first pass takes Belmont -> Cordoba to zero");
    assert_eq!(env.owed(&belmont, &cordoba), 0);

    let err = env
        .clear_cycle(&[&aurora, &belmont, &cordoba])
        .expect_err("and a ring with a dead edge is not a ring");
    assert_custom_error(err, E_EMPTY_CYCLE);

    // The survivors of the first pass are untouched by the second.
    assert_eq!(env.owed(&aurora, &belmont), 3 * DOLLAR);
    assert_eq!(env.owed(&cordoba, &aurora), 5 * DOLLAR);
}

/// An account this program does not own cannot be an edge of anything. Handing the instruction a
/// merchant's USDC vault where an obligation belongs gets it exactly as far as the owner check.
#[test]
fn an_account_this_program_does_not_own_cannot_be_an_edge() {
    let mut env = Env::new();
    let (aurora, belmont, cordoba) = ring_of_three(&mut env);

    let merchants = [aurora.merchant, belmont.merchant, cordoba.merchant];
    let edges = [
        obligation_address(&aurora.merchant, &belmont.merchant),
        obligation_address(&belmont.merchant, &cordoba.merchant),
        // A token account. Owned by the SPL Token program, and full of real money.
        cordoba.vault,
    ];

    let err = env
        .clear_cycle_raw(3, &merchants, &edges, None)
        .expect_err("that is a vault");
    assert_custom_error(err, E_INVALID_CYCLE);
}

/// Nor can an account of the wrong *type*, even when this program does own it. An `Obligation`
/// where a `Merchant` belongs is caught by the discriminator before a single field is read — which
/// matters, because both are this program's accounts and a raw byte-offset read of one as the
/// other would land somewhere plausible.
#[test]
fn an_account_of_the_wrong_type_cannot_stand_in_for_a_merchant() {
    let mut env = Env::new();
    let (aurora, belmont, cordoba) = ring_of_three(&mut env);

    let merchants = [
        aurora.merchant,
        belmont.merchant,
        obligation_address(&cordoba.merchant, &aurora.merchant),
    ];
    let edges = [
        obligation_address(&aurora.merchant, &belmont.merchant),
        obligation_address(&belmont.merchant, &cordoba.merchant),
        obligation_address(&cordoba.merchant, &aurora.merchant),
    ];

    let err = env
        .clear_cycle_raw(3, &merchants, &edges, None)
        .expect_err("that is an obligation, not a merchant");
    assert_custom_error(err, E_ACCOUNT_DISCRIMINATOR_MISMATCH);
}

/// A ring that does not close — the last edge simply does not exist — has nothing to clear.
///
/// An edge that was never created is an address the system program owns and nobody has written to,
/// so it fails the same ownership check as a forged one, and for the same reason: it is not one of
/// this program's obligations. The caller found a path, not a cycle.
#[test]
fn a_path_that_does_not_close_is_not_a_cycle() {
    let mut env = Env::new();
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 20 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 20 * DOLLAR);
    let cordoba = env.issuer("Cordoba Books", 10_000, 3000, 20 * DOLLAR);

    env.owe(&aurora, &belmont, 10 * DOLLAR);
    env.owe(&belmont, &cordoba, 7 * DOLLAR);
    // Cordoba owes Aurora nothing. There is no ring, only a path.

    let err = env
        .clear_cycle(&[&aurora, &belmont, &cordoba])
        .expect_err("Cordoba never owed Aurora anything");
    assert_custom_error(err, E_INVALID_CYCLE);

    assert_eq!(env.owed(&aurora, &belmont), 10 * DOLLAR);
    assert_eq!(env.owed(&belmont, &cordoba), 7 * DOLLAR);
}

/// Two merchants owing each other is a job for `settle`, which moves the net in cash. A "cycle" of
/// two would let a caller cancel it in the ledger and skip the money. And past eight the accounts
/// no longer fit a transaction with any room to spare.
#[test]
fn a_cycle_shorter_than_three_or_longer_than_eight_is_refused() {
    let mut env = Env::new();
    let (aurora, belmont, cordoba) = ring_of_three(&mut env);
    env.owe(&belmont, &aurora, 4 * DOLLAR);

    let two_merchants = [aurora.merchant, belmont.merchant];
    let two_edges = [
        obligation_address(&aurora.merchant, &belmont.merchant),
        obligation_address(&belmont.merchant, &aurora.merchant),
    ];
    let err = env
        .clear_cycle_raw(2, &two_merchants, &two_edges, None)
        .expect_err("two is a settlement, not a cycle");
    assert_custom_error(err, E_INVALID_CYCLE);

    let err = env
        .clear_cycle_raw(0, &[], &[], None)
        .expect_err("nor is nothing");
    assert_custom_error(err, E_INVALID_CYCLE);

    // Nine merchants' worth of accounts, honestly derived. Still refused, on length alone.
    let nine: Vec<Pubkey> = (0..9)
        .map(|i| env.issuer(&format!("m{i}"), 10_000, 3000, 0).merchant)
        .collect();
    let nine_edges: Vec<Pubkey> = (0..9)
        .map(|i| obligation_address(&nine[i], &nine[(i + 1) % 9]))
        .collect();
    let err = env
        .clear_cycle_raw(9, &nine, &nine_edges, Some(400_000))
        .expect_err("nine is past the budget");
    assert_custom_error(err, E_INVALID_CYCLE);

    // And a declared length that disagrees with the accounts actually handed over is refused
    // before any of them are read.
    let err = env
        .clear_cycle_raw(
            3,
            &[aurora.merchant, belmont.merchant, cordoba.merchant],
            &[
                obligation_address(&aurora.merchant, &belmont.merchant),
                obligation_address(&belmont.merchant, &cordoba.merchant),
            ],
            None,
        )
        .expect_err("three merchants, two edges");
    assert_custom_error(err, E_INVALID_CYCLE);

    assert_eq!(env.owed(&aurora, &belmont), 10 * DOLLAR);
}

// ---- the budget -------------------------------------------------------------------------------

/// Eight merchants, eight edges, sixteen writable accounts, eight PDA re-derivations. The biggest
/// ring the instruction accepts, and the number this test prints is the one that decides whether
/// the mechanism is real or a toy.
///
/// It comes in around **38,000 CU** — under a fifth of the 200,000 a transaction is handed for
/// free, so `clear_cycle` never raises its own compute budget and this test does not raise one for
/// it. That is not an accident: every edge is verified with `create_program_address` against a bump
/// the account already carries. `find_program_address` would cost 12,136 CU *per edge* — 97,000 for
/// the ring — and would have turned the headline mechanism into something you had to budget for.
#[test]
fn the_largest_cycle_fits_the_compute_budget() {
    let mut env = Env::new();

    let ring: Vec<MerchantHandle> = (0..8)
        .map(|i| env.issuer(&format!("Merchant {i}"), 10_000, 3000, 20 * DOLLAR))
        .collect();

    // m0 owes m1 $1, m1 owes m2 $2, ... m7 owes m0 $8. The smallest edge is $1.
    for i in 0..8 {
        env.owe(&ring[i], &ring[(i + 1) % 8], (i as u64 + 1) * DOLLAR);
    }

    let vaults: Vec<Vec<u8>> = ring.iter().map(|m| env.raw_data(&m.vault)).collect();

    let members: Vec<&MerchantHandle> = ring.iter().collect();
    // Note the `None`: no compute-budget instruction. This has to fit in what a transaction is
    // given for nothing, or the mechanism is not really permissionless.
    let meta = env.clear_cycle(&members).expect("eight-merchant ring");

    println!("clear_cycle(8) consumed {} CU", meta.compute_units_consumed);
    assert!(
        meta.compute_units_consumed < 200_000,
        "an 8-merchant cycle burned {} CU — past the default budget a transaction is given",
        meta.compute_units_consumed
    );
    // A ceiling with room to breathe, so that a regression which quietly doubles the cost — say,
    // an accidental `find_program_address` — turns this test red instead of merely slow.
    assert!(
        meta.compute_units_consumed < 60_000,
        "an 8-merchant cycle should cost about 38,000 CU, not {}",
        meta.compute_units_consumed
    );

    // $8 of gross debt gone from every one of the eight, and eight vaults byte-identical.
    let event = decode_event::<CycleCleared>(&meta);
    assert_eq!(event.cycle_len, 8);
    assert_eq!(event.amount_cleared, DOLLAR);
    assert_eq!(event.usdc_moved, 0);

    for (i, m) in ring.iter().enumerate() {
        let state = env.merchant_state(m);
        assert_eq!(state.obligations_out, i as u64 * DOLLAR);
        assert_eq!(state.collateral, 20 * DOLLAR);
        assert_eq!(&env.raw_data(&m.vault), &vaults[i]);
    }
    assert_eq!(env.owed(&ring[0], &ring[1]), 0);
    assert_eq!(env.owed(&ring[7], &ring[0]), 7 * DOLLAR);
}
