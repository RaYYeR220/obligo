//! The properties. Not "does this instruction do what I meant" — the other question.
//!
//! Every test above this one drives a scenario somebody thought of. These drive a *seeded random
//! sequence* of every instruction the protocol has, against four merchants with deliberately
//! different terms, and check — after every single step, whether it succeeded or failed — that a
//! handful of statements about the whole system are still true. They are the statements a protocol
//! is allowed to make, and they are the ones that break quietly:
//!
//! - a merchant can never be left holding fewer dollars than the points it has printed require;
//! - no instruction mints or destroys a cent of USDC;
//! - clearing a ring of debt moves no money at all — not "almost none", none;
//! - what everybody owes and what everybody is owed are the same number, always;
//! - and `points_outstanding` is not a number this program keeps in a drawer. It reconciles,
//!   exactly, against what Token-2022 itself believes the supply to be. **That is the test that
//!   proves the hook is doing its job**: if a single point could escape the protocol — moved
//!   between two wallets, sold, gifted, anything — the two numbers would drift apart and this would
//!   go red.
//!
//! The sequence is seeded and deterministic. No wall clock, no `rand`, no dependency: the same seed
//! produces the same several hundred instructions on every machine, forever, so a failure here is a
//! failure you can reproduce rather than a rumour. And `every_instruction_is_actually_exercised`
//! exists because a property suite that never runs the interesting instruction is a property suite
//! that passes for the wrong reason.

mod common;

use common::*;
use obligo::state::MerchantStatus;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use std::collections::BTreeMap;

// ---- the generator ------------------------------------------------------------------------

/// xorshift64*. Twenty lines to avoid a dependency, and it buys determinism: same seed, same walk.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // Zero is a fixed point of xorshift, and a silently degenerate generator would make every
        // test in this file pass by doing the same thing two hundred times.
        assert_ne!(seed, 0);
        Rng(seed)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform enough in `[0, n)` for a state-machine walk.
    fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0);
        self.next_u64() % n
    }
}

// ---- the world ----------------------------------------------------------------------------

const MERCHANTS: usize = 4;
const CUSTOMERS: usize = 5;

struct World {
    merchants: Vec<MerchantHandle>,
    customers: Vec<Keypair>,
    /// Issued to only by the solvency probe, so a probe that unexpectedly succeeds cannot quietly
    /// corrupt a batch the walk is using.
    probe_customer: Keypair,
    /// Every USDC account in existence: the four vaults and the four merchants' own wallets.
    /// Nothing else has ever held a cent of it.
    usdc_accounts: Vec<Pubkey>,
    usdc_total: u64,
    usdc_supply: u64,
}

impl World {
    fn new(env: &mut Env) -> Self {
        // Four merchants, deliberately unalike: different face values, different reserves,
        // different times to live. A protocol whose invariants hold only when every merchant is
        // configured identically has not got invariants, it has got a coincidence.
        //
        // Cordoba's 10% reserve against a thin vault is there on purpose. It is the one that goes
        // under, and something has to, or `liquidate` never runs and half of this file is theatre.
        let specs: [(&str, u64, u16, i64, u64); MERCHANTS] = [
            ("Cafe Aurora", 10_000, 3000, 7_200, 20 * DOLLAR),
            ("Bodega Belmont", 5_000, 2500, 7_200, 15 * DOLLAR),
            ("Cordoba Books", 10_000, 1000, 3_600, 6 * DOLLAR),
            ("Dorset Deli", 2_500, 5000, 10_800, 12 * DOLLAR),
        ];

        let mut merchants = Vec::with_capacity(MERCHANTS);
        for (name, usdc_per_point, reserve_bps, point_ttl, collateral) in specs {
            let m = env.register_merchant(name, usdc_per_point, reserve_bps, point_ttl);
            env.create_points_mint(&m, name, "PTS", "https://example.invalid/points.json")
                .expect("create_points_mint");
            env.deposit(&m, collateral).expect("deposit");
            merchants.push(m);
        }

        // Everybody bids for everybody else's points, at rates running from a discount on the
        // issuer's credit to a bid for its footfall.
        let expires_at = env.now() + 3_650 * 86_400;
        let rates = [9_000u16, 10_000, 11_000, 12_500];
        for i in 0..MERCHANTS {
            for j in 0..MERCHANTS {
                if i == j {
                    continue;
                }
                env.post_offer(
                    &merchants[j],
                    &merchants[i],
                    rates[(i + j) % rates.len()],
                    5_000 * DOLLAR,
                    expires_at,
                )
                .expect("post_offer");
            }
        }

        let customers: Vec<Keypair> = (0..CUSTOMERS).map(|_| Keypair::new()).collect();

        let mut usdc_accounts: Vec<Pubkey> = Vec::new();
        for m in &merchants {
            usdc_accounts.push(m.vault);
            usdc_accounts.push(m.usdc);
        }

        let usdc_total = usdc_accounts.iter().map(|a| env.token_balance(a)).sum();
        let usdc_supply = env.usdc_supply();
        assert_eq!(
            usdc_total, usdc_supply,
            "the harness has minted USDC into an account this suite cannot see"
        );

        World {
            merchants,
            customers,
            probe_customer: Keypair::new(),
            usdc_accounts,
            usdc_total,
            usdc_supply,
        }
    }

