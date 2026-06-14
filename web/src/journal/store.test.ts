import { describe, it, expect, vi } from 'vitest';
import {
  JournalStore,
  STORE_KEY,
  classifyStorageError,
  detectPrivateMode,
  durabilityWarnings,
  type StorageLike,
} from './store';
import type { EntryPayload, ReflectQuestion } from '../session/modes';

// The durable store is exercised entirely against an INJECTED backend (no real
// localStorage), so persistence + reload + corrupt-blob + the R23 failure surface
// are all deterministic in node. The held-run advance is the only logic shared
// with the in-memory store; it's checked here too so the durable store can't
// drift from `InMemoryEntryStore`.

// ── A fake StorageLike with controllable failure ──────────────────────────────

interface FakeBackend extends StorageLike {
  readonly map: Map<string, string>;
  /** Make the next (or every) setItem throw the given error. */
  failNext(err: unknown): void;
  failAlways(err: unknown): void;
}

function fakeBackend(seed: Record<string, string> = {}): FakeBackend {
  const map = new Map<string, string>(Object.entries(seed));
  let oneShot: unknown = null;
  let always: unknown = null;
  return {
    map,
    getItem: (k) => map.get(k) ?? null,
    setItem: (k, v) => {
      if (always !== null) throw always;
      if (oneShot !== null) {
        const e = oneShot;
        oneShot = null;
        throw e;
      }
      map.set(k, v);
    },
    removeItem: (k) => {
      map.delete(k);
    },
    failNext: (err) => {
      oneShot = err;
    },
    failAlways: (err) => {
      always = err;
    },
  };
}

function domError(name: string, code?: number): Error {
  const e = new Error(name);
  e.name = name;
  if (code !== undefined) (e as { code?: number }).code = code;
  return e;
}

function reflectQuestion(id: string, cadence = 'daily'): ReflectQuestion {
  return { id, text: `q for ${id}`, slug: `${id}-slug`, addressee: 'self', cadence, slot: null };
}

function reflectEntry(id: string, clean: string, date = '2026-06-13', time = '09:00'): EntryPayload {
  return { date, time, raw: null, clean, mode: 'reflect', question: reflectQuestion(id) };
}

function journalEntry(clean: string, date = '2026-06-13', time = '08:14'): EntryPayload {
  return { date, time, raw: null, clean, mode: 'journal', question: null };
}

// ── Round-trip across reload ───────────────────────────────────────────────────

describe('JournalStore — persistence round-trip (R18)', () => {
  it('a kept entry survives a reload (a fresh store over the same backend)', () => {
    // #given a store that keeps a journal + a reflect entry
    const backend = fakeBackend();
    const a = new JournalStore({ backend });
    a.keep(journalEntry('a free-form note.'));
    a.keep(reflectEntry('avoidance', 'what i am avoiding.'));

    // #when a new store loads the same backend (a simulated revisit)
    const b = new JournalStore({ backend });

    // #then both entries reload intact
    expect(b.journalForDate('2026-06-13').map((e) => e.clean)).toEqual(['a free-form note.']);
    expect(b.thread('avoidance').map((e) => e.clean)).toEqual(['what i am avoiding.']);
    expect(b.hasKept()).toBe(true);
  });

  it('the reflect selection state survives a reload', () => {
    // #given a store that served two questions
    const backend = fakeBackend();
    const a = new JournalStore({ backend });
    a.noteServed('q1');
    a.noteServed('q2');
    a.noteServed('q1');

    // #when reloaded
    const b = new JournalStore({ backend });
    const sel = b.selection();

    // #then the served counts + recency ordinals survive
    expect(sel.servedCount.get('q1')).toBe(2);
    expect(sel.servedCount.get('q2')).toBe(1);
    // q1 was served last → its ordinal is the highest
    expect((sel.lastServed.get('q1') ?? 0) > (sel.lastServed.get('q2') ?? 0)).toBe(true);
  });
});

// ── Corrupt blob → empty, never throws ─────────────────────────────────────────

