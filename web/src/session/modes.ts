// The mode router (U8) — owns the three experiences and how you move between
// them. It is the state machine main.ts drives: it boots into the reflect front
// door, selects a curated question, runs a session through the U6 pipeline + U7
// controls, lands the entry on completion (via a persistence seam U9 fills in),
// and routes to the between-session picker.
//
// Ownership boundary:
//   - SELECTION (which reflect question) lives here, threaded through the shared
//     talk-wasm `selectQuestion` rotation (the CLI's selection.rs), with the
//     served/recent/held state held in-memory for now (U9 plugs storage in
//     behind the same EntryStore seam).
//   - The OFF-RECORD / settle / cleanup logic stays in the pipeline + talk-wasm
//     (U6/U3); this router never re-implements it.
//   - The U7 controls own done/pause/raw/cancel; this router owns the cross-mode
//     verbs (new-question / re-roll) and the completion → persist → picker flow.
//
// DOM-free + injectable: the clock (hour + now) and a session factory are passed
// in, so the router logic is unit-tested without a browser, a real engine, or a
// real clock. main.ts supplies the browser wiring.

import {
  selectQuestion as wasmSelectQuestion,
  composeReleased as wasmComposeReleased,
  composeClose as wasmComposeClose,
  selectClosePhrase as wasmSelectClosePhrase,
} from '../wasm/talk_wasm.js';
import { isEphemeralMode, type SessionMode } from '../mobile';

// Vendored from talk-cli/questions/*.toml (the CLI's curated packs). Copied —
// not cross-imported from outside web/ — so the Vite build stays self-contained;
// `?raw` brings each pack in as a string for the wasm `selectQuestion` parser.
// The single source of truth for the question CONTENT is still the CLI repo; a
// pack edit there is re-copied here (a build step could automate it later).
import spineToml from '../questions/spine.toml?raw';

/** The pack the reflect front door selects from. The spine is the default
 *  curated set (mirrors `packs.rs::by_name` falling back to the spine). */
export const REFLECT_PACK_TOML: string = spineToml;

// ── Public API the host drives ──────────────────────────────────────────────

/** Where the router is in the experience flow. */
export type ModePhase =
  /** The between-session picker (reflect · journal · unburden). */
  | 'picker'
  /** A live recording session (reflect / journal / unburden). */
  | 'session'
  /** A held closure moment after a session ends (close phrase / release). */
  | 'closing';

/** The completed-entry payload handed to the persistence seam on `done`.
 *  Ephemeral (unburden) sessions never produce one — they keep nothing. */
export interface EntryPayload {
  /** Local civil date, `YYYY-MM-DD` (the journal/reflect section key). */
  readonly date: string;
  /** Local wall-clock time, `HH:MM`. */
  readonly time: string;
  /** Verbatim transcript, or null when keep-raw is off / not retained. */
  readonly raw: string | null;
  /** The cleaned text (the body of the entry). */
  readonly clean: string;
  /** Which experience produced it. */
  readonly mode: SessionMode;
  /** For reflect: the question this entry answers (binds the thread). */
  readonly question: ReflectQuestion | null;
}

/** A curated question as `selectQuestion` returns it (a parsed JSON object). */
export interface ReflectQuestion {
  readonly id: string;
  readonly text: string;
  readonly slug: string | null;
  readonly addressee: string;
  readonly cadence: string;
  readonly slot: string | null;
}

/**
 * The persistence seam (U9 supplies the real browser-local store; an in-memory
 * implementation ships now). The router calls `keep` exactly once per kept
 * (non-ephemeral, non-cancelled) session, AFTER the entry text is final.
 *
 * The store also owns the reflect SELECTION state so repeat answers to one
 * question accumulate into that question's thread and the rotation avoids
 * recently-served questions across sessions — that state survives with the
 * entries U9 persists, so it lives behind the same seam.
 */
