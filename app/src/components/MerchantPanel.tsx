import { useMemo } from 'react';
import type { MerchantView, Network } from '../lib/obligo.ts';
import { bps, healthColor, healthLabel, pct, shortAddr, usd } from '../lib/format.ts';
import { displayName } from '../lib/names.ts';
import { acctUrl } from '../lib/tx.ts';

interface Props {
  net: Network;
  selected: string | null;
  onSelect: (a: string | null) => void;
}

export default function MerchantPanel({ net, selected, onSelect }: Props) {
  const sel = useMemo(
    () => net.merchants.find((m) => m.address === selected) ?? null,
    [net.merchants, selected],
  );

  return (
    <div className="col" style={{ height: '100%' }}>
      <div className="card-head">
        <div className="eyebrow">Merchant registry</div>
        <span className="tag">{net.merchants.length} registered</span>
      </div>

      {sel && <Detail m={sel} onClose={() => onSelect(null)} />}

      <div style={{ overflowY: 'auto', flex: 1 }}>
        {net.merchants.map((m) => (
          <Row key={m.address} m={m} sel={m.address === selected} onSelect={onSelect} />
        ))}
      </div>
    </div>
  );
}

function Row({ m, sel, onSelect }: { m: MerchantView; sel: boolean; onSelect: (a: string) => void }) {
  const col = healthColor(m.health, m.solvent);
  return (
    <div className={`mrow${sel ? ' sel' : ''}`} onClick={() => onSelect(m.address)}>
      <div className="row gap-8" style={{ minWidth: 0 }}>
        <span className="dot" style={{ background: col }} />
        <span style={{ fontFamily: 'var(--font-display)', fontWeight: 700, fontSize: 13 }}>
          {displayName(m).label}
        </span>
        <span className="mono" style={{ fontSize: 9, color: 'var(--ink-4)' }}>{shortAddr(m.address, 4)}</span>
      </div>
      <div className="hchip" style={{ color: col, justifySelf: 'end' }}>{healthLabel(m.health, m.solvent)}</div>
      <div className="mono" style={{ fontSize: 10.5, color: 'var(--ink-3)' }}>
        coll <span style={{ color: 'var(--ink)' }}>{usd(m.collateral)}</span>
      </div>
      <div className="mono" style={{ fontSize: 10.5, color: 'var(--ink-3)', justifySelf: 'end' }}>
        {m.obligationsOut > 0n && <span style={{ color: 'var(--red-hi)' }}>owes {usd(m.obligationsOut)}</span>}
        {m.obligationsOut > 0n && m.obligationsIn > 0n && <span> · </span>}
        {m.obligationsIn > 0n && <span style={{ color: 'var(--green-hi)' }}>owed {usd(m.obligationsIn)}</span>}
        {m.obligationsOut === 0n && m.obligationsIn === 0n && <span>flat</span>}
      </div>
    </div>
  );
}

function Detail({ m, onClose }: { m: MerchantView; onClose: () => void }) {
  const col = healthColor(m.health, m.solvent);
  return (
    <div className="panel" style={{ margin: 12, padding: 14, borderColor: 'var(--line-2)' }}>
      <div className="row spread center" style={{ marginBottom: 10 }}>
        <div className="row gap-8 center">
          <span className="dot" style={{ background: col, width: 9, height: 9 }} />
          <span style={{ fontFamily: 'var(--font-display)', fontWeight: 800, fontSize: 16 }}>{displayName(m).label}</span>
          <span className="hchip" style={{ color: col }}>{healthLabel(m.health, m.solvent)}</span>
        </div>
        <button className="btn btn-ghost" style={{ padding: '3px 8px' }} onClick={onClose}>✕</button>
      </div>
      <a className="mono" href={acctUrl(m.address)} target="_blank" rel="noreferrer" style={{ fontSize: 10, color: 'var(--ink-3)' }}>
        {m.address} ↗
      </a>
      <div style={{ marginTop: 10 }}>
        <div className="kv"><span className="k">health</span><span className="v" style={{ color: col }}>{pct(m.health)}</span></div>
        <div className="kv"><span className="k">collateral</span><span className="v">{usd(m.collateral)}</span></div>
        <div className="kv"><span className="k">required</span><span className="v">{usd(m.required)}</span></div>
        <div className="kv"><span className="k">obligations out</span><span className="v" style={{ color: m.obligationsOut > 0n ? 'var(--red-hi)' : undefined }}>{usd(m.obligationsOut)}</span></div>
        <div className="kv"><span className="k">obligations in</span><span className="v" style={{ color: m.obligationsIn > 0n ? 'var(--green-hi)' : undefined }}>{usd(m.obligationsIn)}</span></div>
        <div className="kv"><span className="k">points out</span><span className="v">{m.pointsOutstanding.toString()}</span></div>
        <div className="kv"><span className="k">terms</span><span className="v">{usd(m.usdcPerPoint)}/pt · {bps(m.reserveBps)} reserve</span></div>
        <div className="kv"><span className="k">lifetime</span><span className="v">{m.totalIssued.toString()} iss · {m.totalRedeemed.toString()} red · {m.totalExpired.toString()} exp</span></div>
        {m.defaults > 0 && (
          <div className="kv"><span className="k">defaults</span><span className="v" style={{ color: 'var(--red-hi)' }}>{m.defaults}× on record</span></div>
        )}
      </div>
    </div>
  );
}