describe('JournalStore — corrupt-safe load', () => {
  it('a non-JSON blob loads as an empty store without throwing', () => {
    const backend = fakeBackend({ [STORE_KEY]: '{ not valid json' });
    let store!: JournalStore;
    expect(() => {
      store = new JournalStore({ backend });
    }).not.toThrow();
    expect(store.hasKept()).toBe(false);
    expect(store.journalDays()).toEqual([]);
    expect(store.threads()).toEqual([]);
  });

  it('a blob of the wrong shape (non-object) loads as empty', () => {
    const backend = fakeBackend({ [STORE_KEY]: '"a bare string"' });
    const store = new JournalStore({ backend });
    expect(store.hasKept()).toBe(false);
  });

  it('a partially-corrupt blob keeps the readable parts and defaults the rest', () => {
    // #given a blob with valid journalByDate but a garbage selection
    const blob = JSON.stringify({
      schemaVersion: 1,
      journalByDate: { '2026-06-13': [journalEntry('kept.')] },
      threads: {},
      selection: 'not an object',
      hasKept: true,
    });
    const backend = fakeBackend({ [STORE_KEY]: blob });
    const store = new JournalStore({ backend });
    // #then the journal entry survives, the selection is a fresh empty one
    expect(store.journalForDate('2026-06-13').map((e) => e.clean)).toEqual(['kept.']);
    expect(store.selection().servedCount.size).toBe(0);
  });
});

// ── Schema-version migration ───────────────────────────────────────────────────

describe('JournalStore — schema-version migration', () => {
  it('a blob with an older/absent schemaVersion migrates and re-stamps the version', () => {
    // #given a v0-shaped blob (no schemaVersion) carrying entries
    const blob = JSON.stringify({
      journalByDate: { '2026-06-10': [journalEntry('old entry.', '2026-06-10')] },
    });
    const backend = fakeBackend({ [STORE_KEY]: blob });

    // #when loaded
    const store = new JournalStore({ backend });
    // #then the entry migrates intact
    expect(store.journalForDate('2026-06-10').map((e) => e.clean)).toEqual(['old entry.']);
    // #and a subsequent save re-stamps the current schema version
    store.keep(journalEntry('new entry.', '2026-06-11'));
    const saved = JSON.parse(backend.map.get(STORE_KEY) as string) as { schemaVersion: number };
    expect(saved.schemaVersion).toBe(1);
  });
});

// ── Storage-failure states surface distinctly (R23) ────────────────────────────

describe('JournalStore — storage failures surface distinctly (R23)', () => {
  it('quota-exceeded returns a recoverable failure and fires the event', () => {
    const events: string[] = [];
    const backend = fakeBackend();
    const store = new JournalStore({ backend, onStorageEvent: (e) => events.push(e.kind) });
    backend.failNext(domError('QuotaExceededError', 22));

    // #when a keep hits the quota
    const result = store.keep(journalEntry('too much.'));

    // #then it surfaces as quota-exceeded (not silently swallowed)
    expect(result.persisted).toBe(false);
    expect(result.failure?.kind).toBe('quota-exceeded');
    expect(result.failure?.message).toContain('export');
    expect(events).toEqual(['quota-exceeded']);
  });

  it('persist-denied (SecurityError) surfaces as the advisory kind', () => {
    const backend = fakeBackend();
    const store = new JournalStore({ backend });
    backend.failNext(domError('SecurityError'));
    const result = store.keep(journalEntry('blocked.'));
    expect(result.failure?.kind).toBe('persist-denied');
  });

  it('a generic write error surfaces as the retriable write-failure', () => {
    const backend = fakeBackend();
    const store = new JournalStore({ backend });
    backend.failNext(new Error('disk gremlins'));
    const result = store.keep(journalEntry('flaky.'));
    expect(result.failure?.kind).toBe('write-failure');
  });

  it('classifyStorageError maps the legacy Firefox quota name', () => {
    expect(classifyStorageError(domError('NS_ERROR_DOM_QUOTA_REACHED', 1014)).kind).toBe(
      'quota-exceeded',
    );
  });

  it('the in-memory entry is kept even when the durable write fails', () => {
    const backend = fakeBackend();
    const store = new JournalStore({ backend });
    backend.failAlways(domError('QuotaExceededError', 22));
    store.keep(reflectEntry('avoidance', 'still readable in-session.'));
    // #then the read path still returns it (the session is not lost)
    expect(store.thread('avoidance').map((e) => e.clean)).toEqual(['still readable in-session.']);
  });
});

