import { useEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { findClearableCycle, type ClearableCycle } from '@obligo/sdk';
import Graph, { type ClearPhase } from '../components/Graph.tsx';
import MoneyShot from '../components/MoneyShot.tsx';
import MerchantPanel from '../components/MerchantPanel.tsx';
import { useNet } from '../hooks/useNetworkData.tsx';
import { useSigner } from '../lib/wallet.tsx';
import { edgesForCycleFinder } from '../lib/obligo.ts';
import { humaniseError } from '../lib/tx.ts';
import { usd } from '../lib/format.ts';
import { displayName } from '../lib/names.ts';

export default function Network() {
  const { net, client, loading, error, reload } = useNet();
  const signer = useSigner();
  const [selected, setSelected] = useState<string | null>(null);

  const [phase, setPhase] = useState<ClearPhase>('idle');
  const [cycle, setCycle] = useState<ClearableCycle | null>(null);
  const [ring, setRing] = useState<string[] | null>(null);
  const [progress, setProgress] = useState(0);
  const [txSig, setTxSig] = useState<string | null>(null);
  const [preview, setPreview] = useState(false);
  const [sending, setSending] = useState(false);
  const [scanMsg, setScanMsg] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [showAll, setShowAll] = useState(false);
  const revealed = useRef(false);

  // Merchants that are actually part of the live obligation graph (an endpoint of a live edge).
  // The network accumulates registrations over time; most sit idle at $0. The default view shows
  // only the ones with live debt, which is where every clearable cycle lives anyway — nothing is
  // hidden, the "all" toggle brings the full registry back.
  const liveAddrs = useMemo(() => {
    const s = new Set<string>();
    for (const e of net?.edges ?? []) {
      s.add(e.debtorStr);
      s.add(e.creditorStr);
    }
    return s;
  }, [net]);
  const liveMerchants = useMemo(
    () => (net ? net.merchants.filter((m) => liveAddrs.has(m.address)) : []),
    [net, liveAddrs],
  );
  const shownMerchants = !net ? [] : showAll || liveMerchants.length === 0 ? net.merchants : liveMerchants;

  useEffect(() => {
    if (net && !revealed.current) revealed.current = true;
  }, [net]);

  // drive the collapse animation while clearing
  useEffect(() => {
    if (phase !== 'clearing') return;
    let raf = 0;
    const t0 = performance.now();
    const dur = 1700;
    const loop = (now: number) => {
      const t = Math.min(1, (now - t0) / dur);
      setProgress(t);
      if (t < 1) raf = requestAnimationFrame(loop);
      else setPhase('done');
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [phase]);

  const ringNames = (c: ClearableCycle): string =>
    c.ring
      .map((pk) => net?.merchants.find((m) => m.address === pk.toBase58()))
      .map((m) => (m ? displayName(m).label : '??'))
      .join(' → ') + ' → ' + (net?.merchants.find((m) => m.address === c.ring[0].toBase58()) ? displayName(net.merchants.find((m) => m.address === c.ring[0].toBase58())!).label : '');

  function scan() {
    setErr(null);
    setScanMsg(null);
    if (!net) return;
    const c = findClearableCycle(edgesForCycleFinder(net.edges), { programId: client.programId });
    if (!c) {
      setCycle(null);
      setRing(null);
      setPhase('idle');
      setScanMsg('No clearable ring in the live graph right now — every cycle has already netted to zero. Weave one from the Merchant console, or clear a bilateral pair via settle.');
      return;
    }
    setCycle(c);
    setRing(c.ring.map((pk) => pk.toBase58()));
    setPhase('armed');
    setScanMsg(null);
  }

  async function runClear() {
    if (!cycle || !signer.connected || !signer.publicKey) return;
    setErr(null);
    setSending(true);
    try {
      const ix = client.clearCycle({ cranker: signer.publicKey, cycle });
      const sig = await signer.signAndSend(client, [ix]);
      setTxSig(sig);
      setPreview(false);
      setSending(false);
      setProgress(0);
      setPhase('clearing');
    } catch (e) {
      setSending(false);
      setErr(humaniseError(e));
    }
  }

  function runPreview() {
    if (!cycle) return;
    setTxSig(null);
    setPreview(true);
    setProgress(0);
    setPhase('clearing');
  }

  function dismiss() {
    setPhase('idle');
    setCycle(null);
    setRing(null);
    setTxSig(null);
    setProgress(0);
    setScanMsg(null);
    void reload();
  }

  const extinguished = cycle ? cycle.minAmount * BigInt(cycle.ring.length) : 0n;
  const canRun = signer.connected && (signer.sol === null || signer.sol > 0.001);

  return (
    <div className="split">
      <div className="left">
        {net && (
          <Graph
            merchants={shownMerchants}
            edges={net.edges}
            ring={ring}
            phase={phase}
            clearProgress={progress}
            minAmount={cycle?.minAmount ?? 0n}
            selected={selected}
            onSelect={setSelected}
            reveal={!revealed.current}
          />
        )}

        {/* view filter — the graph accumulates idle merchants; default to the ones with live debt */}
        {net && liveMerchants.length > 0 && liveMerchants.length < net.merchants.length && (
          <div style={{ position: 'absolute', top: 14, left: 16, zIndex: 2, display: 'flex', gap: 2, padding: 3, background: 'linear-gradient(180deg,var(--panel),var(--panel-2))', border: '1px solid var(--line)', borderRadius: 3 }}>
            {([[false, `live debt · ${liveMerchants.length}`], [true, `all · ${net.merchants.length}`]] as const).map(([val, label]) => (
              <button
                key={label}
                onClick={() => setShowAll(val)}
                className="mono"
                style={{
                  cursor: 'pointer',
                  fontSize: 10,
                  letterSpacing: '0.08em',
                  textTransform: 'uppercase',
                  padding: '5px 10px',
                  borderRadius: 2,
                  border: 0,
                  background: showAll === val ? 'var(--amber)' : 'transparent',
                  color: showAll === val ? '#120c00' : 'var(--ink-3)',
                  fontWeight: showAll === val ? 600 : 400,
                  transition: 'background 0.14s ease, color 0.14s ease',
                }}
              >
                {label}
              </button>
            ))}
          </div>
        )}

        {loading && !net && (
          <div className="row center" style={{ position: 'absolute', inset: 0, justifyContent: 'center' }}>
            <span className="mono dim">reading live devnet state…</span>
          </div>
        )}
        {error && (
          <div className="banner err" style={{ position: 'absolute', top: 16, left: 16, right: 16 }}>
            RPC error: {error}
          </div>
        )}

        {/* control deck */}
        <div style={{ position: 'absolute', left: 16, bottom: 16, width: 384, maxWidth: 'calc(100% - 32px)' }}>
          <AnimatePresence mode="wait">
            {phase === 'armed' && cycle ? (
              <motion.div
                key="armed"
                initial={{ opacity: 0, y: 14 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: 14 }}
                className="panel"
                style={{ padding: 16, borderColor: 'var(--amber)' }}
              >
                <div className="eyebrow" style={{ color: 'var(--amber)' }}>clearable ring located</div>
                <div className="mono" style={{ fontSize: 12.5, margin: '8px 0 4px', color: 'var(--ink)' }}>{ringNames(cycle)}</div>
                <div className="mono dim" style={{ fontSize: 11 }}>
                  smallest edge {usd(cycle.minAmount)} · clearing extinguishes{' '}
                  <span style={{ color: 'var(--amber)' }}>{usd(extinguished)}</span> across {cycle.ring.length} edges,
                  moving <span style={{ color: 'var(--green)' }}>$0.00</span>.
                </div>
                {err && <div className="banner err" style={{ marginTop: 10 }}>{err}</div>}
                <div className="row gap-8" style={{ marginTop: 14 }}>
                  <button className="btn btn-amber grow" disabled={!canRun || sending} onClick={runClear} title={canRun ? '' : 'connect a wallet or import a funded dev key to sign'}>
                    {sending ? 'confirming…' : 'run clear_cycle'}
                  </button>
                  <button className="btn grow" disabled={sending} onClick={runPreview}>preview</button>
                  <button className="btn btn-ghost" disabled={sending} onClick={dismiss}>✕</button>
                </div>
                {!canRun && (
                  <div className="mono" style={{ fontSize: 10.5, marginTop: 8, color: 'var(--ink-2)' }}>
                    {signer.connected ? 'this signer needs devnet SOL to sign' : 'reads only — connect a wallet or import a funded dev key to run it for real'}
                  </div>
                )}
              </motion.div>
            ) : phase === 'idle' ? (
              <motion.div key="idle" initial={{ opacity: 0, y: 14 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: 14 }} className="panel" style={{ padding: 16 }}>
                <div className="row spread center gap-12">
                  <div style={{ minWidth: 0 }}>
                    <div className="eyebrow">the money shot</div>
                    <div className="mono dim" style={{ fontSize: 11, marginTop: 6 }}>
                      Find a ring of debt and cancel it around the cycle — zero cash moves.
                    </div>
                  </div>
                  <button className="btn btn-amber" style={{ flexShrink: 0 }} onClick={scan} disabled={!net}>scan for cycle</button>
                </div>
                {scanMsg && <div className="banner warn" style={{ marginTop: 12 }}>{scanMsg}</div>}
              </motion.div>
            ) : null}
          </AnimatePresence>
        </div>

        {/* legend */}
        <div style={{ position: 'absolute', right: 16, bottom: 16 }} className="panel">
          <div style={{ padding: '10px 12px' }}>
            <div className="eyebrow" style={{ marginBottom: 8 }}>health</div>
            <div className="row gap-8 center" style={{ marginBottom: 6 }}>
              <div style={{ width: 84, height: 6, borderRadius: 3, background: 'linear-gradient(90deg,#ff4747,hsl(90 82% 58%),#2fe6a0)' }} />
            </div>
            <div className="row spread mono" style={{ fontSize: 8.5, color: 'var(--ink-3)', letterSpacing: '0.06em' }}>
              <span>INSOLVENT</span><span>COVERED</span><span>STRONG</span>
            </div>
            <div className="mono dim" style={{ fontSize: 8.5, marginTop: 8, lineHeight: 1.6 }}>
              node size = collateral<br />edge width = debt · arrow debtor→creditor
            </div>
          </div>
        </div>

        <MoneyShot phase={phase} extinguished={extinguished} hops={cycle?.ring.length ?? 0} txSig={txSig} preview={preview} onClose={dismiss} />
      </div>

      <div className="right">{net && <MerchantPanel net={net} selected={selected} onSelect={setSelected} />}</div>
    </div>
  );
}
