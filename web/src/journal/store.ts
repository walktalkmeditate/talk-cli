// The durable browser-local journal store (U9). It is the production
// implementation behind `session/modes.ts::EntryStore` — the mode router calls
// `keep` / `selection` / `noteServed` exactly as it did against the in-memory
// store, but here the state survives a reload in one versioned localStorage blob.
//
// Ported from `meditate-cli/web/src/store.ts`: a single corrupt-safe load (any
// unreadable blob becomes a fresh store rather than blocking the app), a
// try/catch save, an explicit schema version, and a migration shape. talk's
// schema is richer than meditate's streak map — it holds the kept entries
// (journal by local-civil date + reflect threads keyed by question id) AND the
// reflect selection state (served counts + recency + held run) so the rotation
// behaves across visits.
//
// Storage-failure honesty (R23): saves do NOT silently swallow. Each failure is
// classified (quota-exceeded / persist-denied / write-failure) and surfaced via
// the constructor `onStorageEvent` listener AND the return value of the mutating
// calls, so the UI can react distinctly. The store is injectable (it takes a
// `StorageLike` backend), so the whole thing is unit-testable without a real
// `localStorage`.

import type {
  EntryPayload,
  EntryStore,
  ReflectQuestion,
  SelectionSnapshot,
} from '../session/modes';

/** The localStorage key (versioned, mirrors meditate's `meditate.v1`). */
export const STORE_KEY = 'talk.v1';
/** The current on-disk schema version. A bump runs the migration ladder. */
export const SCHEMA_VERSION = 1;

/**
 * The minimal `localStorage` surface the store needs. Injecting it (rather than
 * touching the global) is what makes the store testable in node + lets the host
 * pass a fallback when `localStorage` is unavailable (private mode hard-block).
 */
export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/** Why a write could not be committed (R23) — each surfaced distinctly. */
export type StorageFailureKind =
  /** The browser refused the write because the origin's quota is exhausted.
   *  Recoverable: the UI surfaces "storage full — export to keep these". */
  | 'quota-exceeded'
  /** Persistence is denied (private/incognito, or `persist()` was refused).
   *  Advisory: writes may not survive — surface an eviction-risk banner. */
  | 'persist-denied'
  /** A non-quota write error (serialization, transient backend). Retriable. */
  | 'write-failure';

/** A storage event the host listens to so it can surface R23 states. */
export interface StorageEvent {
  readonly kind: StorageFailureKind;
  /** A human-facing message the UI can show verbatim. */
  readonly message: string;
}

/** The result of a mutating store call: did the durable write land? */
export interface SaveResult {
  /** True if the blob was committed to the backend. */
  readonly persisted: boolean;
  /** Set when `persisted` is false — the classified failure. */
  readonly failure?: StorageEvent;
}

const OK: SaveResult = { persisted: true };

// ── On-disk shape ─────────────────────────────────────────────────────────────

/** The persisted selection state (parallel to `SelectionSnapshot`, but plain
 *  objects so it JSON-serializes; rebuilt into Maps on load). */
interface StoredSelection {
  servedCount: Record<string, number>;
  lastServed: Record<string, number>;
  heldRun: [string, number] | null;
  serveOrdinal: number;
}

interface StoreData {
  schemaVersion: number;
  /** Reflect threads: question id → entries answering it, in order. */
  threads: Record<string, EntryPayload[]>;
  /** Journal entries by local-civil date (`YYYY-MM-DD`). */
  journalByDate: Record<string, EntryPayload[]>;
  selection: StoredSelection;
  /** Set true once the first entry was kept — drives the one-time first-keep
   *  export prompt (R21). Persisted so the prompt fires once, ever. */
  hasKept: boolean;
}

function emptyData(): StoreData {
  return {
    schemaVersion: SCHEMA_VERSION,
    threads: {},
    journalByDate: {},
    selection: { servedCount: {}, lastServed: {}, heldRun: null, serveOrdinal: 0 },
    hasKept: false,
  };
}

// ── Corrupt-safe load + migration ─────────────────────────────────────────────

