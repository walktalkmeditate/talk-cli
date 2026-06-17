// The touch chip row (U7) — ported from meditate-cli/web/src/mobile.ts. On a
// coarse pointer the chip bar is the ONLY control surface, so it must carry
// exactly the controls that are keyboard-driven on desktop. Each chip's
// `command` is the same normalized verb the desktop key handler dispatches, so
// tap and keypress drive one code path (SessionControls.command / the mode
// router in main.ts).

export interface Chip {
  /** What the user sees on the chip. */
  readonly label: string;
  /** The normalized command the chip dispatches (key-parity with desktop). */
  readonly command: string;
  /** A spoken-form accessible name when the visible label is a glyph (↑ / ↓). */
  readonly ariaLabel?: string;
  /** A visual emphasis hint — `primary` pills (the picker's modes) read louder. */
  readonly emphasis?: 'primary' | 'danger';
}

/** A coarse pointer (finger) → show the chip row. */
export function isTouch(): boolean {
  return window.matchMedia('(pointer: coarse)').matches;
}

/** iOS / iPadOS (incl. iPadOS Safari that reports itself as "Mac" — disambiguated
 *  by touch points). iOS Safari's WebGPU + ONNX-runtime path crashes the tab when
 *  loading Whisper, so the engine forces the WASM backend there (see resolveEngine). */
export function isIOS(): boolean {
  if (typeof navigator === 'undefined') return false;
  if (/iPhone|iPad|iPod/i.test(navigator.userAgent)) return true;
  return /Mac/i.test(navigator.platform ?? '') && (navigator.maxTouchPoints ?? 0) > 1;
}

/** A phone/tablet — gets the lighter, faster tiny.en model by default. */
export function isMobile(): boolean {
  if (typeof navigator === 'undefined') return false;
  return isIOS() || /Android/i.test(navigator.userAgent);
}

/**
 * Build the safe-area-aware chip toolbar. `role="toolbar"` groups the buttons
 * for assistive tech; each tap fires `onCommand(chip.command)`. The bar is a
 * single flex row that scrolls horizontally when the chips overflow.
 */
export function createChipBar(chips: readonly Chip[], onCommand: (command: string) => void): HTMLElement {
  const bar = document.createElement('div');
  bar.id = 'chips';
  bar.setAttribute('role', 'toolbar');
  bar.setAttribute('aria-label', 'talk controls');
  // A tap on a pill is NOT a tap on the terminal: stop it bubbling so the
  // document's begin/resume/dismiss handler doesn't also fire under the press.
  bar.addEventListener('pointerdown', (e) => e.stopPropagation());

  for (const chip of chips) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = chip.emphasis ? `chip chip--${chip.emphasis}` : 'chip';
    button.textContent = chip.label;
    if (chip.ariaLabel) button.setAttribute('aria-label', chip.ariaLabel);
    button.addEventListener('click', () => onCommand(chip.command));
    bar.appendChild(button);
  }
  return bar;
}

// ── Per-mode chip sets (U7) ────────────────────────────────────────────────
//
// The plan enumerates the chip bar contents per mode/phase. These are the same
// controls bound to keys on desktop, so the mobile chip bar reaches every
// session action without a keyboard. The `command` strings map onto the
// SessionControls commands ('done'|'pause'|'toggle-raw'|'cancel') plus the
// mode-router verbs ('new-question' for reflect re-roll; 'new-entry'/'export'/
// 'back' for the journal view, wired in U8/U9).

/**
 * Which chip set to show. A "session" phase is an in-progress recording
 * (reflect/journal/unburden); "journal-view" is the durable journal browser.
 */
export type ChipScreen =
  | 'picker'
  | 'reflect-session'
  | 'journal-session'
  | 'unburden-session'
  | 'journal-view'
  | 'journal-confirm';

/** A live session's mode (the U8 router selects it; `ephemeral` = unburden). */
export type SessionMode = 'reflect' | 'journal' | 'ephemeral';

/** Whether a session mode is ephemeral (unburden keeps nothing). */
export function isEphemeralMode(mode: SessionMode): boolean {
  return mode === 'ephemeral';
}

