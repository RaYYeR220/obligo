import { useEffect, useMemo, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import {
  forceCenter,
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  forceX,
  forceY,
  type SimulationNodeDatum,
} from 'd3-force';
import type { EdgeView, MerchantView } from '../lib/obligo.ts';
import { healthColor, shortAddr, usdCompact } from '../lib/format.ts';
import { displayName } from '../lib/names.ts';

const W = 1000;
const H = 700;

export type ClearPhase = 'idle' | 'armed' | 'clearing' | 'done';

interface Node extends SimulationNodeDatum {
  id: string;
  r: number;
  m: MerchantView;
}

interface Geom {
  key: string;
  debtor: string;
  creditor: string;
  amount: bigint;
  p0: [number, number];
  cp: [number, number];
  p1: [number, number];
  width: number;
}

interface Props {
  merchants: MerchantView[];
  edges: EdgeView[];
  ring: string[] | null;
  phase: ClearPhase;
  clearProgress: number;
  minAmount: bigint;
  selected: string | null;
  onSelect: (addr: string | null) => void;
  reveal: boolean;
}

const nodeRadius = (m: MerchantView): number => {
  const c = Number(m.collateral) / 1e6;
  // floor at 22 so even a $0.00 merchant is big enough for its label to sit inside the circle
  return Math.max(22, 13 + Math.sqrt(c) * 3.1); // ~22 (empty) .. ~35 ($50)
};

const edgeWidth = (amount: bigint): number => {
  const v = Number(amount) / 1e6;
  return Math.min(7, 1.3 + Math.sqrt(v) * 1.5);
};

function bezier(p0: [number, number], cp: [number, number], p1: [number, number], t: number): [number, number] {
  const u = 1 - t;
  return [
    u * u * p0[0] + 2 * u * t * cp[0] + t * t * p1[0],
    u * u * p0[1] + 2 * u * t * cp[1] + t * t * p1[1],
  ];
}

export default function Graph(props: Props) {
  const { merchants, edges, ring, phase, clearProgress, minAmount, selected, onSelect, reveal } = props;
  const [positions, setPositions] = useState<Map<string, { x: number; y: number }>>(new Map());
  const posRef = useRef(positions);
  posRef.current = positions;
  const [hover, setHover] = useState<string | null>(null);

  // Topology key — recompute the layout only when the node set or the set of edges changes,
  // never when only amounts move, so the graph stays put across polling refreshes.
  const topoKey = useMemo(() => {
    const ns = merchants.map((m) => m.address).sort().join(',');
    const es = edges.map((e) => `${e.debtorStr}>${e.creditorStr}`).sort().join(',');
    return ns + '|' + es;
  }, [merchants, edges]);

  useEffect(() => {
    const prev = posRef.current;
    const N = merchants.length;
    const nodes: Node[] = merchants.map((m, i) => {
      const seed = prev.get(m.address);
      const a = (i / Math.max(1, N)) * Math.PI * 2;
      return {
        id: m.address,
        r: nodeRadius(m),
        m,
        x: seed?.x ?? W / 2 + Math.cos(a) * 240,
        y: seed?.y ?? H / 2 + Math.sin(a) * 200,
      };
    });
    const idx = new Map(nodes.map((n) => [n.id, n]));
    const links = edges
      .filter((e) => idx.has(e.debtorStr) && idx.has(e.creditorStr))
      .map((e) => ({ source: e.debtorStr, target: e.creditorStr }));

    const cy = H * 0.42;
    const sim = forceSimulation(nodes)
      .force('charge', forceManyBody().strength(-1600))
      .force('link', forceLink(links).id((d: SimulationNodeDatum & { id?: string }) => d.id!).distance(200).strength(0.5))
      .force('center', forceCenter(W / 2, cy))
      .force('x', forceX(W / 2).strength(0.06))
      .force('y', forceY(cy).strength(0.09))
      // collide radius leaves room for the name + address labels that hang below each node
      .force('collide', forceCollide<Node>().radius((d) => d.r + 44).strength(0.95))
      .stop();

    for (let i = 0; i < 340; i++) sim.tick();

    const padX = 74;
    const next = new Map<string, { x: number; y: number }>();
    for (const n of nodes) {
      // keep the bottom band clear — the money-shot deck and the health legend live there
      next.set(n.id, {
        x: Math.max(padX, Math.min(W - padX, n.x ?? W / 2)),
        y: Math.max(58, Math.min(H - 150, n.y ?? cy)),
      });
    }
    setPositions(next);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [topoKey]);

  const ringSet = useMemo(() => {
    const s = new Set<string>();
    if (ring) for (let i = 0; i < ring.length; i++) s.add(`${ring[i]}>${ring[(i + 1) % ring.length]}`);
    return s;
  }, [ring]);
  const ringNodes = useMemo(() => new Set(ring ?? []), [ring]);

  // Edge geometry from the settled layout.
  const geoms = useMemo<Geom[]>(() => {
    const out: Geom[] = [];
    for (const e of edges) {
      const a = positions.get(e.debtorStr);
      const b = positions.get(e.creditorStr);
      if (!a || !b) continue;
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const len = Math.hypot(dx, dy) || 1;
      const nx = -dy / len;
      const ny = dx / len;
      // curve bidirectional pairs apart; deterministic side by address order
      const side = e.debtorStr < e.creditorStr ? 1 : -1;
      const bow = 26 * side;
      const mx = (a.x + b.x) / 2 + nx * bow;
      const my = (a.y + b.y) / 2 + ny * bow;
      // trim endpoints to node edges
      const ra = (positions.get(e.debtorStr) && nodeRadius(merchants.find((m) => m.address === e.debtorStr)!)) || 16;
      const rb = (positions.get(e.creditorStr) && nodeRadius(merchants.find((m) => m.address === e.creditorStr)!)) || 16;
      const p0: [number, number] = [a.x + (dx / len) * ra, a.y + (dy / len) * ra];
      const p1: [number, number] = [b.x - (dx / len) * (rb + 7), b.y - (dy / len) * (rb + 7)];
      out.push({
        key: `${e.debtorStr}>${e.creditorStr}`,
        debtor: e.debtorStr,
        creditor: e.creditorStr,
        amount: e.amount,
        p0,
        cp: [mx, my],
        p1,
        width: edgeWidth(e.amount),
      });
    }
    return out;
  }, [edges, positions, merchants]);

  const ringActive = phase === 'armed' || phase === 'clearing' || phase === 'done';
  const focus = hover;

  // particle beads travel each ring edge while clearing
  const [beadT, setBeadT] = useState(0);
  useEffect(() => {
    if (phase !== 'clearing') return;
    let raf = 0;
    let t0 = performance.now();
    const loop = (now: number) => {
      setBeadT(((now - t0) / 900) % 1);
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [phase]);

  const isConnected = (addr: string): boolean => {
    if (!focus) return true;
    if (addr === focus) return true;
    return edges.some(
      (e) =>
        (e.debtorStr === focus && e.creditorStr === addr) ||
        (e.creditorStr === focus && e.debtorStr === addr),
    );
  };

  return (
    <svg
      viewBox={`0 0 ${W} ${H}`}
      preserveAspectRatio="xMidYMid meet"
      width="100%"
      height="100%"
      style={{ display: 'block', position: 'absolute', inset: 0 }}
      onClick={() => onSelect(null)}
    >
      <defs>
        <marker id="arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
          <path d="M0,0 L10,5 L0,10 z" fill="var(--ink-3)" />
        </marker>
        <marker id="arrow-ring" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="7.5" markerHeight="7.5" orient="auto-start-reverse">
          <path d="M0,0 L10,5 L0,10 z" fill="var(--amber)" />
        </marker>
        <filter id="glow" x="-60%" y="-60%" width="220%" height="220%">
          <feGaussianBlur stdDeviation="4" result="b" />
          <feMerge>
            <feMergeNode in="b" />
            <feMergeNode in="SourceGraphic" />
          </feMerge>
        </filter>
        <radialGradient id="nodefill" cx="35%" cy="30%">
          <stop offset="0%" stopColor="#1b2029" />
          <stop offset="100%" stopColor="#0b0d12" />
        </radialGradient>
      </defs>

      {/* faint grid so the plane reads as a terminal, not a void */}
      <g opacity={0.5}>
        {Array.from({ length: 11 }, (_, i) => (
          <line key={'v' + i} x1={(i * W) / 10} y1={0} x2={(i * W) / 10} y2={H} stroke="rgba(255,255,255,0.022)" />
        ))}
        {Array.from({ length: 8 }, (_, i) => (
          <line key={'h' + i} x1={0} y1={(i * H) / 7} x2={W} y2={(i * H) / 7} stroke="rgba(255,255,255,0.022)" />
        ))}
      </g>

      {/* EDGES */}
      {geoms.map((g) => {
        const inRing = ringSet.has(g.key);
        const dim = (ringActive && !inRing) || (!!focus && !(g.debtor === focus || g.creditor === focus));
        const shrink = phase === 'clearing' && inRing ? clearProgress : phase === 'done' && inRing ? 1 : 0;
        const shownAmount = inRing ? g.amount - BigInt(Math.round(Number(minAmount) * shrink)) : g.amount;
        const w = Math.max(0.4, edgeWidth(shownAmount < 0n ? 0n : shownAmount));
        const d = `M${g.p0[0]},${g.p0[1]} Q${g.cp[0]},${g.cp[1]} ${g.p1[0]},${g.p1[1]}`;
        return (
          <g key={g.key} opacity={dim ? 0.12 : 1} style={{ transition: 'opacity 0.25s ease' }}>
            <path
              d={d}
              fill="none"
              stroke={inRing && ringActive ? 'var(--amber)' : 'var(--ink-4)'}
              strokeWidth={inRing && ringActive ? w + 0.6 : w}
              markerEnd={inRing && ringActive ? 'url(#arrow-ring)' : 'url(#arrow)'}
              strokeLinecap="round"
              filter={inRing && ringActive ? 'url(#glow)' : undefined}
              style={{ transition: 'stroke-width 0.15s linear' }}
            />
            {inRing && ringActive && (
              <path
                d={d}
                fill="none"
                stroke="var(--amber-hi)"
                strokeWidth={1}
                strokeDasharray="2 10"
                opacity={0.9}
                style={{ animation: 'dashflow 0.7s linear infinite' }}
              />
            )}
          </g>
        );
      })}

      {/* PARTICLE BEADS while clearing */}
      {phase === 'clearing' &&
        geoms
          .filter((g) => ringSet.has(g.key))
          .flatMap((g) =>
            [0, 0.5].map((off) => {
              const t = (beadT + off) % 1;
              const [bx, by] = bezier(g.p0, g.cp, g.p1, t);
              return <circle key={g.key + off} cx={bx} cy={by} r={2.6} fill="var(--amber-hi)" filter="url(#glow)" />;
            }),
          )}

      {/* NODES */}
      {merchants.map((m, i) => {
        const p = positions.get(m.address);
        if (!p) return null;
        const r = nodeRadius(m);
        const col = healthColor(m.health, m.solvent);
        const onRing = ringNodes.has(m.address);
        const dim = (ringActive && !onRing) || (!!focus && !isConnected(m.address));
        const sel = selected === m.address;
        // ring nodes heal on clear: nudge stroke toward green as progress runs
        const healBoost = phase === 'clearing' && onRing ? clearProgress : phase === 'done' && onRing ? 1 : 0;
        const stroke = healBoost > 0 ? mix(col, '#2fe6a0', healBoost * 0.7) : col;
        return (
          <motion.g
            key={m.address}
            initial={reveal ? { opacity: 0, scale: 0.4 } : false}
            animate={{ opacity: dim ? 0.22 : 1, scale: sel || (onRing && ringActive) ? 1.08 : 1 }}
            transition={{
              opacity: { delay: reveal ? 0.15 + i * 0.04 : 0, duration: 0.5 },
              scale: { type: 'spring', stiffness: 220, damping: 18, delay: reveal ? 0.15 + i * 0.04 : 0 },
            }}
            style={{ transformOrigin: `${p.x}px ${p.y}px`, cursor: 'pointer' }}
            onMouseEnter={() => setHover(m.address)}
            onMouseLeave={() => setHover(null)}
            onClick={(e) => {
              e.stopPropagation();
              onSelect(sel ? null : m.address);
            }}
          >
            {(onRing && ringActive) || sel ? (
              <circle cx={p.x} cy={p.y} r={r + 9} fill="none" stroke={stroke} strokeWidth={1} opacity={0.4}>
                {onRing && ringActive && (
                  <animate attributeName="r" values={`${r + 6};${r + 12};${r + 6}`} dur="1.6s" repeatCount="indefinite" />
                )}
              </circle>
            ) : null}
            <circle cx={p.x} cy={p.y} r={r} fill="url(#nodefill)" stroke={stroke} strokeWidth={sel ? 2.4 : 1.6} filter={onRing && ringActive ? 'url(#glow)' : undefined} />
            {!m.solvent && m.required > 0n && (
              <circle cx={p.x} cy={p.y} r={r} fill="none" stroke="var(--red)" strokeWidth={1} strokeDasharray="3 4" opacity={0.9}>
                <animateTransform attributeName="transform" type="rotate" from={`0 ${p.x} ${p.y}`} to={`360 ${p.x} ${p.y}`} dur="9s" repeatCount="indefinite" />
              </circle>
            )}
            <text x={p.x} y={p.y + Math.max(3, r * 0.16)} textAnchor="middle" fontFamily="var(--font-mono)" fontSize={Math.max(8, Math.min(11, r * 0.42))} fontWeight={600} fill={stroke} style={{ pointerEvents: 'none' }}>
              {usdCompact(m.collateral)}
            </text>
            <text
              x={p.x}
              y={p.y + r + 15}
              textAnchor="middle"
              fontFamily="var(--font-display)"
              fontSize={11}
              fontWeight={700}
              fill="var(--ink)"
              paintOrder="stroke"
              stroke="var(--bg)"
              strokeWidth={4}
              strokeLinejoin="round"
              style={{ pointerEvents: 'none' }}
            >
              {displayName(m).label}
            </text>
            {(hover === m.address || sel) && (
              <text
                x={p.x}
                y={p.y + r + 26}
                textAnchor="middle"
                fontFamily="var(--font-mono)"
                fontSize={8}
                fill="var(--ink-3)"
                paintOrder="stroke"
                stroke="var(--bg)"
                strokeWidth={3.5}
                strokeLinejoin="round"
                style={{ pointerEvents: 'none' }}
              >
                {shortAddr(m.address, 4)}
              </text>
            )}
          </motion.g>
        );
      })}
    </svg>
  );
}

// mix two hex/hsl-ish colours in sRGB — kept tiny; inputs are our own tokens
function mix(a: string, b: string, t: number): string {
  const pa = parseColor(a);
  const pb = parseColor(b);
  if (!pa || !pb) return a;
  const r = Math.round(pa[0] + (pb[0] - pa[0]) * t);
  const g = Math.round(pa[1] + (pb[1] - pa[1]) * t);
  const bl = Math.round(pa[2] + (pb[2] - pa[2]) * t);
  return `rgb(${r},${g},${bl})`;
}
function parseColor(c: string): [number, number, number] | null {
  if (c.startsWith('#')) {
    const h = c.slice(1);
    return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
  }
  const m = c.match(/hsl\(\s*([\d.]+)\s+([\d.]+)%\s+([\d.]+)%/);
  if (m) return hslToRgb(+m[1], +m[2], +m[3]);
  return null;
}
function hslToRgb(h: number, s: number, l: number): [number, number, number] {
  s /= 100;
  l /= 100;
  const k = (n: number) => (n + h / 30) % 12;
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
  return [Math.round(f(0) * 255), Math.round(f(8) * 255), Math.round(f(4) * 255)];
}