export interface EntryStore {
  /** Persist a completed entry. Reflect entries append to their question's
   *  thread (keyed by `payload.question.id`); journal entries append by date.
   *
   *  Returns `SaveResult | void`: the durable store reports whether the write
   *  landed (so a quota/persist failure is observable AT this seam), while the
   *  in-memory store returns `void`. The router does not depend on the result —
   *  the entry is held in-memory either way — but the host can read it to react. */
  keep(payload: EntryPayload): SaveResult | void;
  /** The selection state the reflect rotation reads (served counts + recency +
   *  any in-progress held run). Returned by reference is fine — the router only
   *  reads it; mutation goes through `keep` / `noteServed`. */
  selection(): SelectionSnapshot;
  /** Record that a question was SHOWN (served) — drives the rotation's recency
   *  even when the user skips/re-rolls before answering, mirroring the CLI which
   *  advances the served ordinal on selection. Returns `SaveResult | void` (same
   *  honesty seam as `keep`). */
  noteServed(questionId: string): SaveResult | void;
}

/**
 * The minimal outcome of a mutating store call: did the durable write land? The
 * durable store (`journal/store.ts`) returns a RICHER `SaveResult` (with the
 * classified failure) that is structurally assignable to this; the in-memory
 * store returns `void`. Widening the seam to `SaveResult | void` lets a host
 * observe a storage failure at the seam without forcing the in-memory store to
 * fabricate one — `void`-returning callers still satisfy it.
 */
export interface SaveResult {
  /** True if the write was durably committed. */
  readonly persisted: boolean;
}

/** A read-only view of the reflect selection state for `selectQuestion`. */
export interface SelectionSnapshot {
  /** id → times served. */
  readonly servedCount: ReadonlyMap<string, number>;
  /** id → last-served ordinal (monotonic; higher = more recent). */
  readonly lastServed: ReadonlyMap<string, number>;
  /** An in-progress held run `[questionId, turnsDone]`, or null. */
  readonly heldRun: readonly [string, number] | null;
}

/** A live session the router drives. The pipeline + controls (U6/U7) implement
 *  this; the router only needs to read the final text and end the session. */
export interface ModeSession {
  /** The settled clean text at session end (the entry body). */
  finalClean(): string;
  /** The verbatim transcript, or null when keep-raw is off / not retained. */
  finalRaw(): string | null;
  /** Whether the session was cancelled (discarded) rather than finished. */
  wasCancelled(): boolean;
  /** Release engine/wasm resources; for ephemeral this also wipes buffers. */
  end(): void;
}

/** Builds a live session for a mode (main.ts wires the real pipeline+controls;
 *  tests inject a fake). Called once per `start`. */
export type SessionFactory = (mode: SessionMode) => ModeSession;

/** Injected clock so selection + entry timestamps are deterministic in tests. */
export interface ModeClock {
  /** The local hour (0..=23) for slot-aware selection. */
  hour(): number;
  /** A `{ date: 'YYYY-MM-DD', time: 'HH:MM' }` local-civil stamp for the entry. */
  stamp(): { date: string; time: string };
  /** `performance.now()`-style monotonic ms, for the closure dwell. */
  now(): number;
}

export interface ModeRouterOptions {
  readonly store: EntryStore;
  readonly session: SessionFactory;
  readonly clock: ModeClock;
  /** Called after any phase/state change so the host repaints. */
  readonly onChange?: () => void;
  /** Reduced motion shortens the unburden closure dwell (R16). */
  readonly reduceMotion?: boolean;
}

/** How long the closure moment holds before returning to the picker (R16). */
export const CLOSE_DWELL_MS = 2500;
export const CLOSE_DWELL_MS_REDUCED = 900;

/**
 * The mode router. Boots into the reflect front door (a question already
 * selected), then: start(mode) → run a session → done/cancel → closing → picker.
 */
export class ModeRouter {
  private readonly store: EntryStore;
  private readonly sessionFactory: SessionFactory;
  private readonly clock: ModeClock;
  private readonly onChange: (() => void) | undefined;
  private readonly reduceMotion: boolean;

  private phase: ModePhase = 'picker';
  private currentMode: SessionMode | null = null;
  private session: ModeSession | null = null;
  private question: ReflectQuestion | null = null;
  /** The held closure lines (close phrase or release), shown while `closing`. */
  private closeLines: readonly string[] = [];
  private closeTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(opts: ModeRouterOptions) {
    this.store = opts.store;
    this.sessionFactory = opts.session;
    this.clock = opts.clock;
    this.onChange = opts.onChange;
    this.reduceMotion = opts.reduceMotion ?? false;
    // Boot the front door: reflect, with the first question already chosen.
    this.start('reflect');
  }

