import { useMemo, useState } from 'react';
import { ConnectionProvider, WalletProvider as AdapterWalletProvider } from '@solana/wallet-adapter-react';
import { WalletModalProvider } from '@solana/wallet-adapter-react-ui';
import { PhantomWalletAdapter } from '@solana/wallet-adapter-phantom';
import { SolflareWalletAdapter } from '@solana/wallet-adapter-solflare';
import { SignerProvider } from './lib/wallet.tsx';
import { NetworkProvider, useNet } from './hooks/useNetworkData.tsx';
import { networkStats, RPC_DEFAULT } from './lib/obligo.ts';
import { usd, num } from './lib/format.ts';
import KeyBar from './components/KeyBar.tsx';
import Network from './surfaces/Network.tsx';
import Console from './surfaces/Console.tsx';
import Redeem from './surfaces/Redeem.tsx';

type Surface = 'network' | 'console' | 'redeem';

function Header({ surface, setSurface }: { surface: Surface; setSurface: (s: Surface) => void }) {
  const { net, lastRefreshed } = useNet();
  const s = net ? networkStats(net) : null;
  void lastRefreshed;

  return (
    <header className="topbar">
      <div className="brand">
        <span className="brand-mark">OBLIGO</span>
        <span className="col" style={{ gap: 0 }}>
          <span className="eyebrow" style={{ letterSpacing: '0.18em', fontSize: 8.5 }}>CLEARING TERMINAL</span>
          <span className="mono" style={{ fontSize: 8.5, color: 'var(--ink-4)' }}>solana devnet</span>
        </span>
      </div>

      <div className="ticker">
        <div className="tick">
          <span className="k">merchants</span>
          <span className="v">{s ? num(s.merchants) : '—'}</span>
        </div>
        <div className="tick">
          <span className="k">live debt edges</span>
          <span className="v" style={{ color: s && s.edges > 0 ? 'var(--amber)' : undefined }}>{s ? num(s.edges) : '—'}</span>
        </div>
        <div className="tick">
          <span className="k">gross obligations</span>
          <span className="v">{s ? usd(s.grossDebt) : '—'}</span>
        </div>
        <div className="tick">
          <span className="k">collateral posted</span>
          <span className="v" style={{ color: 'var(--green)' }}>{s ? usd(s.collateral) : '—'}</span>
        </div>
        <div className="tick">
          <span className="k">insolvent</span>
          <span className="v" style={{ color: s && s.insolvent > 0 ? 'var(--red)' : 'var(--ink-2)' }}>{s ? num(s.insolvent) : '—'}</span>
        </div>
        <div className="tick" style={{ borderRight: 0 }}>
          <span className="k row gap-4 center"><span className="dot live-dot" style={{ background: 'var(--green)', width: 5, height: 5 }} /> live</span>
          <span className="v mono" style={{ fontSize: 11, color: 'var(--ink-3)' }}>reading chain</span>
        </div>
      </div>

      <nav className="nav">
        <button className={surface === 'network' ? 'active' : ''} onClick={() => setSurface('network')}>Network</button>
        <button className={surface === 'console' ? 'active' : ''} onClick={() => setSurface('console')}>Console</button>
        <button className={surface === 'redeem' ? 'active' : ''} onClick={() => setSurface('redeem')}>Redeem</button>
      </nav>
      <KeyBar />
    </header>
  );
}

function Shell() {
  const [surface, setSurface] = useState<Surface>('network');
  return (
    <div className="app">
      <div className="grain" />
      <Header surface={surface} setSurface={setSurface} />
      <main className="surface">
        {surface === 'network' && <Network />}
        {surface === 'console' && <Console />}
        {surface === 'redeem' && <Redeem />}
      </main>
    </div>
  );
}

export default function App() {
  // Devnet endpoint for the wallet adapter. Reads + sends still flow through the SDK client's own
  // connection (lib/obligo.ts); this feeds the adapter's autoConnect + Standard-Wallet detection.
  const wallets = useMemo(
    () => [new PhantomWalletAdapter(), new SolflareWalletAdapter()],
    [],
  );
  return (
    <ConnectionProvider endpoint={RPC_DEFAULT}>
      <AdapterWalletProvider wallets={wallets} autoConnect>
        <WalletModalProvider>
          <SignerProvider>
            <NetworkProvider>
              <Shell />
            </NetworkProvider>
          </SignerProvider>
        </WalletModalProvider>
      </AdapterWalletProvider>
    </ConnectionProvider>
  );
}