    /// Every wallet that could conceivably hold a point.
    fn everybody(&self) -> impl Iterator<Item = &Keypair> + '_ {
        self.customers
            .iter()
            .chain(std::iter::once(&self.probe_customer))
    }
}

/// A customer's balance of one merchant's points, reading a wallet that was never opened as the
/// zero it means.
fn points_held(env: &Env, m: &MerchantHandle, customer: &Pubkey) -> u64 {
    let ata = env.points_account(m, customer);
    if env.account_exists(&ata) {
        env.token_balance(&ata)
    } else {
        0
    }
}

// ---- the walk -----------------------------------------------------------------------------

/// What the walk just did, and what the world looked like a moment before it did it.
struct Step {
    index: usize,
    op: &'static str,
    ok: bool,
    /// Every vault's raw account data, byte for byte, before the instruction ran. A balance read
    /// through a getter is not quite the claim `cycle_clearing_moves_no_usdc` makes.
    vaults_before: Vec<Vec<u8>>,
}

/// Drive `steps` random instructions and hand the caller the world after every one of them.
///
/// Most steps are *expected* to fail — a redemption of points a customer does not hold, a
/// settlement of a pair that owes each other nothing, a liquidation of a merchant that is perfectly
/// solvent. That is deliberate. The properties below must hold after a refused instruction exactly
/// as they hold after an accepted one, because "and then nothing happened" is a claim the protocol
/// has to be able to keep too.
fn random_walk(
    seed: u64,
    steps: usize,
    check: &mut dyn FnMut(&mut Env, &World, &Step),
) -> BTreeMap<&'static str, u32> {
    let mut env = Env::new();
    let world = World::new(&mut env);
    let mut rng = Rng::new(seed);
    let mut tally: BTreeMap<&'static str, u32> = BTreeMap::new();

    let genesis = Step {
        index: 0,
        op: "genesis",
        ok: true,
        vaults_before: vaults(&env, &world),
    };
    check(&mut env, &world, &genesis);

    for index in 1..=steps {
        let vaults_before = vaults(&env, &world);
        let (op, ok) = step(&mut env, &world, &mut rng);
        if ok {
            *tally.entry(op).or_default() += 1;
        }
        check(
            &mut env,
            &world,
            &Step {
                index,
                op,
                ok,
                vaults_before,
            },
        );
    }

    tally
}

fn vaults(env: &Env, world: &World) -> Vec<Vec<u8>> {
    world
        .merchants
        .iter()
        .map(|m| env.raw_data(&m.vault))
        .collect()
}

/// How many times the walk drove `op` all the way to a confirmed transaction. Zero is a valid
/// answer, and — for an instruction a property leans on — a failing one.
fn ran(tally: &BTreeMap<&'static str, u32>, op: &str) -> u32 {
    tally.get(op).copied().unwrap_or(0)
}

