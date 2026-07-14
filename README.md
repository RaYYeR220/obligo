# Obligo

**A permissionless clearing house for loyalty liabilities, on Solana.**

A loyalty point is a debt a merchant hopes you never collect. Merchants issue points, book them as
deferred revenue, and count on breakage — **roughly half of all loyalty points are never redeemed**.
When a merchant tries to make its points spendable somewhere else, the whole thing collapses: accepting
another business's points is an unpriced liability with no trustworthy way to settle up. That is why
coalition programs keep dying — Plenti, the American-Express-backed coalition of Macy's, AT&T, Exxon and
others, shut down in 2018 because members only ever spent at one or two partners and no one trusted the
settlement in between.

Obligo is the settlement layer those programs never had, built as an open protocol instead of a company.
A merchant posts collateral and issues points against it. **Any other merchant can accept those points
and pull settlement straight from the issuer's collateral** — no bilateral contract, no operator, no
permission. The protocol nets what everyone owes each other, cancels debt around cycles without moving a
cent, and liquidates issuers who can't cover what they've promised.

---

## What makes it different

There are collateral-backed loyalty products already (Spree Finance on EVM is the closest). Every one of
them is a company with a settlement admin key: whitelisted issuers, whitelisted acceptors, an
"authorized executor" that reviews and finalizes each redemption. Obligo has none of that.

| | Existing coalition / clearing products | **Obligo** |
|---|---|---|
| Who may issue points | whitelisted brands | **anyone, permissionlessly** |
| Who may accept them | whitelisted, bilateral deals | **anyone, permissionlessly** |
| Who approves a redemption | an operator / admin key | **nobody — the program does** |
| Solvency | trust the operator's books | **an invariant the program checks on every instruction** |
| Backing | full 1:1, or a database entry | **partial collateral, with the credit risk priced on-chain** |
| Settlement between merchants | bilateral, through a hub, operator takes a fee | **multilateral netting; debt around a cycle cancels with zero cash moved** |
| A merchant that can't pay | the operator's problem | **permissionless liquidation, pro-rata to creditors** |

## The mechanism

**Partial collateral is the whole idea.** A merchant escrows a *fraction* of the face value of the
points it issues — 20%, 30%, whatever it chooses. That single decision does two things at once:

- It makes loyalty **capital-efficient** enough for a real café to adopt — you don't lock up a dollar to
  give out a dollar of points.
- It gives every merchant's points **visible credit risk**. A well-collateralized issuer's points are
  worth close to face; a shaky one's trade at a discount. The market prices merchant solvency, publicly,
  on-chain. Nobody has this, because everyone else backs points 1:1 and there is nothing left to price.

**Redemption creates a debt, not a payment.** When a customer spends Café-A's points at Shop-B, the
points are burned and an *obligation* A→B is recorded. **No money moves at redemption.** Shop-B chose to
accept, having seen Café-A's on-chain health, and now holds a claim on Café-A's collateral.

**Debt nets — and that is where the collateral savings come from.** Obligations pile up as a directed
graph over merchants. Two merchants who owe each other settle only the difference. And a *ring* of
debt — A owes B, B owes C, C owes A — is cancelled around the cycle down to its smallest edge, moving
**zero cash**. The consequence is the headline:

> A merchant's collateral requirement scales with its **net** position, not its gross issuance.
> The denser the network of mutual redemption, the less collateral anyone needs.

This is what a real clearing house does (CLS for FX, ACH for payments; cycle-cancellation of debt has
been run at national scale). Obligo is the first one built on-chain for loyalty.

**Acceptance is an auction — and it is a customer-acquisition channel.** A merchant publishes an offer:
"I honour Café-A's points at 110% of face, up to $500 a week." Paying *over* face is rational — it's an
ad spend that only costs you when a customer physically walks in and spends. Redemption routes to the
best live offer, so the customer always gets the best rate without knowing any of this is happening.
Loyalty flips from a retention cost into a **priced acquisition channel**. The same number, below 100%,
is the market discounting a weak issuer's credit. One field carries both meanings.

**Breakage becomes honest.** Points carry an expiry. When they lapse, anyone can burn them; the reserve
behind them is freed and the event is public. Breakage stops being a silent accounting gain and becomes a
state transition on-chain.

## The invariant

Everything reduces to one line the program enforces on every issuance, redemption and withdrawal:

```
collateral(m)  ≥  obligations_out(m)  +  reserve_bps/10_000 · face(points_outstanding(m))
```

A merchant must hold the full value of what it already owes, plus a reserve fraction against the points
still in circulation. A redemption converts reserve-backed liability into full debt, so it *lowers* the
issuer's health — that is not a bug, it is the credit market working. When collateral falls below what a
merchant owes, anyone may liquidate it and creditors are paid pro-rata from the estate.

