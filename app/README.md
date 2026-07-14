# Obligo — web client

A live obligation-graph explorer and clearing-house terminal for the Obligo protocol on Solana
devnet. It reads real on-chain state through the [`@obligo/sdk`](../sdk) package and can drive the
protocol's headline mechanism — **cycle clearing**: finding a ring of debt and cancelling it around
the cycle with zero cash moved.

## Run it

```bash
npm install
npm run dev
```

Then open the printed URL (default `http://localhost:5173`). No wallet extension, no config — the
client connects to `https://api.devnet.solana.com` and renders the live protocol immediately.

## What you're looking at

Three surfaces, switched from the top-right nav:

- **Network** — the obligation graph of every registered merchant. Nodes are merchants (size =
  collateral posted, colour = health on a red→green scale); directed edges are debts (width =
  amount, arrow points debtor→creditor). Hover a node to isolate its books; click it for the full
  ledger in the registry panel. **Scan for cycle** walks the live graph for a clearable ring and, if
  you've imported a funded key, runs `clear_cycle` on-chain — then plays the money shot: the ring
  collapses to zero, a counter ticks the obligations extinguished, and **$0.00 moved** stamps in.
  With no key it plays as a labelled preview.
- **Console** — run a merchant. Register, set terms, create a Token-2022 points mint, post/cancel
  acceptance offers, deposit collateral, issue points. Signs with your dev key; each write links to
  its devnet transaction.
- **Redeem** — the customer view. See which merchants' points your key holds, pick an accepting
  venue and its live rate, and spend them — watch the obligation get created while USDC moved stays
  at $0.00.

## Reads vs. writes

**Reads need no key.** The graph, registry, offers and books are all live devnet state.

**Writes are signed locally with a dev key** — no browser wallet required, so it stays
judge-runnable. Open the **DEV KEY** menu (top-right) and either paste a devnet secret key (base58
or a `[64]`-byte array) or mint a burner and fund it with free devnet SOL
([faucet.solana.com](https://faucet.solana.com)). The key lives only in this browser's
localStorage — use a throwaway.

- `clear_cycle` is permissionless and needs only SOL for the fee, so any funded burner can run the
  money shot on a live ring.
- Registering a merchant, creating a points mint, and posting offers need only SOL.
- Depositing collateral and issuing points need the devnet settlement token (a dev-controlled test
  mint). Those flows are guarded and say so rather than faking success.

## How it connects

Everything on-chain goes through `@obligo/sdk`: `ObligoClient` for instruction building and sending,
the PDA helpers and account decoders for reads, and `findClearableCycle` /
`clearCycleRemainingAccounts` for the cycle finder. The SDK ships as TypeScript source and is aliased
straight into the Vite build, so the client runs the exact code an integrator would import.

## Stack

React + Vite + TypeScript · `d3-force` for the graph layout · `framer-motion` for the money-shot
choreography · `@solana/web3.js` + `@solana/spl-token`.
