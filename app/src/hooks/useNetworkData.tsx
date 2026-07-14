import { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { ObligoClient } from '@obligo/sdk';
import { loadNetwork, makeClient, type Network } from '../lib/obligo.ts';

interface NetCtx {
  client: ObligoClient;
  net: Network | null;
  loading: boolean;
  error: string | null;
  reload: () => Promise<void>;
  lastRefreshed: number;
}

const Ctx = createContext<NetCtx | null>(null);

export function NetworkProvider({ children }: { children: React.ReactNode }) {
  const clientRef = useRef<ObligoClient>(makeClient());
  const [net, setNet] = useState<Network | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastRefreshed, setLast] = useState(0);

  const reload = useCallback(async () => {
    try {
      setError(null);
      const n = await loadNetwork(clientRef.current);
      // learn the settlement mint / hook from chain the first time we have it
      if (n.protocol) {
        clientRef.current.usdcMint = n.protocol.usdcMint;
        clientRef.current.hookId = n.protocol.hookProgram;
      }
      setNet(n);
      setLast(Date.now());
    } catch (e) {
      setError((e as Error).message || 'failed to read devnet');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
    const t = setInterval(reload, 20_000);
    return () => clearInterval(t);
  }, [reload]);

  return (
    <Ctx.Provider
      value={{ client: clientRef.current, net, loading, error, reload, lastRefreshed }}
    >
      {children}
    </Ctx.Provider>
  );
}

export function useNet(): NetCtx {
  const c = useContext(Ctx);
  if (!c) throw new Error('useNet outside provider');
  return c;
}
