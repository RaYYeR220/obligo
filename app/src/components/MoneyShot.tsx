import { useEffect, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import type { ClearPhase } from './Graph.tsx';
import { usd } from '../lib/format.ts';
import { txUrl } from '../lib/tx.ts';

interface Props {
  phase: ClearPhase;
  extinguished: bigint;
  hops: number;
  txSig: string | null; // real signature, or null for a preview
  preview: boolean;
  onClose: () => void;
}

function useCountUp(target: number, run: boolean, ms = 1400): number {
  const [v, setV] = useState(0);
  useEffect(() => {
    if (!run) return;
    let raf = 0;
    const t0 = performance.now();
    const loop = (now: number) => {
      const t = Math.min(1, (now - t0) / ms);
      const eased = 1 - Math.pow(1 - t, 3);
      setV(target * eased);
      if (t < 1) raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [target, run, ms]);
  return v;
}

export default function MoneyShot(props: Props) {
  const { phase, extinguished, hops, txSig, preview, onClose } = props;
  const show = phase === 'clearing' || phase === 'done';
  const running = phase === 'clearing' || phase === 'done';
  const count = useCountUp(Number(extinguished), running);

  return (
    <AnimatePresence>
      {show && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.3 }}
          style={{
            position: 'absolute',
            inset: 0,
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            pointerEvents: 'none',
            zIndex: 20,
          }}
        >
          <motion.div
            initial={{ y: 18, opacity: 0 }}
            animate={{ y: 0, opacity: 1 }}
            transition={{ type: 'spring', stiffness: 200, damping: 22 }}
            className="panel"
            style={{
              pointerEvents: 'auto',
              padding: '26px 40px 30px',
              textAlign: 'center',
              background: 'rgba(10,12,16,0.86)',
              backdropFilter: 'blur(8px)',
              borderColor: 'var(--amber)',
              boxShadow: '0 0 0 1px rgba(255,176,0,0.25), 0 30px 80px -30px rgba(0,0,0,0.9)',
              minWidth: 420,
            }}
          >
            <div className="eyebrow" style={{ color: 'var(--amber)' }}>
              {preview ? 'CYCLE CLEARING · PREVIEW' : 'CYCLE CLEARED ON DEVNET'}
            </div>

            <div
              style={{
                fontFamily: 'var(--font-hero)',
                fontSize: 76,
                lineHeight: 1,
                color: 'var(--amber)',
                margin: '14px 0 2px',
                letterSpacing: '-0.01em',
                textShadow: '0 0 30px rgba(255,176,0,0.45)',
                fontVariantNumeric: 'tabular-nums',
              }}
            >
              {usd(count)}
            </div>
            <div className="eyebrow" style={{ letterSpacing: '0.28em' }}>
              obligations extinguished · {hops} hops
            </div>

            <div style={{ height: 1, background: 'var(--line)', margin: '20px 0 16px' }} />

            {phase === 'done' ? (
              <motion.div
                initial={{ scale: 1.5, opacity: 0 }}
                animate={{ scale: 1, opacity: 1 }}
                transition={{ type: 'spring', stiffness: 260, damping: 16, delay: 0.05 }}
              >
                <div
                  style={{
                    fontFamily: 'var(--font-hero)',
                    fontSize: 54,
                    lineHeight: 1,
                    color: 'var(--green)',
                    textShadow: '0 0 34px var(--green-glow)',
                    fontVariantNumeric: 'tabular-nums',
                  }}
                >
                  $0.00
                </div>
                <div className="eyebrow" style={{ color: 'var(--green)', letterSpacing: '0.3em', marginTop: 4 }}>
                  usdc moved
                </div>
              </motion.div>
            ) : (
              <div className="mono dim" style={{ fontSize: 12 }}>
                netting the ring…
              </div>
            )}

            {phase === 'done' && (
              <div style={{ marginTop: 22 }} className="col gap-8 center">
                {txSig ? (
                  <a
                    href={txUrl(txSig)}
                    target="_blank"
                    rel="noreferrer"
                    className="mono"
                    style={{ fontSize: 11, color: 'var(--amber)' }}
                  >
                    ↗ {txSig.slice(0, 8)}…{txSig.slice(-8)} · view on explorer
                  </a>
                ) : (
                  <div className="tag" style={{ borderColor: 'var(--line-2)' }}>
                    preview only · no transaction sent
                  </div>
                )}
                <button className="btn btn-ghost" onClick={onClose} style={{ marginTop: 6 }}>
                  dismiss
                </button>
              </div>
            )}
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