/**
 * Load the blob, tolerating anything unreadable (absent / non-JSON / wrong shape)
 * by returning a fresh store — the journal must never fail to open because of a
 * bad blob. A lower `schemaVersion` is run through `migrate`.
 */
function loadData(backend: StorageLike): StoreData {
  let raw: string | null;
  try {
    raw = backend.getItem(STORE_KEY);
  } catch {
    // Even the read can throw (e.g. a SecurityError in some locked-down modes).
    return emptyData();
  }
  if (raw === null || raw === '') return emptyData();

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return emptyData();
  }
  if (typeof parsed !== 'object' || parsed === null) return emptyData();

  return migrate(parsed as Partial<StoreData> & { schemaVersion?: unknown });
}

/**
 * Normalize a parsed blob to the current schema. Every field is defensively
 * re-derived so a partially-corrupt or older blob still loads with sensible
 * defaults. As schema versions are added, version-specific transforms slot in
 * here BEFORE the field normalization (the migration ladder).
 */
function migrate(parsed: Partial<StoreData> & { schemaVersion?: unknown }): StoreData {
  // (No v0→v1 transform yet; the ladder is the shape future bumps plug into.)
  const empty = emptyData();
  const sel = parsed.selection ?? null;
  return {
    schemaVersion: SCHEMA_VERSION,
    threads: isRecordOfArrays(parsed.threads) ? parsed.threads : empty.threads,
    journalByDate: isRecordOfArrays(parsed.journalByDate)
      ? parsed.journalByDate
      : empty.journalByDate,
    selection: normalizeSelection(sel),
    hasKept: parsed.hasKept === true,
  };
}

function normalizeSelection(sel: unknown): StoredSelection {
  const empty = emptyData().selection;
  if (typeof sel !== 'object' || sel === null) return empty;
  const s = sel as Partial<StoredSelection>;
  return {
    servedCount: isRecordOfNumbers(s.servedCount) ? s.servedCount : {},
    lastServed: isRecordOfNumbers(s.lastServed) ? s.lastServed : {},
    heldRun: isHeldRun(s.heldRun) ? s.heldRun : null,
    serveOrdinal: typeof s.serveOrdinal === 'number' && Number.isFinite(s.serveOrdinal)
      ? s.serveOrdinal
      : 0,
  };
}

/** A shape guard for one stored entry. A corrupt/old-shape element (missing the
 *  required string fields) would otherwise deliver `undefined` fields into
 *  `appendEntry` at export time; this rejects it at load. `question` is optional
 *  (journal entries have none) and only loosely checked — a malformed question is
 *  tolerated as `null` rather than failing the whole entry. */
function isEntryPayload(v: unknown): v is EntryPayload {
  if (typeof v !== 'object' || v === null) return false;
  const e = v as Record<string, unknown>;
  return (
    typeof e.date === 'string' &&
    typeof e.time === 'string' &&
    typeof e.clean === 'string' &&
    (e.raw === null || typeof e.raw === 'string') &&
    typeof e.mode === 'string'
  );
}

function isRecordOfArrays(v: unknown): v is Record<string, EntryPayload[]> {
  return (
    typeof v === 'object' &&
    v !== null &&
    Object.values(v).every(
      (arr) => Array.isArray(arr) && arr.every(isEntryPayload),
    )
  );
}

function isRecordOfNumbers(v: unknown): v is Record<string, number> {
  return (
    typeof v === 'object' &&
    v !== null &&
    Object.values(v).every((n) => typeof n === 'number' && Number.isFinite(n))
  );
}

function isHeldRun(v: unknown): v is [string, number] {
  return (
    Array.isArray(v) &&
    v.length === 2 &&
    typeof v[0] === 'string' &&
    typeof v[1] === 'number'
  );
}

// ── Concurrent-tab merge ──────────────────────────────────────────────────────
//
// Two tabs share one localStorage origin. Without a merge, the last tab to `save`
// would clobber the other's entries (last-write-wins lost-update). Before every
// save the store re-loads the current on-disk blob and MERGES the in-memory state
// into it: entries are unioned by identity, selection counts/ordinals take the
// max, and `hasKept` is OR-ed. A `storage` event (fired in OTHER tabs) reloads the
// in-memory mirror so a tab reflects a sibling's writes.

