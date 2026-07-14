// Proof that @obligo/sdk works, end to end, against the LIVE devnet deployment.
//
// It reuses the on-chain protocol singleton (reading its settlement mint through the SDK), then
// drives the whole mandatory flow — register -> create mint -> deposit -> post offer -> issue ->
// redeem — three times, to weave a ring of debt CAFE -> SHOP -> KIOSK -> CAFE. It then asks the SDK
// to *find* that ring in the on-chain graph (client-side cycle detection) and clears it, proving the
// headline mechanism: a cycle of obligations cancelled with zero USDC moved.
//
// Run from the sdk/ directory:  node examples/prove.ts
// (Node 23+ runs TypeScript directly; web3.js/spl-token resolve from the repo's node_modules.)

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  LAMPORTS_PER_SOL,
} from '@solana/web3.js';
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  createAssociatedTokenAccountInstruction,
  createMintToInstruction,
} from '@solana/spl-token';
import { ObligoClient, findClearableCycle, healthBps } from '../src/index.ts';

const RPC = 'https://api.devnet.solana.com';
const usd = (v: bigint) => '$' + (Number(v) / 1e6).toFixed(2);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
function assert(cond: boolean, msg: string) {
  if (!cond) throw new Error('ASSERTION FAILED: ' + msg);
  console.log('   ok  ' + msg);
}

const connection = new Connection(RPC, 'confirmed');
const payer = Keypair.fromSecretKey(
  Uint8Array.from(
    JSON.parse(fs.readFileSync(path.join(os.homedir(), '.config', 'solana', 'id.json'), 'utf8')),
  ),
);

// One send helper so every tx gets a priority fee and a generous ceiling on the flaky public RPC.
async function send(
  client: ObligoClient,
  label: string,
  ixs: Parameters<ObligoClient['sendAndConfirm']>[0],
  signers: Keypair[],
  computeUnits?: number,
) {
  const sig = await client.sendAndConfirm(ixs, signers, {
    feePayer: payer.publicKey,
    priorityMicroLamports: 5000,
    computeUnits,
  });
  console.log(`   tx  ${label}  ${sig}`);
  await sleep(400);
  return sig;
}

interface Store {
  name: string;
  authority: Keypair;
  merchant: PublicKey;
}