  // ── State readers (the host renders from these) ────────────────────────────

  /** The current phase (picker / session / closing). */
  currentPhase(): ModePhase {
    return this.phase;
  }

  /** The mode of the running/just-run session, or null at the picker. */
  mode(): SessionMode | null {
    return this.currentMode;
  }

  /** The reflect question text to show on-screen (NO TTS), or null when the
   *  current mode shows no question (journal / unburden / at the picker). */
  questionText(): string | null {
    return this.currentMode === 'reflect' ? (this.question?.text ?? null) : null;
  }

  /** The full selected reflect question (for the renderer's held-label etc.). */
  reflectQuestion(): ReflectQuestion | null {
    return this.currentMode === 'reflect' ? this.question : null;
  }

  /** The live session (for the host to wire controls + render the edge). */
  liveSession(): ModeSession | null {
    return this.session;
  }

  /** The closure lines held during `closing` (the close phrase, or the release
   *  line for unburden). Empty unless the phase is `closing`. */
  closureLines(): readonly string[] {
    return this.closeLines;
  }

  /** The labels the between-session picker offers (reflect default first). */
  pickerOptions(): readonly SessionMode[] {
    return ['reflect', 'journal', 'ephemeral'];
  }

  // ── Transitions ────────────────────────────────────────────────────────────

  /**
   * Start a session in `mode`. For reflect, selects the next curated question
   * (the same rotation a skip/re-roll uses). Mid-session switching is NOT
   * offered — `start` is only valid from the picker (or at boot); leaving a
   * running session goes through the U7 cancel path, then back to the picker.
   */
  start(mode: SessionMode): void {
    this.clearCloseTimer();
    // End any session still bound before replacing it, so a deep-link boot (which
    // can start a second session over the constructor's first) or any re-entry
    // never orphans a running pipeline/timer.
    this.session?.end();
    this.currentMode = mode;
    this.question = mode === 'reflect' ? this.selectNext() : null;
    this.session = this.sessionFactory(mode);
    this.phase = 'session';
    this.emit();
  }

  /**
   * Continue an existing reflect thread (U9's "continue a thread", R19): start a
   * reflect session bound to a SPECIFIC question rather than drawing the next via
   * the rotation, so a kept thread can be answered again. Records the serve so
   * the rotation's recency still advances. Valid from the picker (or boot).
   */
  continueQuestion(question: ReflectQuestion): void {
    this.clearCloseTimer();
    // End any session still bound before replacing it (same orphan guard as start).
    this.session?.end();
    this.currentMode = 'reflect';
    this.question = question;
    this.store.noteServed(question.id);
    this.session = this.sessionFactory('reflect');
    this.phase = 'session';
    this.emit();
  }

  /**
   * Open a SPECIFIC reflect question by id — the deep-link seam (U11's
   * `#q=<id>`). Resolves the id against the reflect pack; an UNKNOWN id is a
   * no-op and returns false, so the host falls back to normal reflect selection
   * (the deep-link's sanitization moat already guarantees the id is alphabet-safe
   * before it reaches here, but an id that simply isn't in the pack must not
   * derail the front door). A known id starts a reflect session bound to it,
   * reusing `continueQuestion` so the rotation recency still advances.
   */
  startWithQuestion(id: string): boolean {
    const question = findQuestionInPack(REFLECT_PACK_TOML, id);
    if (question === null) return false;
    this.continueQuestion(question);
    return true;
  }

  /**
   * Skip / re-roll the reflect question BEFORE answering (the `new-question`
   * chip / `n` key / `:skip`). Draws the next question via the same rotation.
   * A no-op outside an in-progress reflect session.
   */
  newQuestion(): void {
    if (this.phase !== 'session' || this.currentMode !== 'reflect') return;
    this.question = this.selectNext();
    this.emit();
  }

