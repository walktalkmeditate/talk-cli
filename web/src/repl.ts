// A small line editor on top of xterm: the input buffer, cursor, history, and
// tab-completion that the prompt line is rendered from. Pure and DOM-free so it
// unit-tests cleanly; `main.ts` owns the terminal and command dispatch.

export interface KeyResult {
  /** A completed command line, when Enter was pressed on non-empty input. */
  submitted?: string;
  /** Whether the prompt needs redrawing. */
  changed: boolean;
}

const NO_CHANGE: KeyResult = { changed: false };
const CHANGED: KeyResult = { changed: true };

/** Longest shared prefix of a set of strings (for ambiguous tab-completion). */
function commonPrefix(words: string[]): string {
  if (words.length === 0) return '';
  let prefix = words[0];
  for (const w of words.slice(1)) {
    let i = 0;
    while (i < prefix.length && i < w.length && prefix[i] === w[i]) i++;
    prefix = prefix.slice(0, i);
  }
  return prefix;
}

export class Repl {
  private buffer = '';
  private cursor = 0;
  private history: string[] = [];
  private histIdx = -1; // -1 == editing the live line (not browsing history)
  private draft = ''; // the live line, stashed while browsing history

  /** `completions` returns the candidate command names for Tab. */
  constructor(private readonly completions: () => string[] = () => []) {}

  get input(): string {
    return this.buffer;
  }

  handle(data: string): KeyResult {
    switch (data) {
      case '\r':
      case '\n':
        return this.submit();
      case '\x7f':
      case '\b':
        return this.backspace();
      case '\x1b[D':
        return this.moveCursor(-1);
      case '\x1b[C':
        return this.moveCursor(1);
      case '\x1b[A':
        return this.historyPrev();
      case '\x1b[B':
        return this.historyNext();
      case '\x1b[H':
      case '\x01': // Ctrl-A
        return this.setCursor(0);
      case '\x1b[F':
      case '\x05': // Ctrl-E
        return this.setCursor(this.buffer.length);
      case '\x15': // Ctrl-U
        return this.replace('', 0);
      case '\x03': // Ctrl-C — abandon the line
        return this.replace('', 0);
      case '\t':
        return this.complete();
      default: {
        if (data.startsWith('\x1b')) return NO_CHANGE; // an unhandled escape, not text
        // A single char or a paste: strip control bytes and insert the printable
        // remainder (so an embedded \t or \x07 can't drive the terminal, and a
        // paste that merely starts with a control byte isn't dropped wholesale).
        // eslint-disable-next-line no-control-regex
        const clean = data.replace(/[\x00-\x1f\x7f]/g, '');
        return clean ? this.insert(clean) : NO_CHANGE;
      }
    }
  }

  /** Render the prompt line with an inverse-video cursor cell. When the line is
   *  empty, a dim ghost hint follows the cursor. */
  line(prompt: string, ghost: string | null): string {
    const at = this.buffer[this.cursor] ?? ' ';
    const cursorCell = `\x1b[7m${at}\x1b[27m`;
    if (this.buffer === '') {
      const tail = ghost ? `\x1b[2m${ghost}\x1b[0m` : '';
      return prompt + cursorCell + tail;
    }
    const before = this.buffer.slice(0, this.cursor);
    const after = this.buffer.slice(this.cursor + 1);
    return prompt + before + cursorCell + after;
  }

  // ── editing primitives ─────────────────────────────────────────────────────

  private insert(text: string): KeyResult {
    this.buffer = this.buffer.slice(0, this.cursor) + text + this.buffer.slice(this.cursor);
    this.cursor += text.length;
    this.histIdx = -1;
    return CHANGED;
  }

  private backspace(): KeyResult {
    if (this.cursor === 0) return NO_CHANGE;
    this.buffer = this.buffer.slice(0, this.cursor - 1) + this.buffer.slice(this.cursor);
    this.cursor--;
    this.histIdx = -1;
    return CHANGED;
  }

  private moveCursor(delta: number): KeyResult {
    return this.setCursor(this.cursor + delta);
  }

  private setCursor(pos: number): KeyResult {
    const next = Math.max(0, Math.min(this.buffer.length, pos));
    if (next === this.cursor) return NO_CHANGE;
    this.cursor = next;
    return CHANGED;
  }

  private replace(buffer: string, cursor: number): KeyResult {
    this.buffer = buffer;
    this.cursor = cursor;
    this.histIdx = -1;
    return CHANGED;
  }

  private submit(): KeyResult {
    const line = this.buffer.trim();
    if (line.length > 0 && this.history[this.history.length - 1] !== line) {
      this.history.push(line);
    }
    this.buffer = '';
    this.cursor = 0;
    this.histIdx = -1;
    this.draft = '';
    return line.length > 0 ? { submitted: line, changed: true } : CHANGED;
  }

  private historyPrev(): KeyResult {
    if (this.history.length === 0) return NO_CHANGE;
    if (this.histIdx === -1) {
      this.draft = this.buffer;
      this.histIdx = this.history.length;
    }
    if (this.histIdx === 0) return NO_CHANGE;
    this.histIdx--;
    return this.replaceFromHistory(this.history[this.histIdx]);
  }

  private historyNext(): KeyResult {
    if (this.histIdx === -1) return NO_CHANGE;
    this.histIdx++;
    if (this.histIdx >= this.history.length) {
      this.histIdx = -1;
      return this.replaceFromHistory(this.draft);
    }
    return this.replaceFromHistory(this.history[this.histIdx]);
  }

  private replaceFromHistory(value: string): KeyResult {
    this.buffer = value;
    this.cursor = value.length;
    return CHANGED;
  }

  private complete(): KeyResult {
    // Only complete the command word at end-of-line (no space, cursor at end) —
    // completing mid-buffer would discard the text after the cursor.
    if (this.cursor !== this.buffer.length) return NO_CHANGE;
    const head = this.buffer;
    if (head.includes(' ')) return NO_CHANGE;
    const matches = this.completions().filter((c) => c.startsWith(head));
    if (matches.length === 0) return NO_CHANGE;
    const fill = matches.length === 1 ? matches[0] + ' ' : commonPrefix(matches);
    if (fill === head) return NO_CHANGE;
    return this.replace(fill, fill.length);
  }
}
