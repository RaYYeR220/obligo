import { useState } from 'react';
import { useWallet } from '../lib/wallet.tsx';
import { shortAddr } from '../lib/format.ts';
import { acctUrl } from '../lib/tx.ts';

export default function KeyBar() {
  const w = useWallet();
  const [open, setOpen] = useState(false);
  const [raw, setRaw] = useState('');

  const lowSol = w.sol !== null && w.sol < 0.02;

  return (
    <div className="nav" style={{ position: 'relative' }}>
      <button
        className={w.keypair ? 'active' : ''}
        onClick={() => setOpen((o) => !o)}
        title="dev signing key"
      >
        <span
          className="dot"
          style={{ background: w.keypair ? (lowSol ? 'var(--amber)' : 'var(--green)') : 'var(--ink-4)' }}
        />
        {w.keypair ? (
          <span className="mono">{shortAddr(w.address!, 4)}</span>
        ) : (
          <span>DEV KEY</span>
        )}
        {w.keypair && (
          <span className="mono dim">{w.sol === null ? '· …' : `· ${w.sol.toFixed(3)}◎`}</span>
        )}
      </button>

      {open && (
        <div className="panel popover">
          <div className="eyebrow" style={{ marginBottom: 10 }}>Dev signing key</div>
          {w.keypair ? (
            <>
              <div className="kv"><span className="k">address</span>
                <a className="v" href={acctUrl(w.address!)} target="_blank" rel="noreferrer" style={{ color: 'var(--amber)' }}>{shortAddr(w.address!, 6)} ↗</a>
              </div>
              <div className="kv"><span className="k">sol</span><span className="v">{w.sol === null ? '…' : w.sol.toFixed(4)} ◎</span></div>
              {lowSol && (
                <div className="banner warn" style={{ marginTop: 12 }}>
                  Low SOL. Fund this address on devnet to sign writes:
                  <br />faucet.solana.com or `solana airdrop 1 {shortAddr(w.address!, 4)} --url devnet`
                </div>
              )}
              <div className="row gap-8" style={{ marginTop: 14 }}>
                <button className="btn grow" onClick={() => { void w.refresh(); }}>refresh</button>
                <button className="btn btn-ghost grow" onClick={() => { w.clear(); setOpen(false); }}>remove</button>
              </div>
            </>
          ) : (
            <>
              <p className="mono dim" style={{ fontSize: 11, lineHeight: 1.6, marginTop: 0 }}>
                Reads need no key. To sign writes, paste a devnet secret key (base58 or a
                <code> [64]</code> byte array) or mint a burner and fund it. Stored only in this
                browser. Use a throwaway key.
              </p>
              <div className="field">
                <label>secret key</label>
                <input value={raw} onChange={(e) => setRaw(e.target.value)} placeholder="base58 or [1,2,3,…]" />
              </div>
              {w.error && <div className="banner err" style={{ marginBottom: 10 }}>{w.error}</div>}
              <div className="row gap-8">
                <button className="btn btn-amber grow" disabled={!raw.trim()} onClick={() => { w.importKey(raw); setRaw(''); }}>import</button>
                <button className="btn grow" onClick={() => { w.generate(); }}>mint burner</button>
              </div>
              <div className="banner" style={{ marginTop: 12, color: 'var(--ink-3)' }}>
                A burner needs devnet SOL before it can sign. Get some free at faucet.solana.com.
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
