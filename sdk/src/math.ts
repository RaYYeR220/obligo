import { BPS } from './constants.ts';
import type { Merchant } from './coder.ts';

/**
 * Collateral a merchant must hold: the full value of what it already owes, plus a fractional
 * reserve against the points still at large. Mirrors `math::required_collateral` on chain.
 */
export function requiredCollateral(m: Merchant): bigint {
  const atLarge = m.pointsOutstanding * m.usdcPerPoint;
  const reserve = (atLarge * BigInt(m.reserveBps)) / BPS;
  return m.obligationsOut + reserve;
}

/**
 * Collateral as a fraction of what is required, in bps. `10_000` is exactly fully reserved.
 * `null` means "requires nothing" — infinitely healthy.
 */
export function healthBps(m: Merchant): bigint | null {
  const required = requiredCollateral(m);
  if (required === 0n) return null;
  return (m.collateral * BPS) / required;
}

/** Can the merchant pay the debts it has actually incurred? Below this line, anyone may liquidate. */
export function isSolvent(m: Merchant): boolean {
  return m.collateral >= m.obligationsOut;
}