// ── Reflect threads vs journal-by-date ─────────────────────────────────────────

describe('JournalStore — threads by question id, journal by date (R19)', () => {
  it('repeat answers to one question accumulate into a single thread', () => {
    const store = new JournalStore({ backend: fakeBackend() });
    store.keep(reflectEntry('grateful', 'first.'));
    store.keep(reflectEntry('grateful', 'second.'));
    store.keep(reflectEntry('letting-go', 'other.'));
    expect(store.thread('grateful').map((e) => e.clean)).toEqual(['first.', 'second.']);
    expect(store.thread('letting-go').map((e) => e.clean)).toEqual(['other.']);
  });

  it('journal entries group under their local-civil date', () => {
    const store = new JournalStore({ backend: fakeBackend() });
    store.keep(journalEntry('morning.', '2026-06-13', '08:00'));
    store.keep(journalEntry('night.', '2026-06-13', '21:00'));
    store.keep(journalEntry('next day.', '2026-06-14', '07:00'));
    expect(store.journalForDate('2026-06-13').map((e) => e.clean)).toEqual(['morning.', 'night.']);
    expect(store.journalForDate('2026-06-14').map((e) => e.clean)).toEqual(['next day.']);
  });

  it('a held-cadence question advances + clears its run across keeps', () => {
    const store = new JournalStore({ backend: fakeBackend() });
    const held = (clean: string): EntryPayload => ({
      date: '2026-06-13',
      time: '09:00',
      raw: null,
      clean,
      mode: 'reflect',
      question: reflectQuestion('held-q', 'held:2'),
    });
    // first keep starts the run [held-q, 1]; second completes it (>= 2 → cleared)
    store.keep(held('turn 1.'));
    expect(store.selection().heldRun).toEqual(['held-q', 1]);
    store.keep(held('turn 2.'));
    expect(store.selection().heldRun).toBeNull();
  });
});

// ── First-keep prompt fires once (R21) ─────────────────────────────────────────

describe('JournalStore — first-keep export prompt (R21)', () => {
  it('onFirstKeep fires exactly once, on the first kept entry', () => {
    const onFirstKeep = vi.fn();
    const store = new JournalStore({ backend: fakeBackend(), onFirstKeep });
    store.keep(journalEntry('first.'));
    store.keep(journalEntry('second.'));
    store.keep(reflectEntry('q', 'third.'));
    expect(onFirstKeep).toHaveBeenCalledTimes(1);
  });

  it('does not re-fire after a reload of a store that already has entries', () => {
    const backend = fakeBackend();
    new JournalStore({ backend }).keep(journalEntry('already kept.'));
    const onFirstKeep = vi.fn();
    const reloaded = new JournalStore({ backend, onFirstKeep });
    reloaded.keep(journalEntry('another.'));
    expect(onFirstKeep).not.toHaveBeenCalled();
  });
});

// ── Private-mode + durability warnings (R21, R22) ──────────────────────────────

describe('durability honesty decisions (R21, R22)', () => {
  it('detectPrivateMode returns true when the backend write throws', () => {
    const backend = fakeBackend();
    backend.failAlways(domError('QuotaExceededError', 22));
    expect(detectPrivateMode(backend)).toBe(true);
  });

  it('detectPrivateMode returns false when a write round-trips', () => {
    expect(detectPrivateMode(fakeBackend())).toBe(false);
  });

  it('private mode warns only that nothing persists (subsumes eviction)', () => {
    expect(durabilityWarnings({ privateMode: true, persisted: false, hasKept: true })).toEqual([
      'private-mode',
    ]);
  });

  it('a non-persisted profile with entries warns eviction + shared-device', () => {
    expect(durabilityWarnings({ privateMode: false, persisted: false, hasKept: true })).toEqual([
      'eviction-risk',
      'shared-device',
    ]);
  });

  it('a persisted profile with no entries warns nothing', () => {
    expect(durabilityWarnings({ privateMode: false, persisted: true, hasKept: false })).toEqual([]);
  });

  it('a persisted profile with entries still warns shared-device (R22)', () => {
    expect(durabilityWarnings({ privateMode: false, persisted: true, hasKept: true })).toEqual([
      'shared-device',
    ]);
  });
});
