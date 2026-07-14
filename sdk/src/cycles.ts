import { PublicKey, type AccountMeta } from '@solana/web3.js';
import { obligationPda } from './pdas.ts';
import { OBLIGO_PROGRAM_ID } from './constants.ts';

/** One directed edge of the debt graph: `debtor` owes `creditor` `amount` USDC micro-units. */
export interface ObligationEdge {
  debtor: PublicKey;
  creditor: PublicKey;
  amount: bigint;
}

/** A ring the protocol can clear: `ring[i] -> ring[i+1]`, closing `ring[k-1] -> ring[0]`. */
export interface ClearableCycle {
  /** Merchant PDAs, in ring order. */
  ring: PublicKey[];
  /** The `[b"obligation", debtor, creditor]` edge PDAs, one per hop, in ring order. */
  edges: PublicKey[];
  /** The amount every edge is decremented by — the smallest edge in the ring. */
  minAmount: bigint;
}

/**
 * Find a clearable cycle in an obligation graph — the piece an integrator most needs, and the one
 * that directly drives the protocol's headline mechanism.
 *
 * `clear_cycle` on chain takes a ring of merchants and the edge between each consecutive pair, and
 * cancels the smallest edge off every edge in the ring: everyone owes less, everyone is owed less,
 * and **no USDC moves**. But the program will not *find* the ring for you — its job is to prove the
 * ring you hand it is real. Finding it is the client's job, and this is it: a depth-first search for
 * a simple directed cycle of length `minLen..=maxLen` in which every edge carries a positive balance.
 *
 * Returns the first such cycle found, or `null` if the graph has none. Feed the result straight into
 * `ObligoClient.clearCycle({ cranker, cycle })`.
 */
export function findClearableCycle(
  edges: ObligationEdge[],
  opts: { minLen?: number; maxLen?: number; programId?: PublicKey } = {},
): ClearableCycle | null {
  const minLen = opts.minLen ?? 3;
  const maxLen = opts.maxLen ?? 8;
  const programId = opts.programId ?? OBLIGO_PROGRAM_ID;

  const key = (pk: PublicKey) => pk.toBase58();

  // Adjacency over live edges only; a zero-balance edge clears nothing and would only shrink the
  // amount we could cancel.
  const adj = new Map<string, string[]>();
  const nodePk = new Map<string, PublicKey>();
  const edgeAmount = new Map<string, bigint>();
  for (const e of edges) {
    if (e.amount <= 0n) continue;
    const d = key(e.debtor);
    const c = key(e.creditor);
    nodePk.set(d, e.debtor);
    nodePk.set(c, e.creditor);
    if (!adj.has(d)) adj.set(d, []);
    adj.get(d)!.push(c);
    edgeAmount.set(`${d}->${c}`, e.amount);
  }

  const path: string[] = [];
  const onPath = new Set<string>();

  const dfs = (start: string, node: string): string[] | null => {
    path.push(node);
    onPath.add(node);

    for (const next of adj.get(node) ?? []) {
      if (next === start && path.length >= minLen) {
        return [...path]; // closed a ring of an acceptable length
      }
      if (!onPath.has(next) && path.length < maxLen) {
        const found = dfs(start, next);
        if (found) return found;
      }
    }

    path.pop();
    onPath.delete(node);
    return null;
  };

  for (const start of adj.keys()) {
    const ringKeys = dfs(start, start);
    if (!ringKeys) {
      path.length = 0;
      onPath.clear();
      continue;
    }

    const ring = ringKeys.map((k) => nodePk.get(k)!);
    const edgePdas: PublicKey[] = [];
    let minAmount = 1n << 63n;
    for (let i = 0; i < ring.length; i++) {
      const d = ringKeys[i];
      const c = ringKeys[(i + 1) % ring.length];
      edgePdas.push(obligationPda(ring[i], ring[(i + 1) % ring.length], programId));
      const amt = edgeAmount.get(`${d}->${c}`)!;
      if (amt < minAmount) minAmount = amt;
    }
    return { ring, edges: edgePdas, minAmount };
  }

  return null;
}

/**
 * The `remaining_accounts` `clear_cycle` expects, in the exact order it re-derives them:
 * the `k` merchants first, then the `k` edges. All writable, none signers.
 */
export function clearCycleRemainingAccounts(cycle: ClearableCycle): AccountMeta[] {
  const metas: AccountMeta[] = [];
  for (const merchant of cycle.ring) {
    metas.push({ pubkey: merchant, isSigner: false, isWritable: true });
  }
  for (const edge of cycle.edges) {
    metas.push({ pubkey: edge, isSigner: false, isWritable: true });
  }
  return metas;
}
