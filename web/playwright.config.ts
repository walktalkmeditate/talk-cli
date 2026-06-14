import { defineConfig, devices } from '@playwright/test';

// E2E harness for the net-silence canary (tests/no-egress.spec.ts). It runs the
// PRODUCTION build under `vite preview` — the same bundle Pages serves, with the
// strict CSP from index.html applied — so the egress proof is against what ships,
// not the dev server. `npm run build` runs build:wasm first, so the WASM façade
// is present; the canary itself only needs the page to boot the mock pipeline.
//
// Run: `npx playwright install chromium && npm run e2e`.
const PORT = 4321;

export default defineConfig({
  testDir: 'tests',
  testMatch: '**/*.spec.ts',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? 'github' : 'list',
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: 'on-first-retry',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  webServer: {
    command: `npm run build && npx vite preview --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}`,
    reuseExistingServer: !process.env.CI,
    timeout: 180_000,
  },
});
