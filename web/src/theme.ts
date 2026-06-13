// The rust palette, sourced from `talk-wasm palette()` — the single source of
// truth shared with the CLI. meditate hard-coded its accent in `ansi.ts`, which
// let the TS and Rust tones drift; here the three tones are read once from the
// WASM façade so the browser can never disagree with the engine.
//
// The palette is three RGB triples: `core` (settled text, brightest), `dim` (the
// live edge + the question, mid), `edge` (chrome — borders/header/status,
// quietest). Mono returns an empty byte array, meaning "use the terminal fg".

import { palette as wasmPalette } from './wasm/talk_wasm.js';

const RESET = '\x1b[0m';

export type Rgb = readonly [number, number, number];

/** A `LineKind` as emitted by `Settle.compose()` JSON (`kind` field). */
export type LineKind = 'chrome' | 'settled' | 'edge' | 'question';

const KNOWN_LINE_KINDS: ReadonlySet<string> = new Set<LineKind>([
  'chrome',
  'settled',
  'edge',
  'question',
]);

/** One composed line as `Settle.compose()` emits it (a `{text, kind}` object). */
export interface ComposedLine {
  text: string;
  kind: LineKind;
}

function isComposedLine(value: unknown): value is ComposedLine {
  if (typeof value !== 'object' || value === null) return false;
  const line = value as Record<string, unknown>;
  return typeof line.text === 'string' && typeof line.kind === 'string' && KNOWN_LINE_KINDS.has(line.kind);
}

/**
 * Parse the JSON `Settle.compose()` returns into typed composed lines. Defensive
 * at the wasm↔JS boundary: malformed JSON yields `[]` (the screen blanks for a
 * frame rather than the loop throwing), and any element that is not a
 * `{text: string, kind: <known LineKind>}` is dropped and logged.
 */
export function parseComposed(json: string): ComposedLine[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch (err) {
    console.error('parseComposed: invalid JSON from Settle.compose()', err);
    return [];
  }
  if (!Array.isArray(parsed)) {
    console.error('parseComposed: expected a JSON array, got', parsed);
    return [];
  }
  const lines: ComposedLine[] = [];
  for (const element of parsed) {
    if (isComposedLine(element)) {
      lines.push(element);
    } else {
      console.warn('parseComposed: dropping malformed composed line', element);
    }
  }
  return lines;
}

/** The three paintable tones for a theme. `null` (Mono) defers to the terminal fg. */
export interface ThemeTones {
  core: Rgb | null;
  dim: Rgb | null;
  edge: Rgb | null;
}

/**
 * Read a theme's tones from the WASM palette. The façade returns 9 bytes
 * (core/dim/edge RGB) for a color theme, or an empty array for Mono (defer to
 * the terminal foreground). `init()` must have resolved before calling this.
 */
export function themeTones(theme: string): ThemeTones {
  const bytes = wasmPalette(theme);
  if (bytes.length === 0) {
    return { core: null, dim: null, edge: null };
  }
  if (bytes.length < 9) {
    throw new Error(`palette('${theme}') returned ${bytes.length} bytes, expected 9`);
  }
  return {
    core: [bytes[0], bytes[1], bytes[2]],
    dim: [bytes[3], bytes[4], bytes[5]],
    edge: [bytes[6], bytes[7], bytes[8]],
  };
}

/** Wrap text in a truecolor SGR escape for the given RGB, or a faint pass-through
 *  when the tone is `null` (Mono — defer to the terminal's own foreground). */
function paint(rgb: Rgb | null, faint: boolean, s: string): string {
  if (rgb === null) {
    return faint ? `\x1b[2m${s}${RESET}` : s;
  }
  const [r, g, b] = rgb;
  return `\x1b[38;2;${r};${g};${b}m${s}${RESET}`;
}

/**
 * A theme renderer bound to a set of tones: `core`/`dim`/`edge` wrap text in the
 * truecolor escape for that palette triple, and `toneForKind` maps a composed
 * `LineKind` to the right wrapper (Settled→core, Edge & Question→dim,
 * Chrome→edge).
 */
export class Theme {
  constructor(private readonly tones: ThemeTones) {}

  static load(theme = 'rust'): Theme {
    return new Theme(themeTones(theme));
  }

  core(s: string): string {
    return paint(this.tones.core, false, s);
  }

  dim(s: string): string {
    return paint(this.tones.dim, true, s);
  }

  edge(s: string): string {
    return paint(this.tones.edge, true, s);
  }

  /** Wrap a composed line in the tone its `LineKind` maps to. */
  render(kind: LineKind, text: string): string {
    return this.toneForKind(kind)(text);
  }

  /**
   * Render composed lines into a frame body: each line painted in its kind's
   * tone (with `\x1b[K` to erase a shrinking row's stale tail) and joined with
   * `\r\n`. Feed this to `frameSequence` for one synchronized write.
   */
  renderComposed(lines: ComposedLine[]): string {
    return lines.map((l) => `${this.render(l.kind, l.text)}\x1b[K`).join('\r\n');
  }

  /** The wrapper a `LineKind` paints in: Settled→core, Edge & Question→dim,
   *  Chrome→edge. Returned as a bound function so callers can apply it directly.
   *  An unknown/new Rust `LineKind` falls back to `core` so it is still visible
   *  (never silently undefined) — Phase C is expected to consume this directly. */
  toneForKind(kind: LineKind): (s: string) => string {
    switch (kind) {
      case 'settled':
        return (s) => this.core(s);
      case 'edge':
      case 'question':
        return (s) => this.dim(s);
      case 'chrome':
        return (s) => this.edge(s);
      default:
        return (s) => this.core(s);
    }
  }
}
