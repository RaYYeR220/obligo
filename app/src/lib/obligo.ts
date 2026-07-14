import { Buffer } from 'buffer';
import { Connection, PublicKey } from '@solana/web3.js';
import {
  ObligoClient,
  OBLIGO_PROGRAM_ID,
  decodeMerchant,
  decodeObligation,
  decodeOffer,
  healthBps,
  isSolvent,
  requiredCollateral,
  type Merchant,
  type AcceptanceOffer,
  type Protocol,
} from '@obligo/sdk';
import type { ObligationEdge } from '@obligo/sdk';

export const RPC_DEFAULT = 'https://api.devnet.solana.com';

export function getRpc(): string {
  try {
    return localStorage.getItem('obligo.rpc') || RPC_DEFAULT;
  } catch {
    return RPC_DEFAULT;
  }
}

export const connection = new Connection(getRpc(), 'confirmed');

// Fixed on-chain account sizes (Anchor InitSpace). Used as getProgramAccounts filters so the whole
// live graph loads in three calls, independent of who registered the merchants.
const SIZE = { merchant: 222, obligation: 81, offer: 99, batch: 89 } as const;

export interface MerchantView extends Merchant {
  pda: PublicKey;
  address: string;
  health: bigint | null;
  required: bigint;
  solvent: boolean;
}

export interface EdgeView {
  debtor: PublicKey;
  creditor: PublicKey;
  debtorStr: string;
  creditorStr: string;
  amount: bigint;
}

export interface OfferView extends AcceptanceOffer {
  acceptorStr: string;
  issuerStr: string;
}

export interface Network {
  protocol: Protocol | null;
  merchants: MerchantView[];
  edges: EdgeView[];
  offers: OfferView[];
  loadedAt: number;
}

function toMerchantView(pda: PublicKey, m: Merchant): MerchantView {
  return {
    ...m,
    pda,
    address: pda.toBase58(),
    health: healthBps(m),
    required: requiredCollateral(m),
    solvent: isSolvent(m),
  };
}

/** Read the entire live obligation graph from devnet in three getProgramAccounts calls. */
export async function loadNetwork(client: ObligoClient): Promise<Network> {
  const conn = client.connection;
  const [protocol, mAccs, oAccs, ofAccs] = await Promise.all([
    client.getProtocol(),
    conn.getProgramAccounts(OBLIGO_PROGRAM_ID, { filters: [{ dataSize: SIZE.merchant }] }),
    conn.getProgramAccounts(OBLIGO_PROGRAM_ID, { filters: [{ dataSize: SIZE.obligation }] }),
    conn.getProgramAccounts(OBLIGO_PROGRAM_ID, { filters: [{ dataSize: SIZE.offer }] }),
  ]);

  const merchants = mAccs
    .map((a) => toMerchantView(a.pubkey, decodeMerchant(Buffer.from(a.account.data))))
    .sort((a, b) => Number(b.collateral - a.collateral));

  const edges: EdgeView[] = oAccs
    .map((a) => decodeObligation(Buffer.from(a.account.data)))
    .filter((o) => o.amount > 0n)
    .map((o) => ({
      debtor: o.debtor,
      creditor: o.creditor,
      debtorStr: o.debtor.toBase58(),
      creditorStr: o.creditor.toBase58(),
      amount: o.amount,
    }));

  const offers: OfferView[] = ofAccs
    .map((a) => decodeOffer(Buffer.from(a.account.data)))
    .map((o) => ({ ...o, acceptorStr: o.acceptor.toBase58(), issuerStr: o.issuer.toBase58() }));

  return { protocol, merchants, edges, offers, loadedAt: Date.now() };
}

export function edgesForCycleFinder(edges: EdgeView[]): ObligationEdge[] {
  return edges.map((e) => ({ debtor: e.debtor, creditor: e.creditor, amount: e.amount }));
}

/** Aggregate stats across the network — the header ticker. */
export function networkStats(net: Network) {
  const grossDebt = net.edges.reduce((s, e) => s + e.amount, 0n);
  const collateral = net.merchants.reduce((s, m) => s + m.collateral, 0n);
  const insolvent = net.merchants.filter((m) => !m.solvent && m.required > 0n).length;
  const active = net.merchants.filter((m) => m.status === 0).length;
  return {
    merchants: net.merchants.length,
    active,
    edges: net.edges.length,
    grossDebt,
    collateral,
    offers: net.offers.length,
    insolvent,
  };
}

export function makeClient(): ObligoClient {
  return new ObligoClient(connection);
}
