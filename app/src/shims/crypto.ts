// A minimal, synchronous, browser-safe stand-in for the one thing the SDK imports from node:crypto:
// createHash('sha256') used to build Anchor instruction discriminators. Backed by @noble/hashes,
// which is pure JS and needs no Node builtins.
import { sha256 } from '@noble/hashes/sha256';
import { Buffer } from 'buffer';

class Sha256Hash {
  private chunks: Uint8Array[] = [];
  update(data: Buffer | Uint8Array | string): this {
    const bytes =
      typeof data === 'string' ? new TextEncoder().encode(data) : new Uint8Array(data);
    this.chunks.push(bytes);
    return this;
  }
  digest(): Buffer {
    const total = this.chunks.reduce((n, c) => n + c.length, 0);
    const joined = new Uint8Array(total);
    let o = 0;
    for (const c of this.chunks) {
      joined.set(c, o);
      o += c.length;
    }
    return Buffer.from(sha256(joined));
  }
}

export function createHash(algorithm: string): Sha256Hash {
  if (algorithm !== 'sha256') {
    throw new Error(`crypto shim only implements sha256, got: ${algorithm}`);
  }
  return new Sha256Hash();
}

export default { createHash };