/** A stable identity for an entry, for union-dedup across tabs. Entries carry no
 *  id, so the natural key is date+time+mode+clean (a second identical entry at the
 *  same minute is indistinguishable and treated as the same — acceptable). */
function entryKey(e: EntryPayload): string {
  return `${e.date} ${e.time} ${e.mode} ${e.clean}`;
}

/** Union two ordered entry lists, keeping `base`'s order then appending entries
 *  from `incoming` whose key isn't already present. */
function unionEntries(base: EntryPayload[], incoming: EntryPayload[]): EntryPayload[] {
  const seen = new Set(base.map(entryKey));
  const out = [...base];
  for (const e of incoming) {
    const k = entryKey(e);
    if (!seen.has(k)) {
      seen.add(k);
      out.push(e);
    }
  }
  return out;
}

/** Union two `Record<string, EntryPayload[]>` maps key-by-key. */
function unionRecord(
  base: Record<string, EntryPayload[]>,
  incoming: Record<string, EntryPayload[]>,
): Record<string, EntryPayload[]> {
  const out: Record<string, EntryPayload[]> = {};
  for (const k of new Set([...Object.keys(base), ...Object.keys(incoming)])) {
    out[k] = unionEntries(base[k] ?? [], incoming[k] ?? []);
  }
  return out;
}

/** Max-merge two number maps (served counts / recency ordinals). */
function maxMergeNumbers(
  base: Record<string, number>,
  incoming: Record<string, number>,
): Record<string, number> {
  const out: Record<string, number> = { ...base };
  for (const [k, v] of Object.entries(incoming)) {
    out[k] = Math.max(out[k] ?? 0, v);
  }
  return out;
}

/**
 * Merge the on-disk blob (`disk`, possibly written by another tab) with this
 * tab's in-memory state (`mine`). Entries are unioned; selection counts/ordinals
 * take the max so neither tab's rotation regresses; `hasKept` is OR-ed; the held
 * run prefers the one carrying the higher serve ordinal (the more-recent writer).
 */
function mergeData(disk: StoreData, mine: StoreData): StoreData {
  const serveOrdinal = Math.max(disk.selection.serveOrdinal, mine.selection.serveOrdinal);
  const heldRun =
    mine.selection.serveOrdinal >= disk.selection.serveOrdinal
      ? mine.selection.heldRun
      : disk.selection.heldRun;
  return {
    schemaVersion: SCHEMA_VERSION,
    threads: unionRecord(disk.threads, mine.threads),
    journalByDate: unionRecord(disk.journalByDate, mine.journalByDate),
    selection: {
      servedCount: maxMergeNumbers(disk.selection.servedCount, mine.selection.servedCount),
      lastServed: maxMergeNumbers(disk.selection.lastServed, mine.selection.lastServed),
      heldRun,
      serveOrdinal,
    },
    hasKept: disk.hasKept || mine.hasKept,
  };
}

// ── Failure classification ────────────────────────────────────────────────────

/**
 * Classify a thrown save error into the R23 surface. `QuotaExceededError` (and
 * its legacy Firefox `NS_ERROR_DOM_QUOTA_REACHED` / IE `code === 22`) means the
 * blob is too big — recoverable via export. A `SecurityError` (or any
 * `localStorage` access throwing in private mode) means persistence is denied.
 * Everything else is a retriable write failure.
 */
export function classifyStorageError(err: unknown): StorageEvent {
  if (isQuotaError(err)) {
    return {
      kind: 'quota-exceeded',
      message: 'storage full — export to keep these entries',
    };
  }
  if (isPersistDeniedError(err)) {
    return {
      kind: 'persist-denied',
      message: 'this browser is not saving — entries may be lost on close',
    };
  }
  return {
    kind: 'write-failure',
    message: 'could not save — retry, or export to keep these entries',
  };
}