/// One random instruction. Returns what it was and whether the chain took it.
fn step(env: &mut Env, w: &World, rng: &mut Rng) -> (&'static str, bool) {
    /// Two different merchants.
    fn pair(rng: &mut Rng) -> (usize, usize) {
        let i = rng.below(MERCHANTS as u64) as usize;
        let j = (i + 1 + rng.below(MERCHANTS as u64 - 1) as usize) % MERCHANTS;
        (i, j)
    }

    match rng.below(100) {
        0..=21 => {
            // Up to 1500 points at a swing, which for the merchants above is anywhere from $3.75 to
            // $15.00 of face value. Most of the large ones will be refused — the reserve invariant
            // says so — and the ones that squeak through leave the merchant sitting right on its
            // own limit, which is the interesting place for it to be sitting when a customer walks
            // in and calls the promise in.
            let m = &w.merchants[rng.below(MERCHANTS as u64) as usize];
            let c = &w.customers[rng.below(CUSTOMERS as u64) as usize];
            let points = 1 + rng.below(1_500);
            ("issue", env.issue(m, &c.pubkey(), points).is_ok())
        }

        22..=47 => {
            let (i, j) = pair(rng);
            let issuer = &w.merchants[i];
            let acceptor = &w.merchants[j];
            let c = &w.customers[rng.below(CUSTOMERS as u64) as usize];

            let held = points_held(env, issuer, &c.pubkey());
            if held == 0 {
                // Nothing to spend. Still a step, and the properties still have to hold.
                return ("redeem", false);
            }
            let points = 1 + rng.below(held);
            ("redeem", env.redeem(issuer, acceptor, c, points).is_ok())
        }

        48..=55 => {
            let (i, j) = pair(rng);
            (
                "settle",
                env.settle(&w.merchants[i], &w.merchants[j]).is_ok(),
            )
        }

        56..=63 => match find_ring(env, w) {
            Some(ring) => {
                let members: Vec<&MerchantHandle> = ring.iter().map(|i| &w.merchants[*i]).collect();
                ("clear_cycle", env.clear_cycle(&members).is_ok())
            }
            None => ("clear_cycle", false),
        },

        64..=71 => {
            // Guess first: an expiry aimed at a customer who is still shopping has to be refused,
            // and driving that refusal is worth a transaction. Then look properly, the way a keeper
            // watching the clock would.
            let m = &w.merchants[rng.below(MERCHANTS as u64) as usize];
            let c = &w.customers[rng.below(CUSTOMERS as u64) as usize];
            if env.expire(m, &c.pubkey()).is_ok() {
                return ("expire", true);
            }
            match find_lapsed(env, w) {
                Some((m, c)) => (
                    "expire",
                    env.expire(&w.merchants[m], &w.customers[c].pubkey())
                        .is_ok(),
                ),
                None => ("expire", false),
            }
        }

        72..=79 => {
            // Same shape. A liquidation aimed at a solvent merchant must be refused; a liquidator
            // does not guess, it scans for insolvency. Both happen here.
            let (i, j) = pair(rng);
            if env.liquidate(&w.merchants[i], &w.merchants[j]).is_ok() {
                return ("liquidate", true);
            }
            match find_insolvent(env, w) {
                Some((d, c)) => (
                    "liquidate",
                    env.liquidate(&w.merchants[d], &w.merchants[c]).is_ok(),
                ),
                None => ("liquidate", false),
            }
        }

        80..=83 => {
            let m = &w.merchants[rng.below(MERCHANTS as u64) as usize];
            if env.reinstate(m).is_ok() {
                return ("reinstate", true);
            }
            match find_reinstatable(env, w) {
                Some(i) => ("reinstate", env.reinstate(&w.merchants[i]).is_ok()),
                None => ("reinstate", false),
            }
        }

        84..=87 => {
            // Small. A backer with a bottomless wallet would keep every merchant solvent forever,
            // and a suite in which nobody ever goes under is a suite that never liquidates anybody.
            let m = &w.merchants[rng.below(MERCHANTS as u64) as usize];
            let amount = 1 + rng.below(2 * DOLLAR);
            ("deposit", env.deposit(m, amount).is_ok())
        }

        88..=91 => {
            let m = &w.merchants[rng.below(MERCHANTS as u64) as usize];
            let amount = 1 + rng.below(4 * DOLLAR);
            ("withdraw", env.withdraw(m, amount).is_ok())
        }

        92..=95 => {
            let (i, j) = pair(rng);
            let rate = 8_000 + rng.below(6_000) as u16;
            let capacity = DOLLAR + rng.below(200 * DOLLAR);
            let expires_at = env.now() + 3_650 * 86_400;
            (
                "post_offer",
                env.post_offer(&w.merchants[j], &w.merchants[i], rate, capacity, expires_at)
                    .is_ok(),
            )
        }

        _ => {
            // The clock: the only thing here that no instruction controls, and the one expiry lives
            // or dies by. The merchants' TTLs run from one hour to three, so an hour a step is
            // enough to strand a customer who stops shopping without stranding one who does not.
            env.warp(1 + rng.below(2_400) as i64);
            ("warp", true)
        }
    }
}

