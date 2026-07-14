import { PublicKey, SystemProgram } from '@solana/web3.js';
import {
  TOKEN_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from '@solana/spl-token';

/** The deployed Obligo core program on devnet. */
export const OBLIGO_PROGRAM_ID = new PublicKey(
  '3YV8p2MhPQpSZS8fp3D7VoQYPy8GwxV221koWpsptRpN',
);

/** The transfer-hook program every points mint is permanently bound to. */
export const OBLIGO_HOOK_ID = new PublicKey(
  'AtDpNdzKVRxMwK5bTotfmjxQdVU854RopJccgYRP8wQ7',
);

export {
  TOKEN_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  SystemProgram,
};

export const SYSTEM_PROGRAM_ID = SystemProgram.programId;

/** PDA seeds, byte-for-byte the ones the on-chain program uses. */
export const SEEDS = {
  protocol: Buffer.from('protocol'),
  authority: Buffer.from('authority'),
  merchant: Buffer.from('merchant'),
  vault: Buffer.from('vault'),
  points: Buffer.from('points'),
  batch: Buffer.from('batch'),
  offer: Buffer.from('offer'),
  obligation: Buffer.from('obligation'),
  extraAccountMetas: Buffer.from('extra-account-metas'),
  permit: Buffer.from('permit'),
} as const;

/** `MerchantStatus` on the wire. Not a TS enum on purpose — a plain map, so the module stays
 * type-strippable and runs unbuilt on Node 23+. */
export const MerchantStatus = { Active: 0, Defaulted: 1 } as const;

/** Basis points denominator. `10_000` bps == 1.0. */
export const BPS = 10_000n;

/**
 * The custom error codes the program returns, as Anchor emits them (6000 + variant index).
 * Handy for turning an `0x1771` on a failed redemption into a sentence.
 */
export const OBLIGO_ERRORS: Record<number, string> = {
  6000: 'Overflow',
  6001: 'ReserveBreached',
  6002: 'InvalidTerms',
  6003: 'InvalidAmount',
  6004: 'MerchantDefaulted',
  6005: 'NameTooLong',
  6006: 'InsufficientCollateral',
  6007: 'TermsLocked',
  6008: 'MetadataTooLong',
  6009: 'MintAlreadyExists',
  6010: 'InvalidRate',
  6011: 'OfferExpired',
  6012: 'SelfOffer',
  6013: 'OfferExhausted',
  6014: 'IssuerDefaulted',
  6015: 'PointsExpired',
  6016: 'InsufficientPoints',
  6017: 'NothingToSettle',
  6018: 'InvalidCycle',
  6019: 'EmptyCycle',
  6020: 'NotLiquidatable',
  6021: 'NoClaim',
  6022: 'StillInsolvent',
  6023: 'NotDefaulted',
  6024: 'NotYetExpired',
  6025: 'PermitNotConsumed',
};