function isQuotaError(err: unknown): boolean {
  if (!(err instanceof Error)) return false;
  const name = err.name;
  // The legacy numeric code is on DOMException; read it defensively.
  const code = (err as { code?: number }).code;
  return (
    name === 'QuotaExceededError' ||
    name === 'NS_ERROR_DOM_QUOTA_REACHED' ||
    code === 22 ||
    code === 1014
  );
}

function isPersistDeniedError(err: unknown): boolean {
  if (!(err instanceof Error)) return false;
  return err.name === 'SecurityError';
}

// ── The store ─────────────────────────────────────────────────────────────────

/** The cross-tab event surface (a structural subset of `window`). Injected so the
 *  listener is testable in node and absent gracefully in a non-browser runtime. */
export interface StorageEventTarget {
  addEventListener(
    type: 'storage',
    listener: (event: { key: string | null }) => void,
  ): void;
  removeEventListener(
    type: 'storage',
    listener: (event: { key: string | null }) => void,
  ): void;
}

export interface JournalStoreOptions {
  /** The persistence backend (inject a fake in tests; `localStorage` in main). */
  readonly backend: StorageLike;
  /** Surfaced storage failures (R23) — the host shows the right UI per kind. */
  readonly onStorageEvent?: (event: StorageEvent) => void;
  /** Fired once, the first time an entry is kept (R21 export prompt). */
  readonly onFirstKeep?: () => void;
  /**
   * The cross-tab event source for the `storage` event (defaults to `window` when
   * present). When another tab writes `STORE_KEY`, this store reloads its mirror
   * so it reflects the sibling's writes. Pass a fake in tests; pass `null` (or
   * leave it in a non-browser runtime) to disable the listener.
   */
  readonly storageEvents?: StorageEventTarget | null;
  /** Notified after the mirror reloads from another tab's write, so the host can
   *  repaint a stale journal view. */
  readonly onExternalChange?: () => void;
}

/**
 * The durable journal. Implements `EntryStore`, so `main.ts` swaps it in for
 * `InMemoryEntryStore` with no router change. Reads come from an in-memory mirror
 * loaded once at construction; every mutation updates the mirror AND attempts a
 * durable save, surfacing any failure rather than swallowing it.
 */
export class JournalStore implements EntryStore {
  private readonly backend: StorageLike;
  private readonly onStorageEvent: ((event: StorageEvent) => void) | undefined;
  private readonly onFirstKeep: (() => void) | undefined;
  private readonly onExternalChange: (() => void) | undefined;
  private readonly storageEvents: StorageEventTarget | null;
  private readonly onStorage: (event: { key: string | null }) => void;
  private data: StoreData;

  constructor(opts: JournalStoreOptions) {
    this.backend = opts.backend;
    this.onStorageEvent = opts.onStorageEvent;
    this.onFirstKeep = opts.onFirstKeep;
    this.onExternalChange = opts.onExternalChange;
    this.data = loadData(this.backend);

    // Cross-tab sync: when another tab writes our key, reload the mirror. Default
    // to `window` when present; a non-browser runtime (or an explicit null) skips it.
    this.storageEvents =
      opts.storageEvents !== undefined
        ? opts.storageEvents
        : typeof window !== 'undefined'
          ? (window as unknown as StorageEventTarget)
          : null;
    this.onStorage = (event: { key: string | null }): void => {
      // `key === null` is a `clear()`; either matters to us.
      if (event.key !== null && event.key !== STORE_KEY) return;
      this.data = loadData(this.backend);
      this.onExternalChange?.();
    };
    this.storageEvents?.addEventListener('storage', this.onStorage);
  }

  /** Detach the cross-tab listener (host unload). Safe to call when none attached. */
  dispose(): void {
    this.storageEvents?.removeEventListener('storage', this.onStorage);
  }

  // ── EntryStore seam ─────────────────────────────────────────────────────────

