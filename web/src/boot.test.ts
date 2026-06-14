import { describe, it, expect } from 'vitest';
import { relativeTime, renderBoot, INSTALL_HINT, BOOT_HOST } from './boot';

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

describe('relativeTime', () => {
  it('reads as a calm, friendly phrase across the buckets', () => {
    const now = 1_000_000_000;
    // #given elapsed durations spanning each bucket boundary
    // #then each reads as its friendly phrase
    expect(relativeTime(now - 10_000, now)).toBe('just now'); // < 90s
    expect(relativeTime(now - 89_000, now)).toBe('just now'); // boundary
    expect(relativeTime(now - 5 * MIN, now)).toBe('5 minutes ago');
    expect(relativeTime(now - HOUR, now)).toBe('1 hour ago'); // singular
    expect(relativeTime(now - 3 * HOUR, now)).toBe('3 hours ago'); // plural
    expect(relativeTime(now - DAY, now)).toBe('1 day ago'); // singular
    expect(relativeTime(now - 4 * DAY, now)).toBe('4 days ago'); // plural
  });

  it('never goes negative on a backwards clock skew', () => {
    // #given a "from" later than "now" (clock skew)
    // #then it clamps to "just now" rather than a negative duration
    expect(relativeTime(1000, 0)).toBe('just now');
  });
});

describe('renderBoot', () => {
  it('shows a Last-visit line for a returning visitor over the MOTD', () => {
    // #given a previous visit two hours ago
    const lines = renderBoot('0.1.0', 1000, 1000 + 2 * HOUR);
    const texts = lines.map((l) => l.text);
    // #then the login line names the elapsed time + the web host
    expect(texts).toContain(`Last visit: 2 hours ago on ${BOOT_HOST}`);
    // #and the MOTD carries the wordmark, version stamp, and install funnel
    expect(texts).toContain('talk');
    expect(texts.some((t) => t.includes('v0.1.0'))).toBe(true);
    expect(texts.some((t) => t.includes(INSTALL_HINT))).toBe(true);
  });

  it('welcomes a first-time visitor (no Last-visit line)', () => {
    // #given no previous visit
    const lines = renderBoot('0.1.0', null, Date.now());
    const texts = lines.map((l) => l.text);
    // #then the first line is a welcome, never a "Last visit" line
    expect(texts.some((t) => t.includes('first reflection'))).toBe(true);
    expect(texts.some((t) => t.startsWith('Last visit'))).toBe(false);
  });

  it('tags the login line dim and the wordmark core (host paints from these)', () => {
    // #given a returning visitor's banner
    const lines = renderBoot('0.1.0', 1000, 1000 + HOUR);
    const login = lines.find((l) => l.text.startsWith('Last visit'));
    const wordmark = lines.find((l) => l.text === 'talk');
    // #then the tones drive the rust theme mapping in main.ts
    expect(login?.tone).toBe('dim');
    expect(wordmark?.tone).toBe('core');
  });
});