## Why this needs Solana

Take Solana away and the product doesn't exist. A merchant will never accept a rival's points without a
**trustless guarantee of settlement**, and there is no such guarantee off-chain — that is the exact rock
every coalition program has broken on. Only a program that holds the collateral and moves it by rule,
with no one able to intervene, makes acceptance safe.

And the point rules live in the token itself. Obligo's points are **Token-2022 with a transfer hook**:

- A point can move **only when the clearing house has authorised that exact movement**. Before any
  transfer the core issues a single-use permit; the hook consumes it. No permit, no movement — so points
  can't be dumped on a DEX, split across wallets, or arbitraged below face. The one thing that makes the
  solvency invariant meaningful — knowing where every point is — is enforced at the token level, not
  hoped for at the application level.
- The hook's authority is burned (`None`) at mint creation, so the rules can never be repointed.

A transfer hook enforcing a real economic invariant is exactly what the extension is for, and it is the
architectural core of this project, not decoration.

## Architecture

Two programs:

- **`obligo`** — the clearing house. Owns all state, collateral and accounting: the merchant registry,
  the collateral vaults, point issuance under the reserve invariant, cross-merchant redemption, bilateral
  netting, cycle clearing, liquidation, and expiry.
- **`obligo_hook`** — the Token-2022 transfer-hook program. Its only job is to permit a point movement
  when — and only when — the core authorised it, in that exact transaction.

The split is forced by Token-2022: the hook does **not** fire on mint or burn, so all issuance and
redemption accounting lives in the core, and the hook governs movement alone.

A third program, **`obligo_venue`**, is a minimal example integrator — a point-of-sale that calls the
core's `redeem` by CPI — included to show the protocol is composable by any external program.

**Instruction surface (core):** `init_protocol` · `register_merchant` · `set_terms` ·
`deposit_collateral` · `withdraw_collateral` · `create_points_mint` · `issue_points` · `post_offer` ·
`cancel_offer` · `redeem` · `settle` · `clear_cycle` · `liquidate` · `reinstate` · `expire_points`.

The protocol `authority` may change global parameters and **nothing else** — it cannot move a merchant's
collateral, mint or burn a point, cancel an obligation, or block a redemption. There is no instruction
that would let it. Everything that shrinks the debt graph or returns money (`settle`, `clear_cycle`,
`liquidate`, `reinstate`, `expire_points`) is permissionless: a public crank anyone may turn.

## Proof — it runs on devnet

Both programs are deployed and every mechanism has been exercised end-to-end on Solana devnet with real,
confirmed transactions.

| Program | Address |
|---|---|
| core `obligo` | `3YV8p2MhPQpSZS8fp3D7VoQYPy8GwxV221koWpsptRpN` |
| hook `obligo_hook` | `AtDpNdzKVRxMwK5bTotfmjxQdVU854RopJccgYRP8wQ7` |

Selected live transactions (full list in [`docs/DEVNET.md`](docs/DEVNET.md)):

