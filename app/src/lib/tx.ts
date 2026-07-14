import type { Keypair, TransactionInstruction } from '@solana/web3.js';
import { ObligoClient, OBLIGO_ERRORS } from '@obligo/sdk';

export const EXPLORER = 'https://explorer.solana.com';
export const txUrl = (sig: string) => `${EXPLORER}/tx/${sig}?cluster=devnet`;
export const acctUrl = (addr: string) => `${EXPLORER}/address/${addr}?cluster=devnet`;

/** Turn an on-chain custom-error code (Anchor: 6000 + index) into a sentence. */
export function humaniseError(err: unknown): string {
  const raw = typeof err === 'string' ? err : ((err as Error)?.message ?? String(err));
  const m = raw.match(/custom program error: (0x[0-9a-fA-F]+)|"Custom":\s*(\d+)/);
  let code: number | null = null;
  if (m) code = m[1] ? parseInt(m[1], 16) : parseInt(m[2], 10);
  if (code !== null && OBLIGO_ERRORS[code]) return `${OBLIGO_ERRORS[code]} (0x${code.toString(16)})`;
  if (/insufficient (lamports|funds)/i.test(raw)) return 'not enough SOL for fees + rent';
  if (/blockhash/i.test(raw)) return 'blockhash expired — try again';
  return raw.length > 160 ? raw.slice(0, 160) + '…' : raw;
}

export interface SendResult {
  sig: string;
}

/** Sign + send with the dev keypair. First signer is the fee payer. */
export async function send(
  client: ObligoClient,
  ixs: TransactionInstruction[],
  signers: Keypair[],
  computeUnits?: number,
): Promise<string> {
  return client.sendAndConfirm(ixs, signers, {
    priorityMicroLamports: 5000,
    computeUnits,
  });
}