  /**
   * Persist a completed entry (R18). Reflect entries append to their question's
   * thread; journal entries append by date. Returns whether the durable write
   * landed — the router ignores the result (it keeps the entry in-memory either
   * way), but the host can read it to react to a quota/persist failure.
   *
   * `EntryStore.keep` returns void; this widens the return to `SaveResult` (a
   * structural superset — `void`-typed callers still type-check) so the host has
   * the failure without a second call.
   */
  keep(payload: EntryPayload): SaveResult {
    const wasEmpty = !this.data.hasKept;

    if (payload.mode === 'reflect' && payload.question) {
      const id = payload.question.id;
      const thread = this.data.threads[id] ?? [];
      thread.push(payload);
      this.data.threads[id] = thread;
      this.advanceHeldRun(payload.question);
    } else if (payload.mode === 'journal') {
      const day = this.data.journalByDate[payload.date] ?? [];
      day.push(payload);
      this.data.journalByDate[payload.date] = day;
    } else {
      // ephemeral never reaches here (the router never persists it); be inert.
      return OK;
    }

    this.data.hasKept = true;
    const result = this.save();
    if (wasEmpty) this.onFirstKeep?.();
    return result;
  }

  selection(): SelectionSnapshot {
    return {
      servedCount: new Map(Object.entries(this.data.selection.servedCount)),
      lastServed: new Map(Object.entries(this.data.selection.lastServed)),
      heldRun: this.data.selection.heldRun,
    };
  }

  noteServed(questionId: string): SaveResult {
    const sel = this.data.selection;
    sel.servedCount[questionId] = (sel.servedCount[questionId] ?? 0) + 1;
    sel.serveOrdinal += 1;
    sel.lastServed[questionId] = sel.serveOrdinal;
    return this.save();
  }

  // ── Read accessors the view/export consume ──────────────────────────────────

  /** The accumulated entries answering one reflect question (its thread). */
  thread(questionId: string): readonly EntryPayload[] {
    return this.data.threads[questionId] ?? [];
  }

  /** All reflect threads as `[questionId, entries]`, in insertion order. */
  threads(): ReadonlyArray<readonly [string, readonly EntryPayload[]]> {
    return Object.entries(this.data.threads);
  }

  /** Journal entries for one local-civil date. */
  journalForDate(date: string): readonly EntryPayload[] {
    return this.data.journalByDate[date] ?? [];
  }

  /** All journal days as `[date, entries]`, in insertion order. */
  journalDays(): ReadonlyArray<readonly [string, readonly EntryPayload[]]> {
    return Object.entries(this.data.journalByDate);
  }

  /** Whether any entry has ever been kept (drives the first-keep prompt + the
   *  at-rest warning gate). */
  hasKept(): boolean {
    return this.data.hasKept;
  }

  /** Every kept entry across journal + threads (for a full export). */
  allEntries(): readonly EntryPayload[] {
    const out: EntryPayload[] = [];
    for (const day of Object.values(this.data.journalByDate)) out.push(...day);
    for (const thread of Object.values(this.data.threads)) out.push(...thread);
    return out;
  }

  // ── Durability ──────────────────────────────────────────────────────────────

  /**
   * Opt-in persistent storage (R21) — best-effort: returns true if the origin's
   * storage is (now) persisted. A browser that lacks `navigator.storage.persist`
   * resolves false (the host then keeps the eviction-risk warning). Never throws.
   */
  async requestPersist(nav: Navigator = navigator): Promise<boolean> {
    try {
      if (nav.storage && typeof nav.storage.persist === 'function') {
        return await nav.storage.persist();
      }
    } catch {
      // fall through — persistence simply isn't grantable here
    }
    return false;
  }

  /** Serialize the data for export/backup (a JSON snapshot of the whole store). */
  exportSnapshot(): string {
    return JSON.stringify(this.data);
  }

