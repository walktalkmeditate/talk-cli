// defineConfig from vitest/config re-exports Vite's, widened with the typed
// `test` field — so the vitest exclude block below typechecks while the Vite
// build options stay fully typed.
import { defineConfig } from 'vitest/config';
import wasm from 'vite-plugin-wasm';
import topLevelAwait from 'vite-plugin-top-level-await';

// base '/' because the site is served at the apex of talk.pilgrimapp.org (a
// custom domain), not a project subpath. Deep-links use the hash fragment, so
// they never hit the server and never 404 on Pages.
//
// Self-host (R6): no third-party assets. The font is a system stack (no
// @font-face), addons (xterm) are npm deps Vite bundles into 'self', and the
// ONLY external origin the app reaches is the model host (cdn.pilgrimapp.org).
// The strict CSP in index.html enforces this; the net-silence canary proves it.
export default defineConfig({
  base: '/',
  plugins: [wasm(), topLevelAwait()],
  // The transformers.js inference worker (src/asr/transformers-worker.ts) is an ES
  // module worker (`new Worker(..., { type: 'module' })`); build it as ESM so its
  // dynamic imports + the wasm plugin work inside the worker bundle too.
  worker: {
    format: 'es',
    plugins: () => [wasm(), topLevelAwait()],
  },
  // @huggingface/transformers ships onnxruntime-web + large dynamic chunks that
  // esbuild's dep pre-bundling mishandles (it tries to bundle the .wasm/.mjs ORT
  // runtime). Excluding it lets Vite serve it as-is and code-split it at build.
  optimizeDeps: {
    exclude: ['@huggingface/transformers'],
  },
  build: {
    target: 'esnext',
    outDir: 'dist',
  },
  test: {
    // vitest owns the unit suite (src/**/*.test.ts). The Playwright net-silence
    // canary (tests/no-egress.spec.ts) drives a real browser and is run by
    // `npm run e2e`, NOT vitest — exclude it so vitest never tries to collect
    // a spec that imports @playwright/test.
    exclude: ['node_modules', 'dist', 'tests/**'],
  },
});
