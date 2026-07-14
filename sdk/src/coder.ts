import { createHash } from 'node:crypto';
import { PublicKey } from '@solana/web3.js';

// ---- instruction encoding ------------------------------------------------------------------
//
// There is no IDL (the anchor-cli in this toolchain is version-mismatched and emits none), so the
// wire format is built by hand — exactly as the on-chain program decodes it, and exactly as the
// devnet transcript proved end to end:
//
//   discriminator = sha256("global:<snake_case_ix_name>")[..8]
//   args          = borsh, in declaration order (u64/i64 little-endian, u16 LE, u8, bool,
//                   Pubkey = 32 bytes, String = u32-LE length + utf8 bytes)

/** `sha256("global:<name>")[..8]`. */
export const discriminator = (name: string): Buffer =>
  createHash('sha256').update(`global:${name}`).digest().subarray(0, 8);

export const enc = {
  u8: (v: number): Buffer => Buffer.from([v & 0xff]),
  u16: (v: number): Buffer => {
    const b = Buffer.alloc(2);
    b.writeUInt16LE(v);
    return b;
  },
  u32: (v: number): Buffer => {
    const b = Buffer.alloc(4);
    b.writeUInt32LE(v);
    return b;
  },
  u64: (v: bigint | number): Buffer => {
    const b = Buffer.alloc(8);
    b.writeBigUInt64LE(BigInt(v));
    return b;
  },
  i64: (v: bigint | number): Buffer => {
    const b = Buffer.alloc(8);
    b.writeBigInt64LE(BigInt(v));
    return b;
  },
  bool: (v: boolean): Buffer => Buffer.from([v ? 1 : 0]),
  pubkey: (v: PublicKey): Buffer => v.toBuffer(),
  str: (s: string): Buffer => {
    const sb = Buffer.from(s, 'utf8');
    return Buffer.concat([enc.u32(sb.length), sb]);
  },
};

/** Build an instruction's `data`: 8-byte discriminator followed by borsh-encoded args. */
export const ixData = (name: string, ...parts: Buffer[]): Buffer =>
  Buffer.concat([discriminator(name), ...parts]);

// ---- account decoding ----------------------------------------------------------------------

class Reader {
  private readonly data: Buffer;
  private o: number;
  // `offset` defaults to 8 — past the account discriminator.
  constructor(data: Buffer, offset = 8) {
    this.data = data;
    this.o = offset;
  }
  pubkey(): PublicKey {
    const p = new PublicKey(this.data.subarray(this.o, this.o + 32));
    this.o += 32;
    return p;
  }
  u8(): number {
    return this.data.readUInt8(this.o++);
  }
  u16(): number {
    const v = this.data.readUInt16LE(this.o);
    this.o += 2;
    return v;
  }
  u32(): number {
    const v = this.data.readUInt32LE(this.o);
    this.o += 4;
    return v;
  }
  u64(): bigint {
    const v = this.data.readBigUInt64LE(this.o);
    this.o += 8;
    return v;
  }
  i64(): bigint {
    const v = this.data.readBigInt64LE(this.o);
    this.o += 8;
    return v;
  }
  str(): string {
    const len = this.u32();
    const s = this.data.subarray(this.o, this.o + len).toString('utf8');
    this.o += len;
    return s;
  }
}

export interface Protocol {
  authority: PublicKey;
  usdcMint: PublicKey;
  hookProgram: PublicKey;
  merchantCount: bigint;
  bump: number;
  authorityBump: number;
}

export interface Merchant {
  authority: PublicKey;
  pointsMint: PublicKey;
  vault: PublicKey;
  name: string;
  usdcPerPoint: bigint;
  reserveBps: number;
  pointTtl: bigint;
  collateral: bigint;
  pointsOutstanding: bigint;
  obligationsOut: bigint;
  obligationsIn: bigint;
  totalIssued: bigint;
  totalRedeemed: bigint;
  totalExpired: bigint;
  status: number;
  defaults: number;
  bump: number;
  vaultBump: number;
  mintBump: number;
}

export interface AcceptanceOffer {
  acceptor: PublicKey;
  issuer: PublicKey;
  rateBps: number;
  capacity: bigint;
  consumed: bigint;
  expiresAt: bigint;
  bump: number;
}

export interface Obligation {
  debtor: PublicKey;
  creditor: PublicKey;
  amount: bigint;
  bump: number;
}

export interface PointBatch {
  merchant: PublicKey;
  customer: PublicKey;
  amount: bigint;
  issuedAt: bigint;
  bump: number;
}

export function decodeProtocol(data: Buffer): Protocol {
  const r = new Reader(data);
  return {
    authority: r.pubkey(),
    usdcMint: r.pubkey(),
    hookProgram: r.pubkey(),
    merchantCount: r.u64(),
    bump: r.u8(),
    authorityBump: r.u8(),
  };
}

export function decodeMerchant(data: Buffer): Merchant {
  const r = new Reader(data);
  return {
    authority: r.pubkey(),
    pointsMint: r.pubkey(),
    vault: r.pubkey(),
    name: r.str(),
    usdcPerPoint: r.u64(),
    reserveBps: r.u16(),
    pointTtl: r.i64(),
    collateral: r.u64(),
    pointsOutstanding: r.u64(),
    obligationsOut: r.u64(),
    obligationsIn: r.u64(),
    totalIssued: r.u64(),
    totalRedeemed: r.u64(),
    totalExpired: r.u64(),
    status: r.u8(),
    defaults: r.u32(),
    bump: r.u8(),
    vaultBump: r.u8(),
    mintBump: r.u8(),
  };
}

export function decodeOffer(data: Buffer): AcceptanceOffer {
  const r = new Reader(data);
  return {
    acceptor: r.pubkey(),
    issuer: r.pubkey(),
    rateBps: r.u16(),
    capacity: r.u64(),
    consumed: r.u64(),
    expiresAt: r.i64(),
    bump: r.u8(),
  };
}

export function decodeObligation(data: Buffer): Obligation {
  const r = new Reader(data);
  return {
    debtor: r.pubkey(),
    creditor: r.pubkey(),
    amount: r.u64(),
    bump: r.u8(),
  };
}

export function decodePointBatch(data: Buffer): PointBatch {
  const r = new Reader(data);
  return {
    merchant: r.pubkey(),
    customer: r.pubkey(),
    amount: r.u64(),
    issuedAt: r.i64(),
    bump: r.u8(),
  };
}
