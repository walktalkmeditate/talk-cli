import { describe, it, expect } from 'vitest';
import { chipsFor, type ChipScreen } from './mobile';

// chipsFor is the touch control surface — the pills that let a phone reach every
// action the desktop keys do. These lock the command strings (which tsc can't
// verify against the host's handleCommand router) and the per-screen coverage.

const commands = (screen: ChipScreen): string[] => chipsFor(screen).map((c) => c.command);

describe('chipsFor — the touch pill set per screen', () => {
  it('the picker offers the three modes plus journal + help', () => {
    expect(commands('picker')).toEqual([
      'pick-reflect',
      'pick-journal',
      'pick-unburden',
      'open-journal',
      'help',
    ]);
  });

  it('the journal browser mirrors the cursor keys (↑ ↓ · x · d · c · e · esc)', () => {
    expect(commands('journal-view')).toEqual([
      'journal-up',
      'journal-down',
      'export-one',
      'delete-one',
      'continue',
      'export-all',
      'back',
    ]);
  });

  it('the delete confirmation is a two-pill y/n', () => {
    expect(commands('journal-confirm')).toEqual(['confirm-yes', 'confirm-no']);
  });

  it('a reflect session exposes every control key as a pill', () => {
    expect(commands('reflect-session')).toEqual([
      'done',
      'pause',
      'toggle-raw',
      'new-question',
      'cancel',
    ]);
  });

  it('unburden never offers raw⇄clean (nothing is kept to clean)', () => {
    expect(commands('unburden-session')).not.toContain('toggle-raw');
  });

  it('every pill has a visible label, and glyph pills carry an accessible name', () => {
    const screens: ChipScreen[] = [
      'picker',
      'reflect-session',
      'journal-session',
      'unburden-session',
      'journal-view',
      'journal-confirm',
    ];
    for (const screen of screens) {
      for (const chip of chipsFor(screen)) {
        expect(chip.label.length).toBeGreaterThan(0);
        // A glyph-only label (↑ / ↓) must have an aria-label for screen readers.
        if (/^[↑↓]$/.test(chip.label)) expect(chip.ariaLabel).toBeTruthy();
      }
    }
  });
});
