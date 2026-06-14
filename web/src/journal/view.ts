// The journal information architecture (U9, R19) — a PURE data→view-model layer.
// It takes the durable store's entries and produces an ordered, grouped view the
// host renders to terminal lines via the theme (the actual painting lives in
// main.ts; this file is logic, so it is unit-testable without a browser).
//
// Two axes, per the plan: BY DATE (journal days, newest first) and BY REFLECT
// THREAD (one group per question id). An empty state when nothing is kept yet.
// "Continue a thread" returns the question id so the mode router can re-prompt it
// (start a reflect session on that exact question).

import type { EntryPayload } from '../session/modes';

/** A single entry as the view surfaces it (a thin projection of EntryPayload). */
export interface JournalEntryView {
  readonly time: string;
  /** The body the list shows (the cleaned text). */
  readonly clean: string;
  /** The verbatim transcript, when retained (for an expanded/raw view). */
  readonly raw: string | null;
  /** The source date (`YYYY-MM-DD`) — useful in the by-thread axis where one
   *  group spans many dates. */
  readonly date: string;
}

/** A journal day group (the by-date axis). */
export interface JournalDateGroup {
  readonly date: string;
  readonly entries: readonly JournalEntryView[];
}

/** A reflect-thread group (the by-thread axis), keyed by question id. */
export interface JournalThreadGroup {
  /** The question id — the `continue` action passes this back to the router. */
  readonly questionId: string;
  /** The question text (the thread's heading). */
  readonly questionText: string;
  /** A short label for the thread (slug, else the id). */
  readonly label: string;
  readonly entries: readonly JournalEntryView[];
}

/** The whole journal view-model the host renders. */
export interface JournalViewModel {
  /** True when nothing has been kept — the host shows the empty state. */
  readonly isEmpty: boolean;
  /** The empty-state copy (only meaningful when `isEmpty`). */
  readonly emptyMessage: string;
  /** Journal days, newest date first. */
  readonly byDate: readonly JournalDateGroup[];
  /** Reflect threads, most-recently-touched first. */
  readonly byThread: readonly JournalThreadGroup[];
}

const EMPTY_MESSAGE = 'no reflections yet — speak one and it lands here';

function toEntryView(e: EntryPayload): JournalEntryView {
  return { time: e.time, clean: e.clean, raw: e.raw, date: e.date };
}

/** Newest-time-first within a day (entries arrive append-order = chronological). */
function byTimeDesc(a: JournalEntryView, b: JournalEntryView): number {
  return b.time.localeCompare(a.time);
}

/** The last (most recent) entry in an append-ordered list, or null. */
function lastOf<T>(xs: readonly T[]): T | null {
  return xs.length > 0 ? xs[xs.length - 1] : null;
}

/**
 * Build the journal view-model from the two grouped sources the store exposes.
 * `journalDays` / `threads` are `[key, entries]` pairs in the store's insertion
 * order; this re-sorts them for display (dates newest-first; threads by their
 * most-recent entry's date+time). Pure — no DOM, no clock, fully testable.
 */
export function buildJournalView(
  journalDays: ReadonlyArray<readonly [string, readonly EntryPayload[]]>,
  threads: ReadonlyArray<readonly [string, readonly EntryPayload[]]>,
): JournalViewModel {
  const byDate: JournalDateGroup[] = journalDays
    .filter(([, entries]) => entries.length > 0)
    .map(([date, entries]) => ({
      date,
      entries: entries.map(toEntryView).sort(byTimeDesc),
    }))
    .sort((a, b) => b.date.localeCompare(a.date));

  const byThread: JournalThreadGroup[] = threads
    .filter(([, entries]) => entries.length > 0)
    .map(([questionId, entries]) => {
      const head = entries[0];
      const question = head.question;
      return {
        questionId,
        questionText: question?.text ?? questionId,
        label: question?.slug ?? questionId,
        entries: entries.map(toEntryView),
      };
    })
    .sort((a, b) => threadRecency(b).localeCompare(threadRecency(a)));

  const isEmpty = byDate.length === 0 && byThread.length === 0;
  return { isEmpty, emptyMessage: EMPTY_MESSAGE, byDate, byThread };
}

/** A thread's sort key: its most-recent entry's `date time` (so the most
 *  recently-touched thread floats to the top). Empty threads sort last. */
function threadRecency(group: JournalThreadGroup): string {
  const last = lastOf(group.entries);
  return last ? `${last.date} ${last.time}` : '';
}

/**
 * "Continue a thread" (R19): given a thread group, return the question id the
 * mode router re-prompts (a reflect session bound to that exact question). A thin
 * indirection so the host's `continue` chip / action has one obvious call.
 */
export function continueThread(group: JournalThreadGroup): string {
  return group.questionId;
}
