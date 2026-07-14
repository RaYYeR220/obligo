// Formatting + the health colour scale. All money is USDC micro-units (6 decimals).

export const usd = (v: bigint | number): string => {
  const n = typeof v === 'bigint' ? Number(v) : v;
  return (
    '$' +
    (n / 1e6).toLocaleString('en-US', {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    })
  );
};

export const usdCompact = (v: bigint | number): string => {
  const n = (typeof v === 'bigint' ? Number(v) : v) / 1e6;
  if (Math.abs(n) >= 1000) return '$' + (n / 1000).toFixed(1) + 'k';
  return '$' + n.toFixed(2);
};

export const bps = (v: number): string => (v / 100).toFixed(v % 100 === 0 ? 0 : 1) + '%';

export const pct = (v: bigint | null): string => {
  if (v === null) return '∞';
  const n = Number(v) / 100;
  if (n >= 100000) return '∞';
  return n.toLocaleString('en-US', { maximumFractionDigits: 0 }) + '%';
};

export const shortAddr = (a: string, n = 4): string =>
  a.length <= n * 2 + 1 ? a : `${a.slice(0, n)}…${a.slice(-n)}`;

export const num = (v: bigint | number): string =>
  (typeof v === 'bigint' ? Number(v) : v).toLocaleString('en-US');

const lerp = (a: number, b: number, t: number) => a + (b - a) * t;
const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

/**
 * The health colour. Red is a merchant that cannot pay the debts it has actually incurred
 * (liquidatable); the scale runs amber → green with the reserve coverage ratio. `null` health
 * (nothing required) reads as fully healthy.
 */
export function healthColor(healthBpsValue: bigint | null, solvent: boolean): string {
  if (!solvent) return '#ff4747';
  const ratio = healthBpsValue === null ? 3 : Number(healthBpsValue) / 10000;
  const t = clamp((ratio - 0.5) / 1.75, 0, 1);
  const hue = lerp(36, 152, t);
  const sat = lerp(92, 74, t);
  const light = lerp(56, 60, t);
  return `hsl(${hue.toFixed(0)} ${sat.toFixed(0)}% ${light.toFixed(0)}%)`;
}

export function healthLabel(healthBpsValue: bigint | null, solvent: boolean): string {
  if (!solvent) return 'INSOLVENT';
  if (healthBpsValue === null) return 'IDLE';
  const ratio = Number(healthBpsValue) / 10000;
  if (ratio >= 2) return 'STRONG';
  if (ratio >= 1) return 'COVERED';
  if (ratio >= 0.6) return 'THIN';
  return 'STRESSED';
}
