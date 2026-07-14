# @obligo/sdk

A typed TypeScript client for **Obligo** — a permissionless clearing house for loyalty liabilities on
Solana. Merchants issue Token-2022 points against fractional USDC collateral; other merchants honour
them and accrue obligations; those obligations are netted, cleared around cycles, and liquidated. This
SDK wraps every core instruction, derives every PDA, decodes every account, and — the piece
integrators most need — **finds a clearable cycle in the obligation graph** and builds the
`clear_cycle` call for it.

There is no IDL to generate from (the toolchain's anchor-cli is version-mismatched and emits none), so
the wire format is hand-rolled: `sha256("global:<ix>")[..8]` discriminators plus borsh args, with the
account metas each instruction's `#[derive(Accounts)]` declares. Every encoding here is the one the
devnet transcript proved end to end.

```bash
npm install @obligo/sdk   # peers: @solana/web3.js@^1.98.4, @solana/spl-token@^0.4.15
```

## Usage — register → issue → offer → redeem → clear_cycle

```ts
import { Connection, Keypair } from '@solana/web3.js';
import { ObligoClient } from '@obligo/sdk';

const connection = new Connection('https://api.devnet.solana.com', 'confirmed');
const client = await ObligoClient.load(connection);           // reads the on-chain settlement mint
const cafe = Keypair.generate(), shop = Keypair.generate(), customer = Keypair.generate();
const cafeM = client.merchant(cafe.publicKey), shopM = client.merchant(shop.publicKey);

await client.sendAndConfirm([client.registerMerchant({ authority: cafe.publicKey, name: 'Cafe', usdcPerPoint: 10_000, reserveBps: 3000, pointTtl: 3600 })], [cafe]);
await client.sendAndConfirm([client.createPointsMint({ authority: cafe.publicKey, name: 'Cafe Points', symbol: 'CAFE', uri: 'https://obligo.xyz/cafe.json' })], [cafe], { computeUnits: 600_000 });
await client.sendAndConfirm([client.depositCollateral({ depositor: cafe.publicKey, merchant: cafeM, fromUsdc, amount: 50_000_000 })], [cafe]); // reserve backs issuance
await client.sendAndConfirm([client.issuePoints({ authority: cafe.publicKey, customer: customer.publicKey, amount: 500 })], [cafe]);
await client.sendAndConfirm([client.postOffer({ acceptorAuthority: shop.publicKey, issuerMerchant: cafeM, rateBps: 11_000, capacity: 1_000_000_000, expiresAt: Math.floor(Date.now() / 1e3) + 86_400 })], [shop]);
await client.sendAndConfirm([client.redeem({ payer: shop.publicKey, customer: customer.publicKey, issuerMerchant: cafeM, acceptorMerchant: shopM, points: 500 })], [shop, customer]);

// Once a ring of debt has formed across several merchants, find and clear it — zero USDC moves:
const cycle = await client.findClearableCycle([cafeM, shopM /* , … */]);
if (cycle) await client.sendAndConfirm([client.clearCycle({ cranker: shop.publicKey, cycle })], [shop]);
```

## What's in the box

- **`ObligoClient`** — connection + program config, one method per instruction (`initProtocol`,
  `registerMerchant`, `setTerms`, `depositCollateral`, `withdrawCollateral`, `createPointsMint`,
  `issuePoints`, `postOffer`, `cancelOffer`, `redeem`, `settle`, `clearCycle`, `liquidate`,
  `reinstate`, `expirePoints`) returning a `TransactionInstruction`, plus `sendAndConfirm` with
  backoff for the rate-limited public RPC, and account readers (`getMerchant`, `getObligation`,
  `getOffer`, `getProtocol`, `getPointBatch`, `loadObligationEdges`).
- **PDA helpers** — `protocolPda`, `authorityPda`, `merchantPda`, `vaultPda`, `pointsMintPda`,
  `batchPda`, `offerPda`, `obligationPda`, `eamlPda`, `permitPda`, `pointsAta`, `redemptionEscrow`.
- **Account decoders** — `decodeMerchant`, `decodeObligation`, `decodeOffer`, `decodeProtocol`,
  `decodePointBatch`, with typed results.
- **Cycle detection** — `findClearableCycle(edges)` walks the obligation graph for a clearable ring
  (length 3–8, every edge positive) and `clearCycleRemainingAccounts` lays out the accounts
  `clear_cycle` re-derives, in order. `client.findClearableCycle(merchants)` reads the graph and does
  both.
- **Solvency math** — `requiredCollateral`, `healthBps`, `isSolvent`, mirroring the on-chain numbers.

## Program IDs (devnet)

| Program | Address |
|---|---|
| core `obligo` | `3YV8p2MhPQpSZS8fp3D7VoQYPy8GwxV221koWpsptRpN` |
| hook `obligo_hook` | `AtDpNdzKVRxMwK5bTotfmjxQdVU854RopJccgYRP8wQ7` |

Override either via the `ObligoClient` constructor (`{ programId, hookId }`).

## Proof

`examples/prove.ts` drives the whole mandatory flow three times against **live devnet** to weave a ring
CAFE → SHOP → KIOSK → CAFE, then asks the SDK to find that ring on chain and clear it — proving both
the redemption path and the headline mechanism. `examples/smoke.ts` checks the pure pieces (PDAs,
instruction encoding, cycle detection) with no network.

```bash
node examples/smoke.ts     # offline
node examples/prove.ts     # devnet; uses ~/.config/solana/id.json as payer
```

The source ships as TypeScript; Node 23+ runs it directly (type stripping), or compile with `tsc`.

## License

MIT
