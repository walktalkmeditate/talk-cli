import { describe, it, expect, beforeAll } from 'vitest';
import { initWasmForTest } from './wasm-test-init';
import { Theme, themeTones, type Rgb } from './theme';

// The page's dark warm ground (index.html: `background: #14100e`). The rust
// tones are tuned to clear the WCAG targets against this background.
const PAGE_BG: Rgb = [0x14, 0x10, 0x0e];

/** WCAG sRGB relative luminance (0.0–1.0). */
function relLum([r, g, b]: Rgb): number {
  const ch = (v: number): number => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b);
}

/** WCAG contrast ratio between two colors. */
function contrast(fg: Rgb, bg: Rgb): number {
  const a = relLum(fg);
  const b = relLum(bg);
  const hi = Math.max(a, b);
  const lo = Math.min(a, b);
  return (hi + 0.05) / (lo + 0.05);
}

beforeAll(async () => {
  await initWasmForTest();
});

describe('rust palette', () => {
  it('maps to the expected core/dim/edge triples', () => {
    const tones = themeTones('rust');
    expect(tones.core).toEqual([210, 146, 118]);
    expect(tones.dim).toEqual([170, 124, 104]);
    expect(tones.edge).toEqual([150, 122, 112]);
  });

  it('returns null tones for mono (defer to the terminal foreground)', () => {
    const tones = themeTones('mono');
    expect(tones.core).toBeNull();
    expect(tones.dim).toBeNull();
    expect(tones.edge).toBeNull();
  });
});

describe('LineKind → tone mapping', () => {
  it('maps Settled→core, Edge→dim, Question→dim, Chrome→edge', () => {
    const theme = Theme.load('rust');
    // The truecolor SGR for each rust triple.
    const core = '\x1b[38;2;210;146;118m';
    const dim = '\x1b[38;2;170;124;104m';
    const edge = '\x1b[38;2;150;122;112m';

    expect(theme.render('settled', 'x')).toContain(core);
    expect(theme.render('edge', 'x')).toContain(dim);
    expect(theme.render('question', 'x')).toContain(dim);
    expect(theme.render('chrome', 'x')).toContain(edge);
  });
});

describe('WCAG contrast against the page background', () => {
  it('gives core ≥4.5:1 and dim/edge ≥3:1', () => {
    const tones = themeTones('rust');
    // Non-null for a color theme — asserted in the palette test above.
    expect(contrast(tones.core as Rgb, PAGE_BG)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(tones.dim as Rgb, PAGE_BG)).toBeGreaterThanOrEqual(3.0);
    expect(contrast(tones.edge as Rgb, PAGE_BG)).toBeGreaterThanOrEqual(3.0);
  });

  it('keeps core brighter than dim, dim brighter than edge', () => {
    const tones = themeTones('rust');
    expect(relLum(tones.core as Rgb)).toBeGreaterThanOrEqual(relLum(tones.dim as Rgb));
    expect(relLum(tones.dim as Rgb)).toBeGreaterThanOrEqual(relLum(tones.edge as Rgb));
  });
});
