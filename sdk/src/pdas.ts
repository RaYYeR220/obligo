import { PublicKey } from '@solana/web3.js';
import { getAssociatedTokenAddressSync } from '@solana/spl-token';
import {
  OBLIGO_PROGRAM_ID,
  OBLIGO_HOOK_ID,
  SEEDS,
  TOKEN_2022_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from './constants.ts';

const pda = (seeds: (Buffer | Uint8Array)[], programId: PublicKey): PublicKey =>
  PublicKey.findProgramAddressSync(seeds, programId)[0];

/** The global config singleton, `[b"protocol"]`. */
export const protocolPda = (programId = OBLIGO_PROGRAM_ID): PublicKey =>
  pda([SEEDS.protocol], programId);

/** The program's signer PDA, `[b"authority"]` — the only account the hook grants a permit to. */
export const authorityPda = (programId = OBLIGO_PROGRAM_ID): PublicKey =>
  pda([SEEDS.authority], programId);

/** A merchant, `[b"merchant", authority]`. */
export const merchantPda = (
  authority: PublicKey,
  programId = OBLIGO_PROGRAM_ID,
): PublicKey => pda([SEEDS.merchant, authority.toBuffer()], programId);

/** A merchant's USDC collateral vault, `[b"vault", merchant]`. */
export const vaultPda = (
  merchant: PublicKey,
  programId = OBLIGO_PROGRAM_ID,
): PublicKey => pda([SEEDS.vault, merchant.toBuffer()], programId);

/** A merchant's Token-2022 points mint, `[b"points", merchant]`. */
export const pointsMintPda = (
  merchant: PublicKey,
  programId = OBLIGO_PROGRAM_ID,
): PublicKey => pda([SEEDS.points, merchant.toBuffer()], programId);

/** A customer's point batch with one merchant, `[b"batch", merchant, customer]`. */
export const batchPda = (
  merchant: PublicKey,
  customer: PublicKey,
  programId = OBLIGO_PROGRAM_ID,
): PublicKey =>
  pda([SEEDS.batch, merchant.toBuffer(), customer.toBuffer()], programId);

/** An acceptor's standing bid for an issuer's points, `[b"offer", acceptor, issuer]`. */
export const offerPda = (
  acceptor: PublicKey,
  issuer: PublicKey,
  programId = OBLIGO_PROGRAM_ID,
): PublicKey =>
  pda([SEEDS.offer, acceptor.toBuffer(), issuer.toBuffer()], programId);

/** One directed edge of the debt graph, `[b"obligation", debtor, creditor]`. */
export const obligationPda = (
  debtor: PublicKey,
  creditor: PublicKey,
  programId = OBLIGO_PROGRAM_ID,
): PublicKey =>
  pda([SEEDS.obligation, debtor.toBuffer(), creditor.toBuffer()], programId);

/** A mint's ExtraAccountMetaList, under the *hook* program. */
export const eamlPda = (
  mint: PublicKey,
  hookId = OBLIGO_HOOK_ID,
): PublicKey => pda([SEEDS.extraAccountMetas, mint.toBuffer()], hookId);

/** The hook's one-shot permit for a source token account, under the *hook* program. */
export const permitPda = (
  sourceTokenAccount: PublicKey,
  hookId = OBLIGO_HOOK_ID,
): PublicKey => pda([SEEDS.permit, sourceTokenAccount.toBuffer()], hookId);

/**
 * A points ATA. Points are Token-2022, so the token program is always `TOKEN_2022_PROGRAM_ID`.
 * Pass `allowOwnerOffCurve = true` for a merchant PDA owner (e.g. the redemption escrow).
 */
export const pointsAta = (
  mint: PublicKey,
  owner: PublicKey,
  allowOwnerOffCurve = false,
): PublicKey =>
  getAssociatedTokenAddressSync(
    mint,
    owner,
    allowOwnerOffCurve,
    TOKEN_2022_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );

/** The redemption escrow for an issuer: its own points ATA, owned by the merchant PDA (off-curve). */
export const redemptionEscrow = (
  issuerMerchant: PublicKey,
  pointsMint: PublicKey,
): PublicKey => pointsAta(pointsMint, issuerMerchant, true);
