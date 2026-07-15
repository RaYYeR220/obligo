import { useEffect, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { findClearableCycle, type ClearableCycle } from '@obligo/sdk';
import Graph, { type ClearPhase } from '../components/Graph.tsx';
import MoneyShot from '../components/MoneyShot.tsx';
import MerchantPanel from '../components/MerchantPanel.tsx';
import { useNet } from '../hooks/useNetworkData.tsx';
import { useWallet } from '../lib/wallet.tsx';
import { edgesForCycleFinder } from '../lib/obligo.ts';
import { humaniseError, send } from '../lib/tx.ts';
import { usd } from '../lib/format.ts';
import { displayName } from '../lib/names.ts';

export default function Network() {
  const { net, client, loading, error, reload } = useNet();
  const wallet = useWallet();
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
  const revealed = useRef(false);

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
    if (!cycle || !wallet.keypair) return;
    setErr(null);
    setSending(true);
    try {
      const ix = client.clearCycle({ cranker: wallet.keypair.publicKey, cycle });
      const sig = await send(client, [ix], [wallet.keypair]);
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
  const canRun = !!wallet.keypair && (wallet.sol === null || wallet.sol > 0.001);

  return (
    <div className="split">
      <div className="left">
        {net && (
          <Graph
            merchants={net.merchants}
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
                  <button className="btn btn-amber grow" disabled={!canRun || sending} onClick={runClear} title={canRun ? '' : 'import a funded devnet key to sign'}>
                    {sending ? 'confirming…' : 'run clear_cycle'}
                  </button>
                  <button className="btn grow" disabled={sending} onClick={runPreview}>preview</button>
                  <button className="btn btn-ghost" disabled={sending} onClick={dismiss}>✕</button>
                </div>
                {!canRun && (
                  <div className="mono" style={{ fontSize: 10.5, marginTop: 8, color: 'var(--ink-2)' }}>
                    {wallet.keypair ? 'this key needs devnet SOL to sign' : 'reads only — import a funded dev key to run it for real'}
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