async function main() {
  console.log('payer', payer.publicKey.toBase58());
  const startBalance = await connection.getBalance(payer.publicKey);
  console.log('balance', (startBalance / LAMPORTS_PER_SOL).toFixed(4), 'SOL\n');

  // ---- 1. load the protocol through the SDK -------------------------------------------------
  const client = await ObligoClient.load(connection);
  const protocol = await client.getProtocol();
  if (!protocol) throw new Error('protocol not initialized on devnet');
  const usdcMint = protocol.usdcMint;
  console.log('protocol loaded — settlement mint', usdcMint.toBase58());
  assert(client.usdcMint!.equals(usdcMint), 'ObligoClient.load discovered the settlement mint');

  // The payer controls the devnet test-USDC mint (it minted it at genesis); top up its own ATA so
  // it can fund merchant collateral.
  const payerUsdc = getAssociatedTokenAddressSync(
    usdcMint,
    payer.publicKey,
    false,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );
  const usdcIxs = [];
  if (!(await connection.getAccountInfo(payerUsdc))) {
    usdcIxs.push(
      createAssociatedTokenAccountInstruction(
        payer.publicKey,
        payerUsdc,
        payer.publicKey,
        usdcMint,
        TOKEN_PROGRAM_ID,
        ASSOCIATED_TOKEN_PROGRAM_ID,
      ),
    );
  }
  usdcIxs.push(
    createMintToInstruction(usdcMint, payerUsdc, payer.publicKey, 1_000_000_000n, [], TOKEN_PROGRAM_ID),
  );
  await send(client, 'mint 1,000 test-USDC to payer', usdcIxs, [payer]);

  // ---- 2. three merchants, each running its own points programme -----------------------------
  const names = ['CAFE', 'SHOP', 'KIOSK'];
  const stores: Record<string, Store> = {};
  for (const name of names) {
    const authority = Keypair.generate();
    stores[name] = { name, authority, merchant: client.merchant(authority.publicKey) };
  }

  console.log('\n== register + mint + collateralise 3 merchants ==');
  for (const name of names) {
    const s = stores[name];
    // Fund the merchant authority so it can pay rent for its own account inits.
    await send(
      client,
      `fund ${name} authority`,
      [
        SystemProgram.transfer({
          fromPubkey: payer.publicKey,
          toPubkey: s.authority.publicKey,
          lamports: Math.round(0.05 * LAMPORTS_PER_SOL),
        }),
      ],
      [payer],
    );
    // $0.01 per point, 30% reserve, 1h TTL.
    await send(
      client,
      `register ${name}`,
      [client.registerMerchant({ authority: s.authority.publicKey, name, usdcPerPoint: 10_000, reserveBps: 3000, pointTtl: 3600 })],
      [payer, s.authority],
    );
    await send(
      client,
      `create ${name} points mint`,
      [client.createPointsMint({ authority: s.authority.publicKey, name: `${name} Points`, symbol: name, uri: `https://obligo.xyz/${name}.json` })],
      [payer, s.authority],
      600_000,
    );
    await send(
      client,
      `deposit $50 collateral to ${name}`,
      [client.depositCollateral({ depositor: payer.publicKey, merchant: s.merchant, fromUsdc: payerUsdc, amount: 50_000_000 })],
      [payer],
    );
  }

  // ---- 3. weave a ring: CAFE -> SHOP -> KIOSK -> CAFE -----------------------------------------
  // Edge debtor -> creditor is born when `debtor` issues points and `creditor` honours them. So for
  // each edge: the creditor posts an offer, the debtor issues to a customer, and the customer redeems.
  const FAR = Math.floor(Date.now() / 1000) + 30 * 86_400;
  const ring: [string, string, number][] = [
    ['CAFE', 'SHOP', 500], // CAFE owes SHOP $5.00
    ['SHOP', 'KIOSK', 700], // SHOP owes KIOSK $7.00
    ['KIOSK', 'CAFE', 600], // KIOSK owes CAFE $6.00
  ];

  console.log('\n== issue + offer + redeem (register->issue->offer->redeem, x3) ==');
  for (const [debtorName, creditorName, points] of ring) {
    const debtor = stores[debtorName];
    const creditor = stores[creditorName];
    const customer = Keypair.generate();

    await send(
      client,
      `${creditorName} offers to honour ${debtorName} @100%`,
      [client.postOffer({ acceptorAuthority: creditor.authority.publicKey, issuerMerchant: debtor.merchant, rateBps: 10_000, capacity: 1_000_000_000, expiresAt: FAR })],
      [payer, creditor.authority],
    );
    await send(
      client,
      `${debtorName} issues ${points} points to a customer`,
      [client.issuePoints({ authority: debtor.authority.publicKey, customer: customer.publicKey, amount: points })],
      [payer, debtor.authority],
    );
    await send(
      client,
      `customer redeems ${points} ${debtorName} points at ${creditorName}`,
      [client.redeem({ payer: payer.publicKey, customer: customer.publicKey, issuerMerchant: debtor.merchant, acceptorMerchant: creditor.merchant, points })],
      [payer, customer],
      400_000,
    );
  }

  // ---- 4. assert the redemptions landed (the mandatory flow, proven on chain) ----------------
  console.log('\n== verify the debt graph the SDK just built ==');
  const cafe = stores.CAFE, shop = stores.SHOP, kiosk = stores.KIOSK;
  const eCafeShop = await client.getObligationAmount(cafe.merchant, shop.merchant);
  const eShopKiosk = await client.getObligationAmount(shop.merchant, kiosk.merchant);
  const eKioskCafe = await client.getObligationAmount(kiosk.merchant, cafe.merchant);
  console.log(`   edges  CAFE→SHOP ${usd(eCafeShop)}  SHOP→KIOSK ${usd(eShopKiosk)}  KIOSK→CAFE ${usd(eKioskCafe)}`);
  assert(eCafeShop === 5_000_000n, 'redeem created CAFE→SHOP = $5.00');
  assert(eShopKiosk === 7_000_000n, 'redeem created SHOP→KIOSK = $7.00');
  assert(eKioskCafe === 6_000_000n, 'redeem created KIOSK→CAFE = $6.00');

  const cafeState = (await client.getMerchant(cafe.merchant))!;
  assert(cafeState.obligationsOut === 5_000_000n, 'CAFE.obligations_out = $5.00 (points became debt)');
  assert(cafeState.pointsOutstanding === 0n, 'CAFE.points_outstanding fell to 0 (points were burned)');
  assert(cafeState.totalRedeemed === 500n, 'CAFE.total_redeemed = 500 points');
  const h = healthBps(cafeState);
  console.log('   CAFE health after redemption:', h === null ? '∞' : (Number(h) / 100).toFixed(1) + '%');

  // ---- 5. the headline: let the SDK FIND the cycle, then clear it ----------------------------
  console.log('\n== findClearableCycle + clear_cycle (zero USDC moves) ==');
  const merchants = [cafe.merchant, shop.merchant, kiosk.merchant];
  const edges = await client.loadObligationEdges(merchants);
  const cycle = findClearableCycle(edges, { programId: client.programId });
  assert(cycle !== null, 'findClearableCycle located the ring in the on-chain graph');
  console.log('   ring:', cycle!.ring.map((m) => Object.values(stores).find((s) => s.merchant.equals(m))!.name).join(' → '));
  console.log('   smallest edge (max cancellable):', usd(cycle!.minAmount));
  assert(cycle!.minAmount === 5_000_000n, 'the clearable amount is the smallest edge, $5.00');

  const vaultsBefore = await Promise.all(merchants.map((m) => connection.getTokenAccountBalance(client.vault(m)).then((b) => b.value.amount)));

  await send(client, 'clear_cycle(3)', [client.clearCycle({ cranker: payer.publicKey, cycle: cycle! })], [payer]);

  const eCafeShop2 = await client.getObligationAmount(cafe.merchant, shop.merchant);
  const eShopKiosk2 = await client.getObligationAmount(shop.merchant, kiosk.merchant);
  const eKioskCafe2 = await client.getObligationAmount(kiosk.merchant, cafe.merchant);
  console.log(`   edges after  CAFE→SHOP ${usd(eCafeShop2)}  SHOP→KIOSK ${usd(eShopKiosk2)}  KIOSK→CAFE ${usd(eKioskCafe2)}`);
  assert(eCafeShop2 === 0n, 'CAFE→SHOP cleared to $0.00');
  assert(eShopKiosk2 === 2_000_000n, 'SHOP→KIOSK reduced by $5.00 to $2.00');
  assert(eKioskCafe2 === 1_000_000n, 'KIOSK→CAFE reduced by $5.00 to $1.00');

  const vaultsAfter = await Promise.all(merchants.map((m) => connection.getTokenAccountBalance(client.vault(m)).then((b) => b.value.amount)));
  assert(vaultsBefore.every((b, i) => b === vaultsAfter[i]), 'every vault is unchanged — $15.00 of obligations extinguished, $0.00 moved');

  const endBalance = await connection.getBalance(payer.publicKey);
  console.log('\nSOL spent:', ((startBalance - endBalance) / LAMPORTS_PER_SOL).toFixed(4));
  console.log('\n============================================');
  console.log(' SDK PROOF PASSED — @obligo/sdk drove register → issue → offer → redeem');
  console.log(' (x3) and findClearableCycle → clear_cycle end to end on devnet.');
  console.log('============================================');
}

main().catch((e) => {
  console.error('\nSDK PROOF FAILED:', e?.message ?? e);
  process.exit(1);
});
