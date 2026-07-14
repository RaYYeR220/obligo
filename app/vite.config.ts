import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));

// The Solana stack expects a Node-style Buffer. Rather than a broad node-polyfill plugin (which
// struggles to inject its shims into the SDK, which lives outside app/node_modules), we polyfill
// exactly what's used: `buffer` (aliased to the npm package — the bare name would hit the Node
// builtin), a tiny `node:crypto` sha256 shim, and a Buffer global set in main.tsx. web3.js's
// browser build references no `process`/`global`.
export default defineConfig({
  plugins: [react()],
  define: {
    global: 'globalThis',
  },
  resolve: {
    alias: [
      { find: /^buffer$/, replacement: resolve(here, 'node_modules/buffer/index.js') },
      { find: 'node:crypto', replacement: resolve(here, 'src/shims/crypto.ts') },
      { find: '@obligo/sdk', replacement: resolve(here, '../sdk/src/index.ts') },
    ],
  },
  optimizeDeps: {
    include: ['@solana/web3.js', '@solana/spl-token', 'buffer'],
  },
  server: { port: 5173 },
});
