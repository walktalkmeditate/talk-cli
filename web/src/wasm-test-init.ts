// Test-only WASM initializer. In the browser, `init()` fetches the `.wasm`
// alongside the JS via `import.meta.url`; under vitest (node) there is no
// `fetch` for a `file://` URL, so we read the bytes and hand them to `init()`
// directly. Memoized — the WASM module is a process-global singleton.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import init from './wasm/talk_wasm.js';

let ready: Promise<void> | null = null;

/** Ensure the talk-wasm module is instantiated before the tests touch it. */
export function initWasmForTest(): Promise<void> {
  if (ready === null) {
    const wasmPath = fileURLToPath(new URL('./wasm/talk_wasm_bg.wasm', import.meta.url));
    const bytes = readFileSync(wasmPath);
    ready = init({ module_or_path: bytes }).then(() => undefined);
  }
  return ready;
}