  /**
   * Route a cross-mode command the U7 controls do NOT own. `done` and `cancel`
   * are also accepted so the host can funnel ALL completion through the router
   * after the controls have driven the pipeline; `new-question` re-rolls.
   * Returns true if the router handled it.
   */
  command(verb: string): boolean {
    switch (verb) {
      case 'new-question':
        this.newQuestion();
        return true;
      case 'done':
        this.complete(false);
        return true;
      case 'cancel':
        this.complete(true);
        return true;
      default:
        return false;
    }
  }

  /**
   * End the running session. `cancelled` discards (no entry kept, no closure for
   * reflect/journal — the host has already run the U7 confirm-cancel gate);
   * otherwise the entry is landed and the closure moment plays.
   *
   * The privacy guarantee for unburden is structural: an ephemeral session is
   * NEVER handed to `store.keep`, so nothing it transcribed can reach storage.
   */
  complete(cancelled: boolean): void {
    const session = this.session;
    const mode = this.currentMode;
    if (this.phase !== 'session' || session === null || mode === null) return;

    const ephemeral = isEphemeralMode(mode);

    if (cancelled) {
      // Discarded: keep nothing, no closure phrase. Wipe + return to the picker.
      session.end();
      this.toPicker();
      return;
    }

    if (ephemeral) {
      // Unburden release: keep NOTHING, then play the closure (release) moment.
      session.end(); // wipes the transcript buffers (best-effort in a GC runtime)
      this.beginClosing(parseLines(wasmComposeReleased()));
      return;
    }

    // Reflect / journal: land the entry through the persistence seam, then the
    // contemplative close phrase.
    const payload = this.buildPayload(session, mode);
    this.store.keep(payload);

    const close = this.closeFor(payload);
    session.end();
    this.beginClosing(close);
  }

  /**
   * Dismiss the closure moment early (a keypress / tap), or it auto-dismisses
   * after the dwell. Returns to the between-session picker.
   */
  dismissClosure(): void {
    if (this.phase !== 'closing') return;
    this.toPicker();
  }

  /** Tear down (host unload): cancel any pending dwell + free the session. */
  dispose(): void {
    this.clearCloseTimer();
    this.session?.end();
    this.session = null;
  }

  // ── Internals ────────────────────────────────────────────────────────────

  /** Select the next reflect question via the shared rotation, advancing the
   *  served recency so a re-roll never re-draws the just-shown question. */
  private selectNext(): ReflectQuestion | null {
    const q = this.selectQuestionFromState();
    if (q !== null) this.store.noteServed(q.id);
    return q;
  }

  private selectQuestionFromState(): ReflectQuestion | null {
    const sel = this.store.selection();
    const servedIds = [...sel.servedCount.keys()];
    const servedCounts = Uint32Array.from(servedIds, (id) => sel.servedCount.get(id) ?? 0);
    const recentIds = [...sel.lastServed.keys()];
    const recentOrdinals = Float64Array.from(recentIds, (id) => sel.lastServed.get(id) ?? 0);
    const [heldId, heldDone] = sel.heldRun ?? ['', 0];

    const json = wasmSelectQuestion(
      REFLECT_PACK_TOML,
      servedIds,
      servedCounts,
      recentIds,
      recentOrdinals,
      heldId,
      heldDone,
      clampHour(this.clock.hour()),
    );
    return json === undefined ? null : parseQuestion(json);
  }

  private buildPayload(session: ModeSession, mode: SessionMode): EntryPayload {
    const { date, time } = this.clock.stamp();
    return {
      date,
      time,
      raw: session.finalRaw(),
      clean: session.finalClean(),
      mode,
      question: mode === 'reflect' ? this.question : null,
    };
  }

  private closeFor(payload: EntryPayload): readonly string[] {
    // Mirror the CLI close screen (render_model::compose_close): a path-ish line
    // + a rotated close phrase. The web has no filesystem path; the thread/date
    // label stands in for it so the close still names where the entry landed.
    const where =
      payload.mode === 'reflect' && payload.question
        ? `thread · ${payload.question.slug ?? payload.question.id}`
        : `journal · ${payload.date}`;
    const provenance = payload.mode === 'reflect' ? 'reflection kept' : 'entry kept';
    const phrase = pickClosePhrase(this.clock.now());
    return parseLines(wasmComposeClose(where, provenance, phrase));
  }

