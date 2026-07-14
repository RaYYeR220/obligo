// Must be the very first import in the app. The Solana stack and the SDK reference a Node-style
// Buffer at module-evaluation time (e.g. the SDK's PDA seeds), so the global has to exist before any
// of them are evaluated — which, for ES modules, means setting it in a module imported first.
import { Buffer } from 'buffer';

const g = globalThis as unknown as { Buffer?: typeof Buffer; global?: unknown };
if (!g.Buffer) g.Buffer = Buffer;
if (!g.global) g.global = globalThis;
