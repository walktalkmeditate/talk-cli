import { describe, it, expect } from 'vitest';
import { buildJournalView, continueThread } from './view';
import type { EntryPayload, ReflectQuestion } from '../session/modes';

// The journal IA is a pure data→view-model transform, so it's tested directly off
// `[key, entries]` pairs (the exact shape JournalStore.journalDays()/threads()
// return) — no DOM, no store, no clock.

function question(id: string, text: string, slug: string | null = null): ReflectQuestion {
  return { id, text, slug, addressee: 'self', cadence: 'daily', slot: null };
}

function reflect(clean: string, date: string, time: string, q: ReflectQuestion): EntryPayload {
  return { date, time, raw: null, clean, mode: 'reflect', question: q };
}

function journal(clean: string, date: string, time: string): EntryPayload {
  return { date, time, raw: null, clean, mode: 'journal', question: null };
}

describe('buildJournalView — empty state (R19)', () => {
  it('reports empty with copy when there are no entries', () => {
    const vm = buildJournalView([], []);
    expect(vm.isEmpty).toBe(true);
    expect(vm.emptyMessage.length).toBeGreaterThan(0);
    expect(vm.byDate).toEqual([]);
    expect(vm.byThread).toEqual([]);
  });

  it('groups with only empty entry lists still count as empty', () => {
    const vm = buildJournalView([['2026-06-13', []]], [['q', []]]);
    expect(vm.isEmpty).toBe(true);
  });
});

describe('buildJournalView — by date (R19)', () => {
  it('groups journal entries by date, newest date first', () => {
    const days: Array<[string, EntryPayload[]]> = [
      ['2026-06-11', [journal('older.', '2026-06-11', '09:00')]],
      ['2026-06-13', [journal('newer.', '2026-06-13', '08:00')]],
    ];
    const vm = buildJournalView(days, []);
    expect(vm.isEmpty).toBe(false);
    expect(vm.byDate.map((g) => g.date)).toEqual(['2026-06-13', '2026-06-11']);
  });

  it('orders entries within a day newest-time first', () => {
    const days: Array<[string, EntryPayload[]]> = [
      [
        '2026-06-13',
        [journal('morning.', '2026-06-13', '08:00'), journal('night.', '2026-06-13', '21:00')],
      ],
    ];
    const vm = buildJournalView(days, []);
    expect(vm.byDate[0].entries.map((e) => e.clean)).toEqual(['night.', 'morning.']);
  });
});

describe('buildJournalView — by reflect thread (R19)', () => {
  it('groups reflect entries into one thread per question id', () => {
    const q = question('avoidance', 'What am I avoiding?', 'what-am-i-avoiding');
    const threads: Array<[string, EntryPayload[]]> = [
      [
        'avoidance',
        [
          reflect('first answer.', '2026-06-10', '09:00', q),
          reflect('second answer.', '2026-06-13', '09:00', q),
        ],
      ],
    ];
    const vm = buildJournalView([], threads);
    expect(vm.byThread).toHaveLength(1);
    const t = vm.byThread[0];
    expect(t.questionId).toBe('avoidance');
    expect(t.questionText).toBe('What am I avoiding?');
    expect(t.label).toBe('what-am-i-avoiding');
    expect(t.entries.map((e) => e.clean)).toEqual(['first answer.', 'second answer.']);
  });

  it('orders threads by their most-recent entry, newest first', () => {
    const qa = question('a', 'A?');
    const qb = question('b', 'B?');
    const threads: Array<[string, EntryPayload[]]> = [
      ['a', [reflect('older thread touch.', '2026-06-10', '09:00', qa)]],
      ['b', [reflect('newer thread touch.', '2026-06-13', '09:00', qb)]],
    ];
    const vm = buildJournalView([], threads);
    expect(vm.byThread.map((t) => t.questionId)).toEqual(['b', 'a']);
  });

  it('falls back to the question id for label/text when the question is sparse', () => {
    // a thread whose entries carry no slug → label is the id
    const q = question('bare-id', 'Bare?', null);
    const vm = buildJournalView([], [['bare-id', [reflect('x.', '2026-06-13', '09:00', q)]]]);
    expect(vm.byThread[0].label).toBe('bare-id');
    expect(vm.byThread[0].questionText).toBe('Bare?');
  });
});

describe('continueThread — re-prompt the thread (R19)', () => {
  it('returns the question id so the router re-prompts that exact question', () => {
    const q = question('avoidance', 'What am I avoiding?');
    const vm = buildJournalView([], [['avoidance', [reflect('x.', '2026-06-13', '09:00', q)]]]);
    expect(continueThread(vm.byThread[0])).toBe('avoidance');
  });
});
