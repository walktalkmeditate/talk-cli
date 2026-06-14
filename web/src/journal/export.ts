// Markdown export (U9, R20) — turns kept entries into CLI-identical markdown via
// the talk-wasm `appendEntry` façade, so an exported file drops cleanly into a
// real `~/talk` vault / Obsidian. The markdown-building is PURE (testable: its
// output equals what `entry::append` produces), and the two side effects (a file
// download, a clipboard copy) live behind a thin injectable `ExportSink` seam so
// the logic is unit-tested in node and the real browser bits are wired in main.
//
// Entry points (the plan): a per-entry / per-thread / full export, reachable from
// the journal-view `export` chip (U7) AND a `:export` REPL command AND the R21
// first-keep prompt. Each export returns a post-export confirmation naming what
// was exported. Copy-to-clipboard carries a one-time OS-shared disclosure (the
// clipboard is readable by other apps / paste targets).

import { appendEntry } from '../wasm/talk_wasm.js';
import type { EntryPayload } from '../session/modes';

/** What to export. `day` folds one calendar day's journal entries into a single
 *  `YYYY-MM-DD.md` (the Obsidian daily-note pattern); `thread` folds one reflect
 *  question's entries; `all` is the whole vault; `entry` is a single entry. */
export type ExportScope =
  | { readonly kind: 'entry'; readonly entry: EntryPayload }
  | { readonly kind: 'day'; readonly date: string; readonly entries: readonly EntryPayload[] }
  | { readonly kind: 'thread'; readonly questionId: string; readonly entries: readonly EntryPayload[] }
  | { readonly kind: 'all'; readonly entries: readonly EntryPayload[] };

/** How to deliver the markdown — the side-effecting seam (injected). */
export type ExportChannel = 'download' | 'clipboard';

/** The browser side effects, injected so the pure logic is testable. */
export interface ExportSink {
  /** Offer the markdown as a file download named `filename`. */
  download(markdown: string, filename: string): void;
  /** Copy the markdown to the clipboard. Resolves true on success. */
  copy(markdown: string): Promise<boolean>;
}

/** The outcome the host shows as a confirmation line. */
export interface ExportResult {
  /** Whether the delivery succeeded (clipboard copy can be denied). */
  readonly ok: boolean;
  /** The post-export confirmation line (R20). */
  readonly message: string;
  /** The built markdown (so a caller can also display/inspect it). */
  readonly markdown: string;
  /** The filename used for a download (null for clipboard). */
  readonly filename: string | null;
  /** Set the first time a clipboard copy is performed (the one-time OS-shared
   *  disclosure the host shows once). */
  readonly clipboardDisclosure?: string;
}

const CLIPBOARD_DISCLOSURE =
  'copied to the clipboard — readable by anything you paste into; it leaves this page only where you paste it';

// ── Pure markdown building (parity with entry::append) ─────────────────────────

/** "reflect" entries take date-keyed sections; everything else is journal. The
 *  façade itself falls back to journal for unknown modes — mirror that here so
 *  the filename/mode choice matches the rendered markdown. */
function entryMode(payload: EntryPayload): 'reflect' | 'journal' {
  return payload.mode === 'reflect' ? 'reflect' : 'journal';
}

/**
 * Fold a list of entries into one markdown body by replaying `appendEntry` —
 * exactly how the CLI grows a file turn by turn. Starting from an empty body and
 * appending each entry in order reproduces `entry::append`'s date/time sectioning
 * and the `---` same-day divider, so the bytes match a real `~/talk` file.
 *
 * `raw` is passed through only when the entry retained it (keep-raw on); a `null`
 * raw omits the `<!-- raw: -->` comment, matching the CLI.
 */
export function buildBody(entries: readonly EntryPayload[]): string {
  let body = '';
  for (const e of entries) {
    body = appendEntry(
      body,
      e.date,
      e.time,
      e.raw === null ? undefined : e.raw,
      e.clean,
      entryMode(e),
    );
  }
  return body;
}

/** Build the markdown for an export scope (pure). A thread/full export folds all
 *  its entries; a single-entry export folds the one. */