// ---- the off-chain half -------------------------------------------------------------------
//
// Everything below is a *client*, not a protocol. Cycle clearing, liquidation and expiry are
// permissionless cranks: the chain's job is to prove that what it has been handed is real, and it
// is somebody else's job to go and find it. These four functions are that somebody — a keeper
// watching the graph, the solvency of every issuer, and the clock. They are deterministic, so the
// walk stays reproducible.

/// A ring of three merchants where every edge is live. The first, in index order.
fn find_ring(env: &Env, w: &World) -> Option<Vec<usize>> {
    let owes = |a: usize, b: usize| env.owed(&w.merchants[a], &w.merchants[b]) > 0;

    for a in 0..MERCHANTS {
        for b in 0..MERCHANTS {
            for c in 0..MERCHANTS {
                if a == b || b == c || c == a {
                    continue;
                }
                if owes(a, b) && owes(b, c) && owes(c, a) {
                    return Some(vec![a, b, c]);
                }
            }
        }
    }
    None
}

/// A merchant that owes more than it holds, and a creditor with a live claim on it. This is the
/// whole of a liquidation bot.
fn find_insolvent(env: &Env, w: &World) -> Option<(usize, usize)> {
    for d in 0..MERCHANTS {
        if env.is_solvent(&w.merchants[d]) {
            continue;
        }
        for c in 0..MERCHANTS {
            if c != d && env.owed(&w.merchants[d], &w.merchants[c]) > 0 {
                return Some((d, c));
            }
        }
    }
    None
}

/// A merchant that is defaulted and can pay its way again — the state a stranded creditor has to
/// crank it out of before it can settle with it at all.
fn find_reinstatable(env: &Env, w: &World) -> Option<usize> {
    (0..MERCHANTS).find(|i| {
        env.merchant_state(&w.merchants[*i]).status == MerchantStatus::Defaulted
            && env.is_solvent(&w.merchants[*i])
    })
}

/// A customer who has stopped coming in, holding points past the deadline the merchant published.
fn find_lapsed(env: &Env, w: &World) -> Option<(usize, usize)> {
    let now = env.now();
    for m in 0..MERCHANTS {
        let ttl = env.merchant_state(&w.merchants[m]).point_ttl;
        for c in 0..CUSTOMERS {
            let customer = w.customers[c].pubkey();
            if points_held(env, &w.merchants[m], &customer) == 0 {
                continue;
            }
            let batch = env.batch_state(&w.merchants[m], &customer);
            if batch.amount > 0 && now >= batch.issued_at + ttl {
                return Some((m, c));
            }
        }
    }
    None
}

// ---- the properties -----------------------------------------------------------------------

/// **No instruction can leave a merchant over-issued.**
///
/// The claim needs stating precisely, because the obvious version of it is false and *should* be. A
/// merchant's health can and must fall below 1.0: a redemption converts a fractional reserve into a
/// debt at full face, and it is meant to hurt. Paying that debt in `settle` drains the vault
/// further. Neither is a bug; both are the product.
///
/// What may never happen is a merchant **printing a point, or withdrawing a dollar, that leaves it
/// unable to back what is still outstanding**. So the property has two halves, and after every step
/// of the walk, for every merchant, one of them applies:
///
/// - the merchant is fully covered — `collateral >= required_collateral(...)` — and may do as it
///   likes; or
/// - it is short, and then it is **provably unable to make itself any shorter**: a probe issuance
///   of a single point is refused, and a probe withdrawal of a single micro-dollar is refused. The
///   only ways it can have got here are the ones where somebody called in a promise it had already
///   made, or it paid a debt it already owed.
///
/// The probes are real transactions against the real program. They fire only when the merchant is
/// short, and they are expected to fail — so they change nothing, and a probe that *succeeded* would
/// be precisely the bug this test exists to find.
#[test]
fn no_instruction_can_leave_a_merchant_over_issued() {
    let tally = random_walk(0x0B11_6017_ACE1_2026, 300, &mut |env, w, step| {
        for m in &w.merchants {
            let state = env.merchant_state(m);
            let required = env.required_collateral(m);

            if state.collateral >= required {
                continue;
            }

            // Short. Prove it cannot print its way further into the hole.
            let err = env
                .issue(m, &w.probe_customer.pubkey(), 1)
                .expect_err(&format!(
                    "step {} ({}): {} holds {} against {} required and could still print a point",
                    step.index, step.op, state.name, state.collateral, required
                ));
            assert!(
                is_custom(&err, E_RESERVE_BREACHED) || is_custom(&err, E_MERCHANT_DEFAULTED),
                "step {} ({}): a short merchant's issuance was refused for the wrong reason: {err:?}",
                step.index,
                step.op
            );

            // And it cannot walk out with the reserve, either.
            let err = env.withdraw(m, 1).expect_err(&format!(
                "step {} ({}): {} holds {} against {} required and could still withdraw",
                step.index, step.op, state.name, state.collateral, required
            ));
            assert!(
                is_custom(&err, E_RESERVE_BREACHED) || is_custom(&err, E_INSUFFICIENT_COLLATERAL),
                "step {} ({}): a short merchant's withdrawal was refused for the wrong reason: {err:?}",
                step.index,
                step.op
            );
        }
    });

    // Worth nothing unless the walk actually printed points and called them in.
    assert!(
        ran(&tally, "issue") > 0 && ran(&tally, "redeem") > 0,
        "{tally:?}"
    );
}

