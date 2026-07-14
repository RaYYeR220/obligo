// Offline sanity check: the module loads, PDAs derive, instructions encode, and the cycle finder
// behaves — all with no network. Run: node examples/smoke.ts
import { Connection, Keypair, PublicKey } from '@solana/web3.js';
import {
  ObligoClient,
  findClearableCycle,
  merchantPda,
  protocolPda,
  discriminator,
  decodeMerchant,
  OBLIGO_PROGRAM_ID,
  type ObligationEdge,
} from '../src/index.ts';

// 1. PDA derivation
const auth = Keypair.generate().publicKey;
const m = merchantPda(auth);
console.log('protocolPda', protocolPda().toBase58());
console.log('merchantPda', m.toBase58());
if (protocolPda().toBase58().length < 32) throw new Error('bad protocol pda');

// 2. instruction building
const client = new ObligoClient(new Connection('https://api.devnet.solana.com'), {
  usdcMint: Keypair.generate().publicKey,
});
const ix = client.registerMerchant({ authority: auth, name: 'Cafe', usdcPerPoint: 10_000, reserveBps: 3000, pointTtl: 3600 });
console.log('registerMerchant keys:', ix.keys.length, 'programId matches:', ix.programId.equals(OBLIGO_PROGRAM_ID));
if (ix.keys.length !== 7) throw new Error('register should have 7 keys');
if (!ix.data.subarray(0, 8).equals(discriminator('register_merchant'))) throw new Error('bad register discriminator');

const redeemIx = client.redeem({
  payer: auth,
  customer: Keypair.generate().publicKey,
  issuerMerchant: m,
  acceptorMerchant: merchantPda(Keypair.generate().publicKey),
  points: 500,
});
console.log('redeem keys:', redeemIx.keys.length);
if (redeemIx.keys.length !== 18) throw new Error('redeem should have 18 keys');
if (!redeemIx.data.subarray(0, 8).equals(discriminator('redeem'))) throw new Error('bad redeem discriminator');

// 3. cycle finder on a synthetic 3-ring A->B->C->A
const A = Keypair.generate().publicKey, B = Keypair.generate().publicKey, C = Keypair.generate().publicKey;
const edges: ObligationEdge[] = [
  { debtor: A, creditor: B, amount: 5_000_000n },
  { debtor: B, creditor: C, amount: 7_000_000n },
  { debtor: C, creditor: A, amount: 6_000_000n },
];
const cycle = findClearableCycle(edges);
if (!cycle) throw new Error('cycle not found');
console.log('cycle ring length:', cycle.ring.length, 'minAmount:', cycle.minAmount.toString());
if (cycle.ring.length !== 3) throw new Error('ring should be length 3');
if (cycle.minAmount !== 5_000_000n) throw new Error('minAmount should be 5_000_000');

// no cycle in an acyclic graph
const acyclic = findClearableCycle([{ debtor: A, creditor: B, amount: 1n }, { debtor: B, creditor: C, amount: 1n }]);
if (acyclic !== null) throw new Error('acyclic graph must return null');

// zero-balance edges are ignored
const dead = findClearableCycle([
  { debtor: A, creditor: B, amount: 0n },
  { debtor: B, creditor: C, amount: 7_000_000n },
  { debtor: C, creditor: A, amount: 6_000_000n },
]);
if (dead !== null) throw new Error('a ring with a dead edge is not clearable');

console.log('\nSMOKE OK — module loads, PDAs derive, ix build, cycle finder works');