- **Redemption creates a debt, moves no cash** — a customer spends 500 Café points ($5.00) at Shop.
  Obligation Café→Shop of $5.00 created; both vaults byte-identical; Café's health falls 3333% → 1000%
  as its reserve becomes full debt.
  [`4vuxzkqq…`](https://explorer.solana.com/tx/4vuxzkqqtD4Ft2zNGN4mAuZUgfyGc1yewSU95E8LkZUQUp5LFHsUGrbWKZ3xuBsT7orGLaemreqpDn1Bav6nwwrv?cluster=devnet)
- **A ring of debt cleared with zero cash** — Café→Shop→Kiosk→Café. **$15.00 of obligations
  extinguished, $0.00 of USDC moved, all three vaults byte-identical.**
  [`UagyNDAt…`](https://explorer.solana.com/tx/UagyNDAtz4FHEStnLfah8GkQJjCWiCmRWp7ju5ZBHNMxAHQXo81qbtBwYDhvCvNsyFnyNTszk9zwqLF46m23mKg?cluster=devnet)
- **Bilateral settlement** — $8 vs $3 mutual debt nets to a single $5.00 transfer.
  [`2WhtBnNk…`](https://explorer.solana.com/tx/2WhtBnNkG6j7sw4JmsGhYB7nuzPLu9mVTt1Qo76Jv4qyk3rFBz6jgtxFWU3tJt4G7ckuS736gNeycG6F3ZGq2w4t?cluster=devnet)
- **Pro-rata liquidation** — an insolvent issuer ($3 collateral, $12 owed) pays both creditors exactly
  25% of their claims.
  [`4G6u4dGJ…`](https://explorer.solana.com/tx/4G6u4dGJj49ueN12DRDJmnwSQ9uZyZCmeAu5mm45VCognTPpFFcwpzi2812VRNWDRDF7ZJ8RqwUMWkAwFbNpFH5c?cluster=devnet)
- **Breakage** — lapsed points burned, $1.00 of breakage recognised on-chain.
  [`4bQBCpbR…`](https://explorer.solana.com/tx/4bQBCpbRWebPMdmcVd3aoSjKnPEbv9FNJpuNxri4D9VVxx1VqZ3q483phJH1EPjQmjSLZd7JKJzPfmaCnFan2wXJ?cluster=devnet)

**Tests:** 102 passing across the three programs — unit tests for the reserve math, per-instruction
behaviour, adversarial tests that a forged clearing cycle is unrepresentable, a test that a third-party
program can redeem by CPI atomically, and property tests that USDC is conserved by every instruction, that
cycle clearing moves no USDC, that obligations stay symmetric, and that on-chain point supply always
reconciles with the protocol's books.

## Tradeoffs & constraints

- **The points are deliberately not tradable.** A transfer-hook mint is a closed-loop instrument —
  Raydium rejects hook mints outright, Orca gates them behind a whitelist, and wallets show warnings on
  them. For a loyalty point that is exactly right: points should move along protocol paths and nowhere
  else. The property that makes points un-listable is the same one that makes the solvency invariant
  hold. We chose it on purpose.
- **Expiry uses a permanent delegate.** Expiring lapsed points is permissionless, so no one signs for the
  customer — and a token can't be moved without an authority. The points mint therefore carries a Token-2022
  `PermanentDelegate` set to the merchant's PDA (a key nobody holds; only the program can sign for it). It
  is used in exactly one place, the TTL-gated `expire_points`, and even there the movement still needs a
  permit from the hook. It is not a custody backdoor: the freeze authority is `None` and the delegate can
  only move points into the merchant's own escrow to be burned.
- **Partial collateral means real credit risk.** A merchant issuing on a 20% reserve can go insolvent, and
  its acceptors can take a loss beyond the reserve. That is intended — it is what makes the acceptance
  market a credit market. Liquidation is pro-rata and permissionless so the loss is shared fairly and
  can't be front-run.
- **No oracle.** Collateral is a stablecoin and liabilities are denominated in it, so solvency is pure
  arithmetic. The protocol has no price feed and no external market dependency by design.
- **Settlement asset.** On mainnet the settlement asset would be Circle USDC; the protocol takes the mint
  as a genesis parameter and assumes nothing about it beyond being an SPL/Token-2022 mint. The devnet demo
  uses a self-controlled test mint so it is reproducible without a faucet.

## Composability

[`obligo_venue`](programs/obligo_venue) is a worked example: a third-party point-of-sale program that
writes a receipt and calls the core's `redeem` by CPI in the same transaction — the sale and the
redemption are atomic — with no permission from Obligo. Any dApp, POS, or game can become a redemption
venue the same way.

The [`sdk/`](sdk) package (`@obligo/sdk`) wraps every instruction, derives every PDA, decodes protocol
accounts, and includes a client-side cycle finder (`findClearableCycle`) that walks the live obligation
graph and assembles a `clear_cycle` call with its accounts in the exact order the program re-derives them —
the piece integrators need most. Its [`examples/prove.ts`](sdk/examples/prove.ts) runs the whole
register → issue → offer → redeem → **find-and-clear-a-cycle** flow against devnet.

## The client

[`app/`](app) is a terminal-style web client that reads live devnet state and renders the whole network
as a force-directed **obligation graph** — merchants sized by collateral and coloured by health, debts as
directed edges. Its centrepiece is the cycle-clearing money shot: hit **scan for cycle**, watch a ring of
debt light up, and clear it — the edges collapse to zero, a counter tallies the obligations extinguished,
and **$0.00 USDC moved** stamps in. It also has a merchant console (register, deposit, issue, post offers)
and a customer redeem view. Writes are signed with a local dev key or a funded burner — no browser
extension needed, so `clear_cycle` (which is permissionless and needs only SOL) runs from a free faucet
key. Several clearable rings are seeded live on devnet to try it against.

```bash
cd app && npm install && npm run dev
```

## Build & run the protocol

Requires the Solana toolchain (Agave 3.1+) and Rust 1.89+.

```bash
# build all three programs
cargo build-sbf

# run the full test suite (102 tests across the core, hook and venue)
cargo test

# drive the whole protocol against devnet through the SDK —
# register, issue, post offers, redeem, then find a debt cycle and clear it
cd sdk && npm install && npx tsx examples/prove.ts
```

## License

MIT.