/// **USDC is conserved.** No instruction in this protocol mints a dollar or destroys one.
///
/// Every cent that exists lives in one of eight accounts — four collateral vaults and four
/// merchants' own wallets — and the sum of them is fixed at genesis. `deposit` and `withdraw` move
/// money between a merchant and its own vault; `settle` and `liquidate` move it between two vaults;
/// `issue`, `redeem`, `clear_cycle` and `expire_points` move none at all. Nothing else touches USDC,
/// and there is no path in this program that could: a vault is a PDA whose only signer is this
/// program, and this program has never held the USDC mint's authority.
///
/// Checked two ways, because they fail differently. The **sum across every account** catches a
/// transfer that lost a cent to rounding, or paid it somewhere nobody is looking. The **mint's own
/// supply** catches the thing that would be much worse.
///
/// And the books have to agree with the money: every merchant's `collateral` field is asserted equal
/// to what its vault actually holds. A protocol whose ledger and vault can disagree is a protocol
/// that will one day pay a creditor out of a number.
#[test]
fn usdc_is_conserved() {
    let tally = random_walk(0x0057_7C1E_D9A2_4411, 300, &mut |env, w, step| {
        let total: u64 = w.usdc_accounts.iter().map(|a| env.token_balance(a)).sum();

        assert_eq!(
            total, w.usdc_total,
            "step {} ({}): {} USDC in the world, {} at genesis",
            step.index, step.op, total, w.usdc_total
        );
        assert_eq!(
            env.usdc_supply(),
            w.usdc_supply,
            "step {} ({}): the USDC mint's own supply moved",
            step.index,
            step.op
        );

        for m in &w.merchants {
            let state = env.merchant_state(m);
            assert_eq!(
                state.collateral,
                env.token_balance(&m.vault),
                "step {} ({}): {}'s books and its vault disagree",
                step.index,
                step.op,
                state.name
            );
        }
    });

    // Vacuous unless the walk actually moved money, in both of the two ways it can be moved.
    assert!(ran(&tally, "settle") > 0, "{tally:?}");
    assert!(ran(&tally, "liquidate") > 0, "{tally:?}");
    assert!(
        ran(&tally, "deposit") > 0 && ran(&tally, "withdraw") > 0,
        "{tally:?}"
    );
}

