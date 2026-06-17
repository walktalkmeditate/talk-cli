// Responsive word-wrap for the composed frame. The UI is composed at a fixed
// layout; on a narrow terminal (a phone) long lines (questions, hints, the live
// transcript) exceed the column count and xterm's own auto-wrap collides with the
// cursor-home overwrite render → garbled rows. Wrapping each line to the terminal
// width here keeps every physical row ≤ cols, so the overwrite stays aligned and
// text reflows cleanly.
//
// The composed lines carry truecolor SGR escapes (the rust theme). Wrapping is
// SGR-aware: escapes don't count toward width, and the active color is re-applied
// at the start of each wrapped row so a continuation keeps its tone. Desktop is
// untouched — a line that already fits returns unchanged (the fast path).

const SGR_GLOBAL = /\x1b\[[0-9;]*m/g;
const SGR_AT_START = /^\x1b\[[0-9;]*m/;
const RESET = new Set(['\x1b[0m', '\x1b[m']);

/** Visible width of a string, ignoring SGR color escapes. */
export function visibleWidth(s: string): number {
  return s.replace(SGR_GLOBAL, '').length;
}

/**
 * Word-wrap one line (which may contain SGR escapes) to `width` visible columns.
 * Breaks on spaces; an over-long word overflows rather than splitting mid-word.
 * The active color is carried onto each wrapped row. Returns ≥1 lines.
 */
export function wrapAnsiLine(line: string, width: number): string[] {
  if (width <= 0 || visibleWidth(line) <= width) return [line];

  const out: string[] = [];
  let cur = ''; // current physical row (with escapes)
  let curW = 0; // its visible width
  let active = ''; // the SGR color open in the stream right now ('' after a reset)
  let wordColor = ''; // the color open when the current word began (re-applied on wrap)
  let word = ''; // the pending word (with escapes)
  let wordW = 0;

  // Trailing spaces are invisible (the row is cleared with \x1b[K), so trim them
  // off a row rather than letting them push it past the width.
  const pushRow = (s: string): void => {
    out.push(s.replace(/ +$/, ''));
  };
  // Remember the color in effect when a word's first byte arrives, so a wrap can
  // re-open it on the continuation row (the line's closing reset must not strip
  // the colour off the last wrapped word).
  const startWord = (): void => {
    if (word === '' && wordW === 0) wordColor = active;
  };
  const flushWord = (): void => {
    if (word === '') return;
    if (curW > 0 && curW + wordW > width) {
      pushRow(cur);
      cur = wordColor;
      curW = 0;
    }
    cur += word;
    curW += wordW;
    word = '';
    wordW = 0;
  };

  let i = 0;
  while (i < line.length) {
    const ch = line[i];
    if (ch === '\x1b') {
      const m = SGR_AT_START.exec(line.slice(i));
      if (m) {
        const esc = m[0];
        startWord();
        word += esc;
        active = RESET.has(esc) ? '' : esc; // our theme = one color span per token
        i += esc.length;
        continue;
      }
    }
    if (ch === ' ') {
      flushWord();
      if (curW < width) {
        cur += ' '; // keeps leading indentation on the first row
        curW += 1;
      }
      i += 1;
      continue;
    }
    startWord();
    word += ch;
    wordW += 1;
    i += 1;
  }
  flushWord();
  if (cur !== '' || curW > 0 || out.length === 0) pushRow(cur);
  return out;
}

/** Word-wrap a `\r\n`-joined frame body to `width`, line by line (blank lines and
 *  already-fitting lines pass through). */
export function wrapAnsi(content: string, width: number): string {
  return content
    .split('\r\n')
    .flatMap((line) => wrapAnsiLine(line, width))
    .join('\r\n');
}