/**
 * The cleanup level the clean view derives, per mode — mirrors the CLI's
 * `config.rs::cleanup_for`: journal → High (paragraphs), reflect/unburden →
 * Light. The `u` raw⇄clean toggle and `shapeEntry` use this so a journal entry's
 * clean view paragraphizes exactly like the CLI export.
 */
export function cleanupForMode(mode: SessionMode): 'High' | 'Light' {
  return mode === 'journal' ? 'High' : 'Light';
}

/** The in-session chip screen for a mode (unburden = ephemeral). */
export function sessionChipScreen(mode: SessionMode): ChipScreen {
  switch (mode) {
    case 'journal':
      return 'journal-session';
    case 'ephemeral':
      return 'unburden-session';
    case 'reflect':
      return 'reflect-session';
  }
}

// Session controls (key-parity with desktop).
const DONE: Chip = { label: 'done', command: 'done' };
const DONE_RELEASE: Chip = { label: 'release', command: 'done' };
const PAUSE: Chip = { label: 'pause', command: 'pause' };
const RAW_CLEAN: Chip = { label: 'raw ⇄ clean', command: 'toggle-raw' };
const NEW_QUESTION: Chip = { label: 'new question', command: 'new-question' };
const CANCEL: Chip = { label: 'cancel', command: 'cancel' };

// Picker (the touch equivalent of pressing 1 / 2 / 3 · v · ?).
const PICK_REFLECT: Chip = { label: 'reflect', command: 'pick-reflect', emphasis: 'primary' };
const PICK_JOURNAL: Chip = { label: 'journal', command: 'pick-journal', emphasis: 'primary' };
const PICK_UNBURDEN: Chip = { label: 'unburden', command: 'pick-unburden', emphasis: 'primary' };
const OPEN_JOURNAL: Chip = { label: 'my journal', command: 'open-journal' };
const HELP: Chip = { label: 'help', command: 'help' };

// Journal browser (the touch equivalent of ↑ ↓ · x · d · c · e · esc).
const SEL_PREV: Chip = { label: '↑', command: 'journal-up', ariaLabel: 'select previous' };
const SEL_NEXT: Chip = { label: '↓', command: 'journal-down', ariaLabel: 'select next' };
const EXPORT_ONE: Chip = { label: 'export', command: 'export-one' };
const EXPORT_ALL: Chip = { label: 'export all', command: 'export-all' };
const DELETE_ONE: Chip = { label: 'delete', command: 'delete-one', emphasis: 'danger' };
const CONTINUE: Chip = { label: 'continue', command: 'continue' };
const BACK: Chip = { label: 'back', command: 'back' };

// Delete confirmation (the touch equivalent of y / n).
const CONFIRM_DELETE: Chip = { label: 'yes, delete', command: 'confirm-yes', emphasis: 'danger' };
const KEEP: Chip = { label: 'keep', command: 'confirm-no' };

/**
 * The chip set for a given screen — the touch control surface that mirrors the
 * desktop keys for every screen, so a phone reaches every action:
 *   picker            — reflect · journal · unburden · my-journal · help
 *   reflect-session   — done · pause · raw⇄clean · new-question · cancel
 *   journal-session   — done · pause · raw⇄clean · cancel
 *   unburden-session  — release · pause · cancel
 *   journal-view      — ↑ · ↓ · export · delete · continue · export-all · back
 *   journal-confirm   — yes-delete · keep
 */
export function chipsFor(screen: ChipScreen): readonly Chip[] {
  switch (screen) {
    case 'picker':
      return [PICK_REFLECT, PICK_JOURNAL, PICK_UNBURDEN, OPEN_JOURNAL, HELP];
    case 'reflect-session':
      return [DONE, PAUSE, RAW_CLEAN, NEW_QUESTION, CANCEL];
    case 'journal-session':
      return [DONE, PAUSE, RAW_CLEAN, CANCEL];
    case 'unburden-session':
      return [DONE_RELEASE, PAUSE, CANCEL];
    case 'journal-view':
      return [SEL_PREV, SEL_NEXT, EXPORT_ONE, DELETE_ONE, CONTINUE, EXPORT_ALL, BACK];
    case 'journal-confirm':
      return [CONFIRM_DELETE, KEEP];
  }
}