/// **Cycle clearing moves no USDC.** Not "almost none". None.
///
/// This is the claim the whole protocol is built to be able to make, so it is checked as literally
/// as it can be: every vault's raw account data, byte for byte, before and after. Not the balance
/// read through a getter — the bytes. If a single lamport of rent, a single unit of a delegated
/// amount, a single flag anywhere in the account had moved, this would go red.
///
/// A ring is built by hand first, because a property that only ever runs against a graph that
/// happened to contain a cycle is a property that will one day quietly stop running. Then the random
/// walk clears whatever rings it finds, and every one of those is held to the same standard.
#[test]
fn cycle_clearing_moves_no_usdc() {
    // Built on purpose: Aurora owes Belmont $10, Belmont owes Cordoba $7, Cordoba owes Aurora $12.
    // No pair owes each other both ways, so `settle` has nothing to bite on, and all three are
    // posting collateral against a debt that partly does not exist.
    let mut env = Env::new();
    let aurora = env.issuer("Cafe Aurora", 10_000, 3000, 25 * DOLLAR);
    let belmont = env.issuer("Bodega Belmont", 10_000, 3000, 25 * DOLLAR);
    let cordoba = env.issuer("Cordoba Books", 10_000, 3000, 25 * DOLLAR);

    env.owe(&aurora, &belmont, 10 * DOLLAR);
    env.owe(&belmont, &cordoba, 7 * DOLLAR);
    env.owe(&cordoba, &aurora, 12 * DOLLAR);

    let vaults = [aurora.vault, belmont.vault, cordoba.vault];
    let before: Vec<Vec<u8>> = vaults.iter().map(|v| env.raw_data(v)).collect();

    env.clear_cycle(&[&aurora, &belmont, &cordoba])
        .expect("clear_cycle");

    for (i, vault) in vaults.iter().enumerate() {
        assert_eq!(
            env.raw_data(vault),
            before[i],
            "a vault changed by a byte while a ring of debt was cancelled"
        );
    }
    // $7.00 of obligations extinguished, $0.00 of USDC moved.
    assert_eq!(env.owed(&aurora, &belmont), 3 * DOLLAR);
    assert_eq!(env.owed(&belmont, &cordoba), 0);
    assert_eq!(env.owed(&cordoba, &aurora), 5 * DOLLAR);

    // And now the same claim, against whatever rings a randomly grown graph throws up.
    let mut cleared = 0u32;
    let tally = random_walk(0x0C7C_1E5A_11EE_D51D, 300, &mut |env, w, step| {
        if step.op != "clear_cycle" || !step.ok {
            return;
        }
        cleared += 1;
        for (i, m) in w.merchants.iter().enumerate() {
            assert_eq!(
                env.raw_data(&m.vault),
                step.vaults_before[i],
                "step {}: clearing a ring moved money out of {}",
                step.index,
                env.merchant_state(m).name
            );
        }
    });

    assert!(
        cleared > 0,
        "the random walk never found a ring to clear, so this proved nothing: {tally:?}"
    );
    assert_eq!(cleared, ran(&tally, "clear_cycle"));
}

/// **Obligations are symmetric.** What everybody owes is what everybody is owed.
///
/// Three statements, and each one is stronger than the last:
///
/// - `Σ obligations_out == Σ obligations_in` across every merchant. Every instruction that touches
///   a debt has to touch both sides of it, or this drifts.
/// - and both of them equal **the sum of the actual edges in the graph**. The counters on a merchant
///   are a cache of the debt graph, and a cache that can disagree with the thing it caches is how a
///   merchant ends up posting collateral against a debt nobody holds — or, far worse, not posting it
///   against one somebody does.
/// - and merchant by merchant, not just in aggregate: a merchant's own outgoing edges add up to its
///   own `obligations_out`. Two errors that cancel in the total do not cancel here.
#[test]
fn obligations_are_symmetric() {
    let tally = random_walk(0x0511_1111_5EED_0007, 300, &mut |env, w, step| {
        let mut total_out = 0u128;
        let mut total_in = 0u128;
        for m in &w.merchants {
            let s = env.merchant_state(m);
            total_out += s.obligations_out as u128;
            total_in += s.obligations_in as u128;
        }

        assert_eq!(
            total_out, total_in,
            "step {} ({}): {} owed, {} owing",
            step.index, step.op, total_out, total_in
        );

        // Every directed pair, including the ones that have never traded.
        let mut edges = 0u128;
        for i in 0..MERCHANTS {
            for j in 0..MERCHANTS {
                if i != j {
                    edges += env.owed(&w.merchants[i], &w.merchants[j]) as u128;
                }
            }
        }
        assert_eq!(
            edges, total_out,
            "step {} ({}): the graph says {} and the merchants' books say {}",
            step.index, step.op, edges, total_out
        );

        for i in 0..MERCHANTS {
            let mine: u64 = (0..MERCHANTS)
                .filter(|j| *j != i)
                .map(|j| env.owed(&w.merchants[i], &w.merchants[j]))
                .sum();
            let state = env.merchant_state(&w.merchants[i]);
            assert_eq!(
                mine, state.obligations_out,
                "step {} ({}): {}'s edges do not add up to its books",
                step.index, step.op, state.name
            );
        }
    });

    assert!(
        ran(&tally, "redeem") > 0 && ran(&tally, "settle") > 0,
        "{tally:?}"
    );
    assert!(ran(&tally, "clear_cycle") > 0, "{tally:?}");
}