  private beginClosing(lines: readonly string[]): void {
    this.closeLines = lines;
    this.phase = 'closing';
    this.session = null;
    const dwell = this.reduceMotion ? CLOSE_DWELL_MS_REDUCED : CLOSE_DWELL_MS;
    this.clearCloseTimer();
    this.closeTimer = setTimeout(() => {
      this.closeTimer = null;
      // Only auto-dismiss if we are still showing this closure (the user may
      // have already dismissed it manually).
      if (this.phase === 'closing') this.toPicker();
    }, dwell);
    this.emit();
  }

  private toPicker(): void {
    this.clearCloseTimer();
    this.phase = 'picker';
    this.currentMode = null;
    this.session = null;
    this.question = null;
    this.closeLines = [];
    this.emit();
  }

  private clearCloseTimer(): void {
    if (this.closeTimer !== null) {
      clearTimeout(this.closeTimer);
      this.closeTimer = null;
    }
  }

  private emit(): void {
    this.onChange?.();
  }
}

// ── In-memory persistence (U9 swaps in the real browser-local store) ─────────

/**
 * A simple in-memory EntryStore for now: it accumulates reflect entries into
 * per-question threads, journal entries by date, and tracks the selection
 * recency so the rotation behaves across sessions. U9 replaces this with the
 * versioned-blob localStorage store behind the same interface — the router does
 * not change.
 */
export class InMemoryEntryStore implements EntryStore {
  /** Reflect threads: question id → the entries answering it, in order. */
  readonly threads = new Map<string, EntryPayload[]>();
  /** Journal entries by local-civil date. */
  readonly journalByDate = new Map<string, EntryPayload[]>();

  private readonly servedCount = new Map<string, number>();
  private readonly lastServed = new Map<string, number>();
  private heldRun: [string, number] | null = null;
  /** Monotonic serve ordinal (higher = more recent), mirroring selection.rs. */
  private serveOrdinal = 0;

  keep(payload: EntryPayload): void {
    if (payload.mode === 'reflect' && payload.question) {
      const id = payload.question.id;
      const thread = this.threads.get(id) ?? [];
      thread.push(payload);
      this.threads.set(id, thread);
      this.advanceHeldRun(payload.question);
    } else if (payload.mode === 'journal') {
      const day = this.journalByDate.get(payload.date) ?? [];
      day.push(payload);
      this.journalByDate.set(payload.date, day);
    }
    // ephemeral never reaches here (the router never persists it).
  }

  selection(): SelectionSnapshot {
    return {
      servedCount: this.servedCount,
      lastServed: this.lastServed,
      heldRun: this.heldRun,
    };
  }

  noteServed(questionId: string): void {
    this.servedCount.set(questionId, (this.servedCount.get(questionId) ?? 0) + 1);
    this.serveOrdinal += 1;
    this.lastServed.set(questionId, this.serveOrdinal);
  }

  /** The accumulated entries answering one reflect question (its thread). */
  thread(questionId: string): readonly EntryPayload[] {
    return this.threads.get(questionId) ?? [];
  }

  private advanceHeldRun(question: ReflectQuestion): void {
    const heldLen = parseHeldLen(question.cadence);
    if (heldLen === null) return;
    const [id, done] = this.heldRun ?? [question.id, 0];
    if (id !== question.id) {
      this.heldRun = [question.id, 1];
      return;
    }
    const next = done + 1;
    this.heldRun = next >= heldLen ? null : [question.id, next];
  }
}

// ── Pure helpers ─────────────────────────────────────────────────────────────

/** Parse a JSON string array (composeClose / composeReleased), defensively. */
function parseLines(json: string): readonly string[] {
  try {
    const parsed: unknown = JSON.parse(json);
    if (Array.isArray(parsed) && parsed.every((s) => typeof s === 'string')) {
      return parsed as string[];
    }
  } catch (err) {
    // fall through to the empty closure (the host just shows nothing), but surface
    // the malformed wasm output — a silent empty closure would hide a real bug.
    console.warn('modes: invalid JSON from a compose* wasm export', err);
  }
  return [];
}

