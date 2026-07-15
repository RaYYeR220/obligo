import { useState } from 'react';
import { useWallet as useAdapterWallet } from '@solana/wallet-adapter-react';
import { useWalletModal } from '@solana/wallet-adapter-react-ui';
import { useSigner } from '../lib/wallet.tsx';
import { shortAddr } from '../lib/format.ts';
import { acctUrl } from '../lib/tx.ts';

export default function KeyBar() {
  const s = useSigner();
  const adapter = useAdapterWallet();
  const { setVisible } = useWalletModal();
  const [open, setOpen] = useState(false); // browser-wallet popover
  const [devOpen, setDevOpen] = useState(false); // dev-key fallback popover
  const [raw, setRaw] = useState('');

  const lowSol = s.sol !== null && s.sol < 0.02;
  const onWallet = s.kind === 'wallet';
  const onDev = s.kind === 'devkey';
  const devAddr = s.devkey ? s.devkey.publicKey.toBase58() : null;

  const openWallet = () => { setOpen((o) => !o); setDevOpen(false); };
  const openDev = () => { setDevOpen((o) => !o); setOpen(false); };

  return (
    <div className="nav" style={{ position: 'relative' }}>
      {/* PRIMARY — browser wallet */}
      {onWallet ? (
        <button className="active" onClick={openWallet} title="browser wallet">
          <span className="dot" style={{ background: lowSol ? 'var(--amber)' : 'var(--green)' }} />
          <span className="mono">{shortAddr(s.address!, 4)}</span>
          <span className="mono dim">{s.sol === null ? '· …' : `· ${s.sol.toFixed(3)}◎`}</span>
        </button>
      ) : (
        <button className="connect-cta" onClick={() => setVisible(true)} title="connect Phantom, Solflare or Backpack">
          <span className="dot" style={{ background: 'var(--amber)' }} />
          Connect Wallet
        </button>
      )}

      {/* SECONDARY — dev key / burner fallback */}
      <button
        className={onDev ? 'active' : ''}
        onClick={openDev}
        title="dev key / burner — fallback signer"
        style={{ opacity: onWallet ? 0.62 : 1 }}
      >
        <span
          className="dot"
          style={{ background: onDev ? (lowSol ? 'var(--amber)' : 'var(--green)') : devAddr ? 'var(--ink-3)' : 'var(--ink-4)' }}
        />
        {onDev ? (
          <>
            <span className="mono">{shortAddr(devAddr!, 4)}</span>
            <span className="mono dim">{s.sol === null ? '· …' : `· ${s.sol.toFixed(3)}◎`}</span>
          </>
        ) : (
          <span className="dim">{devAddr ? shortAddr(devAddr, 4) : 'dev key'}</span>
        )}
      </button>

      {/* browser-wallet popover */}
      {open && onWallet && (
        <div className="panel popover">
          <div className="eyebrow" style={{ marginBottom: 10 }}>{s.walletName ?? 'Browser wallet'}</div>
          <div className="kv"><span className="k">address</span>
            <a className="v" href={acctUrl(s.address!)} target="_blank" rel="noreferrer" style={{ color: 'var(--amber)' }}>{shortAddr(s.address!, 6)} ↗</a>
          </div>
          <div className="kv"><span className="k">sol</span><span className="v">{s.sol === null ? '…' : s.sol.toFixed(4)} ◎</span></div>
          {lowSol && (
            <div className="banner warn" style={{ marginTop: 12 }}>
              Low SOL. Fund this wallet on devnet to sign writes:
              <br />faucet.solana.com
            </div>
          )}
          <div className="row gap-8" style={{ marginTop: 14 }}>
            <button className="btn grow" onClick={() => { void s.refresh(); }}>refresh</button>
            <button className="btn btn-ghost grow" onClick={() => { void adapter.disconnect(); setOpen(false); }}>disconnect</button>
          </div>
        </div>
      )}

      {/* dev-key fallback popover */}
      {devOpen && (
        <div className="panel popover">
          <div className="row spread center" style={{ marginBottom: 10 }}>
            <div className="eyebrow">Dev key · fallback</div>
            {!onWallet && !devAddr && (
              <button className="btn btn-ghost" style={{ minHeight: 26, padding: '4px 10px' }} onClick={() => { setDevOpen(false); setVisible(true); }}>use a wallet</button>
            )}
          </div>

          {onWallet && (
            <div className="banner" style={{ marginBottom: 12, color: 'var(--ink-3)' }}>
              A browser wallet is signing. This dev key is a dormant fallback — disconnect the wallet to use it.
            </div>
          )}

          {devAddr ? (
            <>
              <div className="kv"><span className="k">address</span>
                <a className="v" href={acctUrl(devAddr)} target="_blank" rel="noreferrer" style={{ color: 'var(--amber)' }}>{shortAddr(devAddr, 6)} ↗</a>
              </div>
              {onDev && <div className="kv"><span className="k">sol</span><span className="v">{s.sol === null ? '…' : s.sol.toFixed(4)} ◎</span></div>}
              {onDev && lowSol && (
                <div className="banner warn" style={{ marginTop: 12 }}>
                  Low SOL. Fund this address on devnet to sign writes:
                  <br />faucet.solana.com or `solana airdrop 1 {shortAddr(devAddr, 4)} --url devnet`
                </div>
              )}
              <div className="row gap-8" style={{ marginTop: 14 }}>
                {onDev && <button className="btn grow" onClick={() => { void s.refresh(); }}>refresh</button>}
                <button className="btn btn-ghost grow" onClick={() => { s.clear(); setDevOpen(false); }}>remove</button>
              </div>
            </>
          ) : (
            <>
              <p className="mono dim" style={{ fontSize: 11, lineHeight: 1.6, marginTop: 0 }}>
                No extension? Sign with a throwaway devnet key instead. Paste a secret key (base58 or a
                <code> [64]</code> byte array) or mint a burner and fund it. Stored only in this browser.
              </p>
              <div className="field">
                <label>secret key</label>
                <input value={raw} onChange={(e) => setRaw(e.target.value)} placeholder="base58 or [1,2,3,…]" />
              </div>
              {s.error && <div className="banner err" style={{ marginBottom: 10 }}>{s.error}</div>}
              <div className="row gap-8">
                <button className="btn btn-amber grow" disabled={!raw.trim()} onClick={() => { s.importKey(raw); setRaw(''); }}>import</button>
                <button className="btn grow" onClick={() => { s.generate(); }}>mint burner</button>
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
