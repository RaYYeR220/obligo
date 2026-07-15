import { Buffer } from 'buffer';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { motion } from 'framer-motion';
import { batchPda, decodePointBatch } from '@obligo/sdk';
import { connection } from '../lib/obligo.ts';
import { useNet } from '../hooks/useNetworkData.tsx';
import { useSigner } from '../lib/wallet.tsx';
import type { MerchantView, OfferView } from '../lib/obligo.ts';
import { bps, shortAddr, usd } from '../lib/format.ts';
import { displayName } from '../lib/names.ts';
import { humaniseError, txUrl } from '../lib/tx.ts';

interface Holding {
  issuer: MerchantView;
  points: bigint;
}

export default function Redeem() {
  const { net, client, reload } = useNet();
  const signer = useSigner();
  const [holdings, setHoldings] = useState<Holding[]>([]);
  const [pick, setPick] = useState<string | null>(null);
  const [venue, setVenue] = useState<string | null>(null);
  const [amt, setAmt] = useState('');
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ sig: string; issuer: string; venue: string; face: bigint } | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const loadHoldings = useCallback(async () => {
    if (!signer.connected || !signer.publicKey || !net) {
      setHoldings([]);
      return;
    }
    const cust = signer.publicKey;
    // one batched read for every merchant's point batch with this customer — not N calls
    const pdas = net.merchants.map((m) => batchPda(m.pda, cust, client.programId));
    const found: Holding[] = [];
    for (let i = 0; i < pdas.length; i += 100) {
      const slice = net.merchants.slice(i, i + 100);
      const infos = await connection.getMultipleAccountsInfo(pdas.slice(i, i + 100));
      infos.forEach((info, j) => {
        if (!info) return;
        const b = decodePointBatch(Buffer.from(info.data));
        if (b.amount > 0n) found.push({ issuer: slice[j], points: b.amount });
      });
    }
    setHoldings(found);
  }, [signer.connected, signer.publicKey, net, client]);

  useEffect(() => {
    void loadHoldings();
  }, [loadHoldings, net?.loadedAt]);

  const selected = useMemo(() => holdings.find((h) => h.issuer.address === pick) ?? null, [holdings, pick]);

  // live offers for the picked issuer — the acceptance market, best rate first
  const offers = useMemo<OfferView[]>(() => {
    if (!selected || !net) return [];
    const now = Math.floor(Date.now() / 1000);
    return net.offers
      .filter((o) => o.issuerStr === selected.issuer.address && Number(o.expiresAt) > now && o.consumed < o.capacity)
      .sort((a, b) => b.rateBps - a.rateBps);
  }, [selected, net]);

  const venueMerchant = useMemo(
    () => net?.merchants.find((m) => m.address === venue) ?? null,
    [net, venue],
  );

  async function redeem() {
    if (!signer.connected || !signer.publicKey || !selected || !venueMerchant) return;
    setErr(null);
    setBusy(true);
    try {
      const points = Math.round(parseFloat(amt || '0'));
      const ix = client.redeem({
        payer: signer.publicKey,
        customer: signer.publicKey,
        issuerMerchant: selected.issuer.pda,
        acceptorMerchant: venueMerchant.pda,
        points,
      });
      const sig = await signer.signAndSend(client, [ix], undefined, 400_000);
      const face = BigInt(points) * selected.issuer.usdcPerPoint;
      setResult({ sig, issuer: displayName(selected.issuer).label, venue: displayName(venueMerchant).label, face });
      setAmt('');
      await loadHoldings();
      await reload();
    } catch (e) {
      setErr(humaniseError(e));
    } finally {
      setBusy(false);
    }
  }

  if (!signer.connected) {
    return (
      <div className="scroll-surface" style={{ padding: 40 }}>
        <div className="panel" style={{ padding: 32, maxWidth: 560, margin: '20px auto', textAlign: 'center' }}>
          <div className="brand-mark" style={{ fontSize: 30, marginBottom: 12 }}>▸ SPEND YOUR POINTS</div>
          <p className="mono dim" style={{ fontSize: 12.5, lineHeight: 1.7 }}>
            <b style={{ color: 'var(--amber)' }}>Connect a wallet</b> (top-right) that holds a merchant's points — or fall back to a dev key.
            Redeeming burns them at an accepting venue and books a debt from the issuer to that venue —
            <b style={{ color: 'var(--green)' }}> no USDC moves</b>. To get points, issue some to this address from the Console.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="scroll-surface" style={{ padding: '22px 26px', maxWidth: 1000, margin: '0 auto' }}>
      <div className="eyebrow">Redeem · customer</div>
      <h2 style={{ fontSize: 22, margin: '4px 0 18px' }}>Spend points · <span className="mono dim" style={{ fontSize: 12, fontWeight: 400 }}>{shortAddr(signer.address!, 5)}</span></h2>

      {result && (
        <motion.div initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} className="panel" style={{ padding: 20, marginBottom: 18, borderColor: 'var(--green)' }}>
          <div className="eyebrow" style={{ color: 'var(--green)' }}>redemption confirmed · a debt, not a payment</div>
          <div className="row spread center" style={{ marginTop: 14, flexWrap: 'wrap', gap: 14 }}>
            <div>
              <div className="mono dim" style={{ fontSize: 10 }}>OBLIGATION CREATED</div>
              <div style={{ fontFamily: 'var(--font-display)', fontWeight: 800, fontSize: 20 }}>{result.issuer} <span style={{ color: 'var(--red-hi)' }}>→</span> {result.venue}</div>
              <div className="mono" style={{ fontSize: 13, color: 'var(--red-hi)' }}>{usd(result.face)} face</div>
            </div>
            <div style={{ textAlign: 'right' }}>
              <div className="mono dim" style={{ fontSize: 10 }}>USDC MOVED</div>
              <div style={{ fontFamily: 'var(--font-hero)', fontSize: 40, color: 'var(--green)', textShadow: '0 0 24px var(--green-glow)' }}>$0.00</div>
            </div>
          </div>
          <a href={txUrl(result.sig)} target="_blank" rel="noreferrer" className="mono" style={{ fontSize: 11, color: 'var(--amber)', display: 'inline-block', marginTop: 10 }}>↗ {shortAddr(result.sig, 8)} on explorer</a>
        </motion.div>
      )}

      {holdings.length === 0 ? (
        <div className="banner warn">
          This address holds no points yet. From the <b>Console</b>, issue points to{' '}
          <span className="mono">{shortAddr(signer.address!, 4)}</span> (issuance needs the merchant to hold collateral). Then spend them here.
        </div>
      ) : (
        <div className="split" style={{ gridTemplateColumns: '340px 1fr', gap: 0, height: 'auto' }}>
          {/* holdings */}
          <div className="panel" style={{ padding: 0, marginRight: 18 }}>
            <div className="card-head"><span className="eyebrow">your points</span></div>
            {holdings.map((h) => (
              <div key={h.issuer.address} className={`mrow${pick === h.issuer.address ? ' sel' : ''}`} style={{ gridTemplateColumns: '1fr auto' }} onClick={() => { setPick(h.issuer.address); setVenue(null); }}>
                <div className="row gap-8 center">
                  <span style={{ fontFamily: 'var(--font-display)', fontWeight: 700 }}>{displayName(h.issuer).label}</span>
                  <span className="mono" style={{ fontSize: 9, color: 'var(--ink-4)' }}>{shortAddr(h.issuer.address, 4)}</span>
                </div>
                <span className="mono" style={{ fontWeight: 600 }}>{h.points.toString()} pts</span>
              </div>
            ))}
          </div>

          {/* market + redeem */}
          <div className="panel" style={{ padding: 16 }}>
            {!selected ? (
              <div className="mono dim" style={{ fontSize: 12, padding: 20, textAlign: 'center' }}>← pick a points balance to see who accepts it</div>
            ) : (
              <>
                <div className="eyebrow" style={{ color: 'var(--amber)' }}>who accepts {displayName(selected.issuer).label} points</div>
                <div className="mono dim" style={{ fontSize: 10.5, margin: '6px 0 14px' }}>
                  {selected.points.toString()} pts held · {usd(selected.issuer.usdcPerPoint)}/pt face · redemption routes to the best live bid.
                </div>
                {offers.length === 0 ? (
                  <div className="banner warn">No live acceptance offers for this issuer right now. A venue must post one from the Console.</div>
                ) : (
                  <>
                    {offers.map((o) => {
                      const vm = net?.merchants.find((m) => m.address === o.acceptorStr);
                      return (
                        <div key={o.acceptorStr} className={`mrow${venue === o.acceptorStr ? ' sel' : ''}`} style={{ gridTemplateColumns: '1fr auto auto', gap: 12 }} onClick={() => setVenue(o.acceptorStr)}>
                          <span style={{ fontFamily: 'var(--font-display)', fontWeight: 700 }}>{vm ? displayName(vm).label : shortAddr(o.acceptorStr, 4)}</span>
                          <span className="mono" style={{ color: o.rateBps >= 10000 ? 'var(--green-hi)' : 'var(--amber)' }}>{bps(o.rateBps)}</span>
                          <span className="mono dim" style={{ fontSize: 11 }}>cap {usd(o.capacity - o.consumed)}</span>
                        </div>
                      );
                    })}
                    {venue && (
                      <div style={{ marginTop: 16 }}>
                        <div className="row gap-8" style={{ alignItems: 'flex-end' }}>
                          <div className="field grow" style={{ marginBottom: 0 }}>
                            <label>points to spend</label>
                            <input value={amt} onChange={(e) => setAmt(e.target.value)} placeholder={`up to ${selected.points}`} />
                          </div>
                          <button className="btn btn-ghost" style={{ marginBottom: 0 }} onClick={() => setAmt(selected.points.toString())}>max</button>
                        </div>
                        {err && <div className="banner err" style={{ marginTop: 12 }}>{err}</div>}
                        <button className="btn btn-amber" style={{ width: '100%', marginTop: 12 }} disabled={busy || !amt}
                          onClick={redeem}>
                          {busy ? 'confirming…' : `redeem — creates a debt, moves $0`}
                        </button>
                      </div>
                    )}
                  </>
                )}
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