/** Parse the `selectQuestion` JSON object into a typed ReflectQuestion. */
function parseQuestion(json: string): ReflectQuestion | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch (err) {
    console.warn('modes: invalid JSON from selectQuestion wasm export', err);
    return null;
  }
  if (typeof parsed !== 'object' || parsed === null) return null;
  const q = parsed as Record<string, unknown>;
  if (typeof q.id !== 'string' || typeof q.text !== 'string') return null;
  return {
    id: q.id,
    text: q.text,
    slug: typeof q.slug === 'string' ? q.slug : null,
    addressee: typeof q.addressee === 'string' ? q.addressee : 'self',
    cadence: typeof q.cadence === 'string' ? q.cadence : 'daily',
    slot: typeof q.slot === 'string' ? q.slot : null,
  };
}

/** Parse "held:N" → N; anything else → null (mirrors `Pack::held_len`). */
function parseHeldLen(cadence: string): number | null {
  const m = /^held:(\d+)$/.exec(cadence);
  return m ? Number(m[1]) : null;
}

/**
 * Resolve a question by id from a pack's TOML — the deep-link lookup (U11). The
 * shared `selectQuestion` wasm export only ROTATES (it has no by-id seam), so to
 * open a specific `#q=<id>` we scan the pack's `[[questions]]` blocks here.
 *
 * This is a focused reader of the CLI pack shape (`questions.rs` / the
 * `questions/*.toml` files), NOT a general TOML parser: each `[[questions]]`
 * block is flat `key = "value"` lines (`id`, `text`, and optional `slot` /
 * `cadence` / `addressee`), and the defaults mirror `parseQuestion`. The id is
 * matched verbatim (the caller has already alphabet-gated it). Returns null for
 * an unknown id so the router falls back to normal selection.
 */
export function findQuestionInPack(packToml: string, id: string): ReflectQuestion | null {
  // Split on the `[[questions]]` table-array header; the first chunk is the
  // pack-level header (name/description) and carries no question.
  const blocks = packToml.split(/^\s*\[\[questions\]\]\s*$/m).slice(1);
  for (const block of blocks) {
    const fields = parseBlockFields(block);
    const blockId = fields.get('id');
    if (blockId === undefined || blockId !== id) continue;
    const text = fields.get('text');
    if (text === undefined) return null; // a question with no text is unusable
    return {
      id: blockId,
      text,
      slug: fields.get('slug') ?? null,
      addressee: fields.get('addressee') ?? 'self',
      cadence: fields.get('cadence') ?? 'daily',
      slot: fields.get('slot') ?? null,
    };
  }
  return null;
}

/** Read the flat `key = "value"` lines of one `[[questions]]` block into a map.
 *  Stops at the next table header (a `[` line), so a block never bleeds into the
 *  next. Only double-quoted string values are read (the pack shape). */
function parseBlockFields(block: string): Map<string, string> {
  const fields = new Map<string, string>();
  for (const line of block.split('\n')) {
    const trimmed = line.trim();
    if (trimmed.startsWith('[')) break;
    const m = /^([a-z_]+)\s*=\s*"((?:[^"\\]|\\.)*)"\s*$/.exec(trimmed);
    if (m) fields.set(m[1], m[2]);
  }
  return fields;
}

function clampHour(hour: number): number {
  if (!Number.isFinite(hour)) return 0;
  return Math.max(0, Math.min(23, Math.floor(hour)));
}

/**
 * Pick a curated close phrase via the shared wasm export (`selectClosePhrase`),
 * which reads `talk_core::close::CLOSE_PHRASES` — the SINGLE source of truth the
 * CLI uses too, so the web can never drift from it. The CLI rotates by kept-entry
 * count; the web has no on-disk count here yet (U9), so it rotates by the clock so
 * a returning user does not see the same line twice in a row. A non-finite seed
 * clamps to 0 at the boundary.
 */
function pickClosePhrase(seed: number): string {
  const safe = Number.isFinite(seed) ? Math.abs(Math.floor(seed)) : 0;
  return wasmSelectClosePhrase(safe);
}