/// **`points_outstanding` equals minted minus burned. This is the test that proves the hook works.**
///
/// The core mints points and does not see them again until they come home. In between they sit in a
/// customer's Token-2022 account, which the core does not own, cannot freeze, and — but for one
/// deliberate, TTL-gated exception — cannot touch. The only thing standing between a point and the
/// open market is the transfer hook, and the only thing the hook will accept is a permit the core
/// signed for in the same transaction.
///
/// So the reconciliation below is not an accounting nicety. It is the hook's alibi, and it is taken
/// three ways:
///
/// - `points_outstanding == the mint's actual supply`. If a point were burned outside the protocol,
///   or minted inside it without being counted, these two part company.
/// - `total_issued - total_redeemed - total_expired == points_outstanding`. The books close against
///   themselves.
/// - **every point in existence is in a customer's wallet.** The sum of every holder's balance, plus
///   the escrow, is the entire supply. If a single point had escaped — moved to a wallet nobody in
///   this suite knows about, sold, gifted, swapped — the supply would exceed what we can account
///   for, and this line would find it.
///
/// And the escrow is zero after every instruction, always: points enter it and are burned in the
/// same breath. It is a turnstile, not a vault.
#[test]
fn points_outstanding_equals_minted_minus_burned() {
    let tally = random_walk(0x00B0_0C1E_5A11_1E55, 300, &mut |env, w, step| {
        for m in &w.merchants {
            let s = env.merchant_state(m);
            let supply = env.points_supply(m);

            assert_eq!(
                s.points_outstanding, supply,
                "step {} ({}): {} says {} points are outstanding; Token-2022 says the supply is {}",
                step.index, step.op, s.name, s.points_outstanding, supply
            );

            let net = s
                .total_issued
                .checked_sub(s.total_redeemed)
                .and_then(|n| n.checked_sub(s.total_expired))
                .expect("more points left the protocol than ever entered it");
            assert_eq!(
                net, s.points_outstanding,
                "step {} ({}): {}'s books do not close: {} issued - {} redeemed - {} expired",
                step.index, step.op, s.name, s.total_issued, s.total_redeemed, s.total_expired
            );

            // And where every one of those points actually is.
            let held: u64 = w
                .everybody()
                .map(|c| points_held(env, m, &c.pubkey()))
                .sum();
            let escrow = env.escrow_balance(m);

            assert_eq!(
                escrow, 0,
                "step {} ({}): {} points are sitting in {}'s escrow between instructions",
                step.index, step.op, escrow, s.name
            );
            assert_eq!(
                held + escrow,
                supply,
                "step {} ({}): {} of {}'s points are unaccounted for — a point has left the protocol",
                step.index,
                step.op,
                supply.saturating_sub(held + escrow),
                s.name
            );
        }
    });

    assert!(ran(&tally, "issue") > 0, "{tally:?}");
    assert!(ran(&tally, "redeem") > 0, "{tally:?}");
    assert!(ran(&tally, "expire") > 0, "{tally:?}");
}

/// A property suite that never ran the interesting instruction is a property suite that passes for
/// the wrong reason. This one asserts, out loud, that the seeded walk really does drive every
/// instruction in the protocol to a confirmed transaction at least once — and prints the tally, so
/// the next person to change a weight can see what they did to it.
#[test]
fn every_instruction_is_actually_exercised() {
    let tally = random_walk(0x0B11_6017_ACE1_2026, 300, &mut |env, w, _step| {
        // While we are here: a merchant whose default was recorded never quietly loses the record.
        // `reinstate` clears the status; nothing clears the count.
        for m in &w.merchants {
            let s = env.merchant_state(m);
            if s.status == MerchantStatus::Defaulted {
                assert!(
                    s.defaults > 0,
                    "{} is defaulted, and no default was ever recorded",
                    s.name
                );
            }
        }
    });

    println!("{tally:#?}");

    for op in [
        "issue",
        "redeem",
        "settle",
        "clear_cycle",
        "expire",
        "liquidate",
        "reinstate",
        "deposit",
        "withdraw",
        "post_offer",
        "warp",
    ] {
        assert!(
            ran(&tally, op) > 0,
            "the walk never once succeeded at `{op}`, so every property above it is weaker than it looks:\n{tally:#?}"
        );
    }
}