export function buildMarkdown(scope: ExportScope): string {
  switch (scope.kind) {
    case 'entry':
      return buildBody([scope.entry]);
    case 'day':
      return buildBody(scope.entries);
    case 'thread':
      return buildBody(scope.entries);
    case 'all':
      return buildBody(scope.entries);
  }
}

/** A filesystem-safe filename for a scope, ending `.md`. Individual exports use
 *  Obsidian-friendly names — a journal day is its bare date (`2026-06-14.md`, the
 *  daily-note pattern), a reflect thread/entry is its question slug — so they drop
 *  straight into a vault. Only the whole-vault export keeps the `talk-` brand. */
export function filenameFor(scope: ExportScope): string {
  switch (scope.kind) {
    case 'entry': {
      const e = scope.entry;
      return e.mode === 'reflect' && e.question
        ? `${safeStem(e.question.slug ?? e.question.id)}.md`
        : `${e.date}.md`;
    }
    case 'day':
      return `${scope.date}.md`;
    case 'thread': {
      const stem = scope.entries[0]?.question?.slug ?? scope.questionId;
      return `${safeStem(stem)}.md`;
    }
    case 'all':
      return 'talk-journal.md';
  }
}

/** Kebab-ish, ASCII-safe stem: lowercase, non-alphanumerics → '-', collapsed. */
function safeStem(s: string): string {
  const cleaned = s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return cleaned.length > 0 ? cleaned : 'export';
}

function scopeNoun(scope: ExportScope): string {
  switch (scope.kind) {
    case 'entry':
      return 'entry';
    case 'day':
      return 'day';
    case 'thread':
      return 'thread';
    case 'all':
      return 'journal';
  }
}

// ── The export action (side effects behind the sink) ───────────────────────────

/**
 * Tracks the one-time clipboard disclosure across an app session, so the
 * OS-shared note shows on the FIRST clipboard copy only. The host owns one
 * instance and passes it to every export.
 */
export class ExportDisclosure {
  private shown = false;
  /** Returns the disclosure text the first time, then null forever after. */
  takeOnce(): string | null {
    if (this.shown) return null;
    this.shown = true;
    return CLIPBOARD_DISCLOSURE;
  }
}

/**
 * Run an export: build the markdown (pure), then deliver it through the sink. A
 * `download` writes a file; a `clipboard` copy returns the one-time disclosure
 * on its first success. Returns the confirmation the host displays (R20). Never
 * throws — a failed clipboard copy resolves `ok: false` with a retry hint.
 */
export async function runExport(
  scope: ExportScope,
  channel: ExportChannel,
  sink: ExportSink,
  disclosure: ExportDisclosure,
): Promise<ExportResult> {
  const markdown = buildMarkdown(scope);
  const noun = scopeNoun(scope);

  if (channel === 'download') {
    const filename = filenameFor(scope);
    sink.download(markdown, filename);
    return {
      ok: true,
      markdown,
      filename,
      message: `exported ${noun} → ${filename}`,
    };
  }

  // clipboard
  const ok = await sink.copy(markdown);
  if (!ok) {
    return {
      ok: false,
      markdown,
      filename: null,
      message: 'copy failed — try the file download instead',
    };
  }
  const note = disclosure.takeOnce();
  return {
    ok: true,
    markdown,
    filename: null,
    message: `copied ${noun} to the clipboard`,
    ...(note ? { clipboardDisclosure: note } : {}),
  };
}

// ── The real browser sink (wired in main; not exercised by the unit tests) ─────

/**
 * The production `ExportSink` over the DOM: a Blob + a transient `<a download>`
 * for the file, and `navigator.clipboard.writeText` for the copy. Kept thin and
 * isolated so the pure logic above carries the tests; this is the browser-only
 * part flagged as deferred (verified in a real browser / e2e).
 */
export function browserExportSink(): ExportSink {
  return {
    download(markdown: string, filename: string): void {
      const blob = new Blob([markdown], { type: 'text/markdown;charset=utf-8' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = filename;
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
    },
    async copy(markdown: string): Promise<boolean> {
      try {
        if (!navigator.clipboard || typeof navigator.clipboard.writeText !== 'function') {
          return false;
        }
        await navigator.clipboard.writeText(markdown);
        return true;
      } catch {
        return false;
      }
    },
  };
}
