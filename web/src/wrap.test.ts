import { describe, it, expect } from 'vitest';
import { visibleWidth, wrapAnsiLine, wrapAnsi } from './wrap';

const C = (s: string): string => `\x1b[38;2;210;146;118m${s}\x1b[0m`; // a rust-core span

describe('visibleWidth — ignores SGR escapes', () => {
  it('counts only the visible characters', () => {
    expect(visibleWidth('hello')).toBe(5);
    expect(visibleWidth(C('hello'))).toBe(5);
    expect(visibleWidth(`${C('ab')} ${C('cd')}`)).toBe(5); // "ab cd"
  });
});

describe('wrapAnsiLine — SGR-aware word wrap', () => {
  it('leaves a fitting line untouched (desktop fast path)', () => {
    const line = C('short enough');
    expect(wrapAnsiLine(line, 80)).toEqual([line]);
  });

  it('wraps on word boundaries to the given width', () => {
    const rows = wrapAnsiLine('the quick brown fox', 9);
    expect(rows.map(visibleWidth).every((w) => w <= 9)).toBe(true);
    // reflowed text (ignoring escapes) preserves the words in order
    expect(rows.join(' ').replace(/\x1b\[[0-9;]*m/g, '').split(/\s+/).filter(Boolean)).toEqual([
      'the',
      'quick',
      'brown',
      'fox',
    ]);
  });

  it('re-applies the active color on each wrapped row', () => {
    const rows = wrapAnsiLine(C('alpha beta gamma'), 7);
    expect(rows.length).toBeGreaterThan(1);
    // every row that has visible text carries the color escape
    for (const row of rows) {
      if (visibleWidth(row) > 0) expect(row).toContain('\x1b[38;2;210;146;118m');
    }
  });

  it('keeps each wrapped row within the width', () => {
    const long = 'What complexities around life and human nature still mystify you?';
    for (const row of wrapAnsiLine(long, 24)) {
      expect(visibleWidth(row)).toBeLessThanOrEqual(24);
    }
  });

  it('does not split a word shorter than the width across rows', () => {
    const rows = wrapAnsiLine('hello wonderful', 10);
    expect(rows).toEqual(['hello', 'wonderful']);
  });
});

describe('wrapAnsi — whole frame, preserving blank lines', () => {
  it('wraps each line and keeps blank rows', () => {
    const out = wrapAnsi('aaa bbb ccc\r\n\r\nddd', 7);
    const lines = out.split('\r\n');
    expect(lines).toContain(''); // the blank line survived
    expect(lines.every((l) => visibleWidth(l) <= 7)).toBe(true);
  });
});
