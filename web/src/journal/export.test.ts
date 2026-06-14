import { describe, it, expect, beforeAll, vi } from 'vitest';
import { appendEntry } from '../wasm/talk_wasm.js';
import { initWasmForTest } from '../wasm-test-init';
import {
  buildBody,
  buildMarkdown,
  filenameFor,
  runExport,
  ExportDisclosure,
  type ExportScope,
  type ExportSink,
} from './export';
import type { EntryPayload, ReflectQuestion } from '../session/modes';

// Export builds CLI-identical markdown via the REAL talk-wasm `appendEntry`, so
// parity is asserted against the shared façade (the same bytes `entry::append`
// emits), not a TS re-implementation. The download/clipboard side effects run
// through an injected fake sink — the real DOM/clipboard bits are browser-only
// (deferred to e2e); the logic here (which channel, the confirmation, the
// one-time disclosure) is fully covered.

beforeAll(async () => {
  await initWasmForTest();
});

function question(id: string, slug: string | null = null): ReflectQuestion {
  return { id, text: `q for ${id}`, slug, addressee: 'self', cadence: 'daily', slot: null };
}

function reflect(id: string, clean: string, date = '2026-06-13', time = '09:00', raw: string | null = null): EntryPayload {
  return { date, time, raw, clean, mode: 'reflect', question: question(id, `${id}-slug`) };
}

function journal(clean: string, date = '2026-06-13', time = '08:14', raw: string | null = null): EntryPayload {
  return { date, time, raw, clean, mode: 'journal', question: null };
}

/** A recording sink so the export side effects are observable in node. */
function recordingSink(opts: { copyOk?: boolean } = {}): ExportSink & {
  downloads: Array<{ markdown: string; filename: string }>;
  copies: string[];
} {
  const downloads: Array<{ markdown: string; filename: string }> = [];
  const copies: string[] = [];
  return {
    downloads,
    copies,
    download: (markdown, filename) => downloads.push({ markdown, filename }),
    copy: vi.fn(async (markdown: string) => {
      copies.push(markdown);
      return opts.copyOk ?? true;
    }),
  };
}

// ── Parity with entry::append (R20, AE10) ──────────────────────────────────────

describe('buildMarkdown — parity with talk-wasm appendEntry', () => {
  it('a single journal entry equals one appendEntry call', () => {
    const e = journal('A free-form note.', '2026-06-13', '08:14');
    const expected = appendEntry('', e.date, e.time, undefined, e.clean, 'journal');
    expect(buildMarkdown({ kind: 'entry', entry: e })).toBe(expected);
  });

  it('a single reflect entry equals one appendEntry call (date-keyed)', () => {
    const e = reflect('avoidance', 'What I am avoiding.', '2026-06-06', '08:14');
    const expected = appendEntry('', e.date, e.time, undefined, e.clean, 'reflect');
    expect(buildMarkdown({ kind: 'entry', entry: e })).toBe(expected);
  });

  it('a journal thread folds entries the way the CLI grows a file (with the divider)', () => {
    // #given two same-day journal entries
    const entries = [
      journal('Morning.', '2026-06-08', '08:14'),
      journal('Night.', '2026-06-08', '21:30'),
    ];
    // #when built, the bytes equal replaying appendEntry turn by turn
    let expected = '';
    expected = appendEntry(expected, '2026-06-08', '08:14', undefined, 'Morning.', 'journal');
    expected = appendEntry(expected, '2026-06-08', '21:30', undefined, 'Night.', 'journal');
    const built = buildBody(entries);
    expect(built).toBe(expected);
    // #and it carries the same-day `---` divider entry::append requires
    expect(built).toContain('Morning.\n\n---\n\n## 21:30');
  });

  it('a reflect thread nests repeat dates the way the CLI does', () => {
    const q = question('avoidance', 'avoidance-slug');
    const entries: EntryPayload[] = [
      { date: '2026-06-06', time: '08:14', raw: null, clean: 'First.', mode: 'reflect', question: q },
      { date: '2026-06-06', time: '20:15', raw: null, clean: 'Second.', mode: 'reflect', question: q },
    ];
    let expected = '';
    expected = appendEntry(expected, '2026-06-06', '08:14', undefined, 'First.', 'reflect');
    expected = appendEntry(expected, '2026-06-06', '20:15', undefined, 'Second.', 'reflect');
    expect(buildBody(entries)).toBe(expected);
  });

  it('passes the raw transcript through to the <!-- raw --> comment when retained', () => {
    const e = journal('The thing.', '2026-06-08', '08:14', 'um the thing');
    const expected = appendEntry('', e.date, e.time, 'um the thing', e.clean, 'journal');
    const built = buildMarkdown({ kind: 'entry', entry: e });
    expect(built).toBe(expected);
    expect(built).toContain('<!-- raw: um the thing -->');
  });
});

// ── Filenames ──────────────────────────────────────────────────────────────────

describe('filenameFor', () => {
  it('uses the question slug for a reflect entry', () => {
    expect(filenameFor({ kind: 'entry', entry: reflect('avoidance', 'x.') })).toBe(
      'talk-avoidance-slug.md',
    );
  });

  it('uses the date for a journal entry', () => {
    expect(filenameFor({ kind: 'entry', entry: journal('x.', '2026-06-13') })).toBe(
      'talk-journal-2026-06-13.md',
    );
  });

  it('names a full export talk-journal.md', () => {
    expect(filenameFor({ kind: 'all', entries: [] })).toBe('talk-journal.md');
  });
});

// ── Export action: channels + confirmation + disclosure (R20) ──────────────────

describe('runExport — download channel', () => {
  it('downloads the markdown and confirms with the filename', async () => {
    const sink = recordingSink();
    const scope: ExportScope = { kind: 'entry', entry: journal('Note.', '2026-06-13') };
    const result = await runExport(scope, 'download', sink, new ExportDisclosure());
    expect(result.ok).toBe(true);
    expect(sink.downloads).toHaveLength(1);
    expect(sink.downloads[0].filename).toBe('talk-journal-2026-06-13.md');
    expect(result.message).toContain('talk-journal-2026-06-13.md');
    // download carries no clipboard disclosure
    expect(result.clipboardDisclosure).toBeUndefined();
  });
});

describe('runExport — clipboard channel + one-time disclosure (R20)', () => {
  it('copies to the clipboard and confirms', async () => {
    const sink = recordingSink({ copyOk: true });
    const result = await runExport(
      { kind: 'all', entries: [journal('Note.')] },
      'clipboard',
      sink,
      new ExportDisclosure(),
    );
    expect(result.ok).toBe(true);
    expect(sink.copies).toHaveLength(1);
    expect(result.message).toContain('clipboard');
  });

  it('shows the OS-shared disclosure on the FIRST clipboard copy only', async () => {
    const sink = recordingSink({ copyOk: true });
    const disclosure = new ExportDisclosure();
    const scope: ExportScope = { kind: 'all', entries: [journal('Note.')] };

    const first = await runExport(scope, 'clipboard', sink, disclosure);
    const second = await runExport(scope, 'clipboard', sink, disclosure);

    expect(first.clipboardDisclosure).toBeTruthy();
    expect(second.clipboardDisclosure).toBeUndefined();
  });

  it('a denied clipboard copy returns ok:false with a fallback hint', async () => {
    const sink = recordingSink({ copyOk: false });
    const result = await runExport(
      { kind: 'all', entries: [journal('Note.')] },
      'clipboard',
      sink,
      new ExportDisclosure(),
    );
    expect(result.ok).toBe(false);
    expect(result.message).toContain('download');
  });
});
