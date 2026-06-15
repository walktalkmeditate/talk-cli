// Render scripts/og-card.html → public/og.jpg (the 1200×630 social card).
//
//   node scripts/gen-og.mjs
//
// Uses the already-installed Playwright chromium (a devDependency). The output
// dimensions match the og:image:width/height in index.html; regenerate this
// whenever the card copy or palette changes.

import { chromium } from '@playwright/test';
import { fileURLToPath } from 'node:url';

const card = fileURLToPath(new URL('./og-card.html', import.meta.url));
const out = fileURLToPath(new URL('../public/og.jpg', import.meta.url));

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1200, height: 630 } });
await page.goto(`file://${card}`);
await page.screenshot({
  path: out,
  type: 'jpeg',
  quality: 92,
  clip: { x: 0, y: 0, width: 1200, height: 630 },
});
await browser.close();
console.log(`wrote ${out}`);
