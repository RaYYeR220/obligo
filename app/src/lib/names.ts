import type { MerchantView } from './obligo.ts';

/**
 * A merchant's on-chain `name` is a short free-text handle (CAFE, SHOP, …) and several merchants
 * may share one — so the label is the name, and the short address disambiguates it everywhere.
 */
export function displayName(m: { name: string }): { label: string } {
  const n = (m.name || 'MERCHANT').trim().toUpperCase();
  return { label: n.length > 12 ? n.slice(0, 12) : n };
}

export function fullLabel(m: MerchantView): string {
  return `${displayName(m).label} · ${m.address.slice(0, 4)}`;
}
