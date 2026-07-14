import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import { Keypair, LAMPORTS_PER_SOL, PublicKey } from '@solana/web3.js';
import {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from '@solana/spl-token';
import bs58 from 'bs58';
import { connection } from './obligo.ts';

const STORE_KEY = 'obligo.devkey';

/** Accept either a solana-keygen JSON array `[..64]` or a base58 secret key (Phantom export). */
export function keypairFromInput(raw: string): Keypair {
  const s = raw.trim();
  if (!s) throw new Error('empty key');
  if (s.startsWith('[')) {
    const arr = JSON.parse(s) as number[];
    if (arr.length !== 64) throw new Error(`expected 64 bytes, got ${arr.length}`);
    return Keypair.fromSecretKey(Uint8Array.from(arr));
  }
  const bytes = bs58.decode(s);
  if (bytes.length === 64) return Keypair.fromSecretKey(bytes);
  if (bytes.length === 32) return Keypair.fromSeed(bytes);
  throw new Error(`unrecognised secret key length: ${bytes.length}`);
}

export async function fetchSol(pk: PublicKey): Promise<number> {
  const lamports = await connection.getBalance(pk, 'confirmed');
  return lamports / LAMPORTS_PER_SOL;
}

export async function fetchUsdc(mint: PublicKey, owner: PublicKey): Promise<bigint> {
  const ata = getAssociatedTokenAddressSync(
    mint,
    owner,
    false,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );
  try {
    const bal = await connection.getTokenAccountBalance(ata, 'confirmed');
    return BigInt(bal.value.amount);
  } catch {
    return 0n;
  }
}

interface WalletCtx {
  keypair: Keypair | null;
  address: string | null;
  sol: number | null;
  importKey: (raw: string) => void;
  generate: () => Keypair;
  clear: () => void;
  refresh: () => Promise<void>;
  error: string | null;
}

const Ctx = createContext<WalletCtx | null>(null);

export function WalletProvider({ children }: { children: React.ReactNode }) {
  const [keypair, setKeypair] = useState<Keypair | null>(null);
  const [sol, setSol] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Restore a previously-imported burner (kept only in localStorage on this machine; a dev key,
  // never a real one — the UI says so).
  useEffect(() => {
    try {
      const saved = localStorage.getItem(STORE_KEY);
      if (saved) setKeypair(keypairFromInput(saved));
    } catch {
      /* ignore a corrupt saved key */
    }
  }, []);

  const refresh = useCallback(async () => {
    if (!keypair) {
      setSol(null);
      return;
    }
    try {
      setSol(await fetchSol(keypair.publicKey));
    } catch {
      /* leave stale */
    }
  }, [keypair]);

  useEffect(() => {
    void refresh();
    if (!keypair) return;
    const t = setInterval(refresh, 12_000);
    return () => clearInterval(t);
  }, [keypair, refresh]);

  const importKey = useCallback((raw: string) => {
    try {
      const kp = keypairFromInput(raw);
      localStorage.setItem(STORE_KEY, raw.trim());
      setKeypair(kp);
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    }
  }, []);

  const generate = useCallback(() => {
    const kp = Keypair.generate();
    localStorage.setItem(STORE_KEY, JSON.stringify(Array.from(kp.secretKey)));
    setKeypair(kp);
    setError(null);
    return kp;
  }, []);

  const clear = useCallback(() => {
    localStorage.removeItem(STORE_KEY);
    setKeypair(null);
    setSol(null);
  }, []);

  const value = useMemo<WalletCtx>(
    () => ({
      keypair,
      address: keypair?.publicKey.toBase58() ?? null,
      sol,
      importKey,
      generate,
      clear,
      refresh,
      error,
    }),
    [keypair, sol, importKey, generate, clear, refresh, error],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useWallet(): WalletCtx {
  const c = useContext(Ctx);
  if (!c) throw new Error('useWallet outside provider');
  return c;
}