  /**
   * Commit the in-memory state, MERGING the current on-disk blob first so a
   * concurrent tab's writes are never clobbered (lost-update). The merged result
   * becomes this tab's mirror, so both tabs converge. On a write failure the
   * in-memory entry is retained (the session is not lost) but the blob is NOT
   * durably written — the failure is surfaced so the host can prompt an export and
   * not present the entry as durably kept.
   */
  private save(): SaveResult {
    // Re-load + merge so a sibling tab's writes survive this write.
    const disk = loadData(this.backend);
    this.data = mergeData(disk, this.data);
    try {
      this.backend.setItem(STORE_KEY, JSON.stringify(this.data));
      return OK;
    } catch (err) {
      const failure = classifyStorageError(err);
      this.onStorageEvent?.(failure);
      return { persisted: false, failure };
    }
  }

  /** Mirror `InMemoryEntryStore.advanceHeldRun` so the held rotation survives. */
  private advanceHeldRun(question: ReflectQuestion): void {
    const heldLen = parseHeldLen(question.cadence);
    if (heldLen === null) return;
    const sel = this.data.selection;
    const current = sel.heldRun;
    if (current === null || current[0] !== question.id) {
      sel.heldRun = [question.id, 1];
      return;
    }
    const next = current[1] + 1;
    sel.heldRun = next >= heldLen ? null : [question.id, next];
  }
}

/** Parse "held:N" → N; anything else → null (mirrors `Pack::held_len`). */
function parseHeldLen(cadence: string): number | null {
  const m = /^held:(\d+)$/.exec(cadence);
  return m ? Number(m[1]) : null;
}

// ── Durability + at-rest honesty (R21, R22) ────────────────────────────────────
//
// These are the decisions the UI shows the user about what does and does not
// survive, and what is exposed at rest. They are deliberately small pure
// functions / a tiny detector so the policy is unit-tested, not buried in DOM
// glue. The host renders the resulting copy; it makes none of these decisions.

/** The warnings the host may surface, each a distinct R21/R22 concern. */
export type DurabilityWarning =
  /** R21 — best-effort private/incognito detection: nothing will persist. */
  | 'private-mode'
  /** R21 — storage is not persisted, so the browser may evict under pressure. */
  | 'eviction-risk'
  /** R22 — entries are unencrypted in this browser profile (shared device). */
  | 'shared-device';

/** Human-facing copy for each warning (shown verbatim by the host). */
export const DURABILITY_WARNING_COPY: Readonly<Record<DurabilityWarning, string>> = {
  'private-mode':
    'private window — nothing you keep here will survive after you close it. Export to keep it.',
  'eviction-risk':
    'this browser may clear these entries under storage pressure. Grant persistent storage, or export, to keep them.',
  'shared-device':
    'these entries are stored unencrypted in this browser profile — anyone using this profile can read them. Unburden writes nothing to storage (its in-memory wipe is best-effort).',
};

/**
 * Best-effort private/incognito detection (R21). There is no reliable, portable
 * incognito API, so this is a probe: a `setItem`/`removeItem` round-trip on the
 * injected backend. If the write throws (or a read-back differs), persistence is
 * not available — treat it as private mode. Never throws. The host should treat
 * a `true` result as "warn nothing persists," not as a hard fact.
 */
export function detectPrivateMode(backend: StorageLike): boolean {
  const probeKey = `${STORE_KEY}.__probe`;
  try {
    backend.setItem(probeKey, '1');
    const readBack = backend.getItem(probeKey);
    backend.removeItem(probeKey);
    return readBack !== '1';
  } catch {
    return true;
  }
}

/**
 * Decide which durability/exposure warnings the host should show, given the
 * detected facts. Pure so the policy is tested directly:
 *   - private mode → only `private-mode` (it subsumes eviction; nothing persists)
 *   - persisted    → no eviction warning; otherwise `eviction-risk`
 *   - any kept entry on a non-private profile → `shared-device` (once; the host
 *     gates the once-ness, this only decides whether it's applicable)
 */
export function durabilityWarnings(facts: {
  readonly privateMode: boolean;
  readonly persisted: boolean;
  readonly hasKept: boolean;
}): readonly DurabilityWarning[] {
  if (facts.privateMode) return ['private-mode'];
  const out: DurabilityWarning[] = [];
  if (!facts.persisted) out.push('eviction-risk');
  if (facts.hasKept) out.push('shared-device');
  return out;
}
