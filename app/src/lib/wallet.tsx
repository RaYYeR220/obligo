import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import {
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  type Transaction,
  type TransactionInstruction,
} from '@solana/web3.js';
import {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from '@solana/spl-token';
import { useWallet as useAdapterWallet } from '@solana/wallet-adapter-react';
import type { ObligoClient } from '@obligo/sdk';
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

/** Which signer is actually backing writes right now. A connected browser wallet always wins; the
 *  local dev key / burner is the clearly-secondary fallback. */
export type SignerKind = 'wallet' | 'devkey';

export interface Signer {
  /** The active fee payer / authority. Null when nothing is connected. */
  publicKey: PublicKey | null;
  address: string | null;
  sol: number | null;
  connected: boolean;
  kind: SignerKind | null;
  /** Display name of the connected browser wallet (Phantom / Solflare / …), when `kind === 'wallet'`. */
  walletName: string | null;

  /**
   * Build, sign and send `ixs` through the active signer, reusing the SDK's devnet backoff. The
   * active pubkey is the fee payer; `extraSigners` (ancillary keypairs — e.g. a freshly-minted
   * points mint) are partial-signed before the wallet / dev key signs.
   */
  signAndSend: (
    client: ObligoClient,
    ixs: TransactionInstruction[],
    extraSigners?: Keypair[],
    computeUnits?: number,
  ) => Promise<string>;

  refresh: () => Promise<void>;

  // ---- dev-key fallback (secondary) ----
  /** The local dev key / burner, if one is loaded — dormant while a browser wallet is connected. */
  devkey: Keypair | null;
  importKey: (raw: string) => void;
  generate: () => Keypair;
  clear: () => void;
  error: string | null;
}

const Ctx = createContext<Signer | null>(null);

export function SignerProvider({ children }: { children: React.ReactNode }) {
  const adapter = useAdapterWallet();
  const [devkey, setDevkey] = useState<Keypair | null>(null);
  const [sol, setSol] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Restore a previously-imported burner (kept only in localStorage on this machine — a dev key,
  // never a real one, and the UI says so). A browser wallet, when present, takes precedence over it.
  useEffect(() => {
    try {
      const saved = localStorage.getItem(STORE_KEY);
      if (saved) setDevkey(keypairFromInput(saved));
    } catch {
      /* ignore a corrupt saved key */
    }
  }, []);

  // Browser wallet wins; the dev key is the fallback.
  const walletConnected = adapter.connected && !!adapter.publicKey;
  const kind: SignerKind | null = walletConnected ? 'wallet' : devkey ? 'devkey' : null;
  const publicKey: PublicKey | null = walletConnected
    ? adapter.publicKey
    : devkey
      ? devkey.publicKey
      : null;
  const address = publicKey ? publicKey.toBase58() : null;

  const refresh = useCallback(async () => {
    if (!publicKey) {
      setSol(null);
      return;
    }
    try {
      setSol(await fetchSol(publicKey));
    } catch {
      /* leave stale */
    }
  }, [publicKey]);

  // Poll the active pubkey's balance (re-keyed whenever the active signer changes).
  useEffect(() => {
    void refresh();
    if (!publicKey) return;
    const t = setInterval(refresh, 12_000);
    return () => clearInterval(t);
  }, [publicKey, refresh]);

  const importKey = useCallback((raw: string) => {
    try {
      const kp = keypairFromInput(raw);
      localStorage.setItem(STORE_KEY, raw.trim());
      setDevkey(kp);
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    }
  }, []);

  const generate = useCallback(() => {
    const kp = Keypair.generate();
    localStorage.setItem(STORE_KEY, JSON.stringify(Array.from(kp.secretKey)));
    setDevkey(kp);
    setError(null);
    return kp;
  }, []);

  const clear = useCallback(() => {
    localStorage.removeItem(STORE_KEY);
    setDevkey(null);
    setError(null);
  }, []);

  const signAndSend = useCallback<Signer['signAndSend']>(
    async (client, ixs, extraSigners = [], computeUnits) => {
      const opts = { priorityMicroLamports: 5000, computeUnits, extraSigners };
      // Browser wallet takes precedence when connected.
      if (adapter.connected && adapter.publicKey) {
        const signTransaction = adapter.signTransaction;
        if (!signTransaction) {
          throw new Error('this wallet cannot sign transactions in-page — try Phantom, Solflare or Backpack');
        }
        return client.sendSigned(
          ixs,
          adapter.publicKey,
          (tx: Transaction) => signTransaction(tx),
          opts,
        );
      }
      if (devkey) {
        const kp = devkey;
        return client.sendSigned(
          ixs,
          kp.publicKey,
          async (tx: Transaction) => {
            tx.partialSign(kp);
            return tx;
          },
          opts,
        );
      }
      throw new Error('no signer — connect a wallet or import a dev key');
    },
    [adapter, devkey],
  );

  const value = useMemo<Signer>(
    () => ({
      publicKey,
      address,
      sol,
      connected: kind !== null,
      kind,
      walletName: walletConnected ? (adapter.wallet?.adapter.name ?? null) : null,
      signAndSend,
      refresh,
      devkey,
      importKey,
      generate,
      clear,
      error,
    }),
    [
      publicKey,
      address,
      sol,
      kind,
      walletConnected,
      adapter.wallet,
      signAndSend,
      refresh,
      devkey,
      importKey,
      generate,
      clear,
      error,
    ],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useSigner(): Signer {
  const c = useContext(Ctx);
  if (!c) throw new Error('useSigner outside provider');
  return c;
}
