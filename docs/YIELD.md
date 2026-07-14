# Self-funding loyalty: the `YieldAdapter` seam

A merchant's collateral sits idle between the moment it is posted and the moment a creditor is paid.
Obligo can put that idle USDC to work in a lending market so the collateral earns yield — "self-funding
loyalty" — **without touching the solvency invariant and without adding any dependency to the devnet
demo path.** This is an opt-in depth-add, not a change to how the audited core behaves.

## The seam

The core routes `deposit_collateral` and `withdraw_collateral` through one trait
([`programs/obligo/src/yield_adapter.rs`](../programs/obligo/src/yield_adapter.rs)):

```rust
pub trait YieldAdapter {
    fn deposit(&self, principal_in: u64) -> Result<()>;
    fn withdraw(&self, principal_out: u64) -> Result<u64>; // returns USDC actually freed
    fn total_assets(&self) -> Result<u64>;                 // principal + accrued
}
```

There are two implementations, selected at compile time:

| | **`NullAdapter`** (default) | **`KaminoAdapter`** (`feature = "kamino"`) |
|---|---|---|
| Where collateral rests | plain USDC in the vault | routed into Kamino's KLend USDC reserve as cTokens |
| `deposit` | no-op — the USDC is already in the vault | `refresh_reserve` + `deposit_reserve_liquidity` |
| `withdraw` | passthrough, returns `principal_out` | `refresh_reserve` + `redeem_reserve_collateral`, returns realized USDC |
| `total_assets` | vault balance | cToken balance valued at the reserve's exchange rate + idle USDC |
| Compiled on devnet? | **yes — this is the demo path** | **no — devnet never compiles the `kamino` feature** |

With `NullAdapter`, `deposit`/`withdraw` do nothing beyond what the surrounding instruction already did.
The seam is real — the two collateral instructions genuinely call it — but its on-chain effect is
**byte-for-byte identical to the pre-seam behaviour**, which is why all 102 existing tests keep passing
unchanged.

## Solvency is measured on principal, never on principal + yield

This is the load-bearing rule. `math.rs` and its invariant tests are untouched. Solvency asks only
whether a merchant can cover the debts it has already incurred, and it asks it of `collateral` — the
principal the merchant *deposited* — never of whatever that principal has since grown into. Yield is
strictly **additive and claimable**: it can make a merchant wealthier, but it can never, by being
counted early, let a merchant issue points it cannot back. `total_assets()` exists for reporting a
merchant's claimable balance; nothing in the solvency path reads it.

## Why hand-rolled CPI

KLend (`KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`) is on Anchor 0.29, `publish = false`, BUSL-1.1 —
there is no Anchor-1.x-compatible CPI crate for it and there is not going to be one. So `KaminoAdapter`
talks to KLend the same way `hook_cpi.rs` talks to the transfer hook: a hand-built `Instruction` and
`invoke_signed`, with the discriminators and account orders pinned in source. Nothing from KLend/Kamino
is vendored into this repo.

## The proof — real yield against real mainnet KLend

[`kamino-fork/tests/fork.rs`](../kamino-fork/tests/fork.rs) is a LiteSVM harness that loads the **actual
mainnet KLend bytecode** and the **actual mainnet USDC reserve state**, then drives the real
`KaminoAdapter` (through a throwaway probe program that calls the same `vault_adapter()` factory the core
uses): deposit 1,000 USDC, advance the clock ~180 days, redeem the whole position.

Only two things are cheated, both of them legitimate fork moves: we seed ourselves USDC by writing a
token account (the equivalent of Surfpool's `surfnet_setTokenAccount`), and we rewrite the scope price's
timestamp so `refresh_reserve` does not reject it as stale. **Every number KLend derives — the exchange
rate, the compounded interest — is its own.**

Latest run:

```
deposited             1000.000000 USDC
cTokens received      839.292145 cUSDC        (exchange rate ~1.192 USDC/cToken)
clock advanced        31,000,000 slots (~180 days)
withdraw realized     1014.749609 USDC
accrued interest      14.749609 USDC          ← genuine KLend supply interest, not a mock
```

Mainnet accounts used (all verified: the reserve's `liquidity.mint_pubkey` is USDC and its
`lending_market` is the Kamino main market):

| role | pubkey |
|---|---|
| KLend program | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` |
| lending market (main) | `7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF` |
| USDC reserve | `D6q6wuQSrifJKZYpR1M8R4YawnLDtDsMmWM1NbBmgJ59` |
| USDC mint | `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v` |
| cUSDC collateral mint | `B8V6WVjPxW1UGwVDfxH2d2r8SyT4cqn7dQRK6XneVa7D` |
| reserve liquidity supply | `Bgq7trRgVMeq33yt235zM2onQ4bRDBsY5EWiTetF4qw6` |
| scope price oracle | `3t4JZcueEzTbVP6kLxXrL3VpWx45jDer4eqysweBchNH` (chain index 13) |

Running it:

```powershell
cd kamino-fork
./fetch-fixtures.ps1        # dumps the mainnet accounts + KLend bytecode into ./fixtures (gitignored)
cargo-build-sbf             # builds the probe program
cargo test -- --nocapture   # runs the proof and prints the numbers above
```

The harness is a **standalone workspace** on purpose: if it were a member of the root workspace, a plain
`cargo-build-sbf` there would unify the `kamino` feature into the devnet `obligo.so` — the
feature-unification trap the codebase is careful about. Kept apart, the root build never sees `kamino`.

## Deliberate boundary

The scope here is bounded on purpose, so the seam is real without making the demo fragile:

- **The payout paths (`settle`, `liquidate`) stay on principal in USDC and remain oracle-free.** Threading
  Kamino through them would force a cToken redemption — and therefore an oracle read and external-liquidity
  dependency — into the exact paths whose virtue is having neither. The base collateral that backs
  obligations is kept as USDC in the vault; the yield position is proven at the adapter level and its
  production wiring is documented here rather than forced into the audited paths.
- **`withdraw` in the fork redeems the whole position** and returns the realized USDC, which is the common
  case (a merchant pulling its yield deposit) and the one the proof needs. A production partial withdrawal
  would redeem `principal_out · collateral_supply / total_liquidity` cTokens at the reserve's exchange
  rate — the same rate `total_assets()` already reads — leaving the remainder invested.
- **`refresh_reserve` is wired for a Scope-priced reserve** (the Kamino main-market USDC reserve, chain
  index 13). A Pyth-priced reserve would place its oracle in the first optional slot instead of the last;
  the target reserve is Scope, and the fork test pins it.
