import { describe, it, expect, beforeAll } from 'vitest';
import {
  ModeRouter,
  InMemoryEntryStore,
  CLOSE_PHRASES,
  REFLECT_PACK_TOML,
  findQuestionInPack,
  type EntryPayload,
  type ModeClock,
  type ModeSession,
  type ReflectQuestion,
  type SessionFactory,
} from './modes';
import type { SessionMode } from '../mobile';
import { initWasmForTest } from '../wasm-test-init';

// The router logic is exercised DOM-free + deterministically: the clock is fixed
// (hour drives slot-aware selection; stamp drives the entry date/time), and the
// session is a recording fake. The reflect selection + closure lines run through
// the REAL talk-wasm (selectQuestion / composeReleased / composeClose), so the
// rotation + close-screen parity is asserted against the shared engine, not a
// TS re-implementation. (Real key/tap wiring is browser-only — deferred to e2e.)

beforeAll(async () => {
  await initWasmForTest();
});

// ── Injected fakes ───────────────────────────────────────────────────────────

function fixedClock(opts: { hour?: number; date?: string; time?: string; now?: number } = {}): ModeClock {
  const hour = opts.hour ?? 7; // morning by default (slot-aware)
  const date = opts.date ?? '2026-06-13';
  const time = opts.time ?? '08:14';
  const now = opts.now ?? 0;
  return {
    hour: () => hour,
    stamp: () => ({ date, time }),
    now: () => now,
  };
}

/** A recording fake session: returns scripted final text + cancel flag, and
 *  records whether `end()` (the wipe / free) was called. */
function fakeSession(opts: { clean?: string; raw?: string | null; cancelled?: boolean }): ModeSession & {
  readonly ended: boolean;
} {
  const state = { ended: false };
  return {
    finalClean: () => opts.clean ?? '',
    finalRaw: () => opts.raw ?? null,
    wasCancelled: () => opts.cancelled ?? false,
    end: () => {
      state.ended = true;
    },
    get ended() {
      return state.ended;
    },
  };
}

/** A factory that hands out a pre-built session per mode, recording each start. */
function recordingFactory(builder: (mode: SessionMode) => ModeSession): {
  factory: SessionFactory;
  starts: SessionMode[];
} {
  const starts: SessionMode[] = [];
  const factory: SessionFactory = (mode) => {
    starts.push(mode);
    return builder(mode);
  };
  return { factory, starts };
}

function makeRouter(opts: {
  store?: InMemoryEntryStore;
  clock?: ModeClock;
  builder?: (mode: SessionMode) => ModeSession;
  reduceMotion?: boolean;
} = {}): { router: ModeRouter; store: InMemoryEntryStore; starts: SessionMode[] } {
  const store = opts.store ?? new InMemoryEntryStore();
  const { factory, starts } = recordingFactory(
    opts.builder ?? (() => fakeSession({ clean: 'a kept reflection.' })),
  );
  const router = new ModeRouter({
    store,
    session: factory,
    clock: opts.clock ?? fixedClock(),
    reduceMotion: opts.reduceMotion ?? true, // short dwell so tests don't linger
  });
  return { router, store, starts };
}

function reflectEntry(id: string, clean: string, date = '2026-06-13'): EntryPayload {
  const question: ReflectQuestion = {
    id,
    text: 'q',
    slug: null,
    addressee: 'self',
    cadence: 'daily',
    slot: null,
  };
  return { date, time: '09:00', raw: null, clean, mode: 'reflect', question };
}

// ── Reflect front door ───────────────────────────────────────────────────────

describe('ModeRouter — reflect front door (R14)', () => {
  it('boots into reflect with a curated question shown as text', () => {
    // #given a freshly-constructed router (boots the front door)
    const { router } = makeRouter({ clock: fixedClock({ hour: 7 }) });
    // #then it is in a reflect session with a question selected
    expect(router.currentPhase()).toBe('session');
    expect(router.mode()).toBe('reflect');
    const q = router.questionText();
    expect(q).not.toBeNull();
    expect((q as string).length).toBeGreaterThan(0);
  });

  it('selects deterministically from the pack for a fixed morning hour', () => {
    // #given two fresh routers at the same morning hour with empty selection state
    const a = makeRouter({ clock: fixedClock({ hour: 7 }) }).router;
    const b = makeRouter({ clock: fixedClock({ hour: 7 }) }).router;
    // #then both pick the same first question (the rotation is deterministic)
    expect(a.questionText()).toBe(b.questionText());
    // #and a morning-slotted question is chosen (the spine's morning set)
    expect(a.reflectQuestion()?.slot).toBe('morning');
  });

  it('new-question before answering draws the NEXT question (re-roll)', () => {
    // #given the booted reflect front door
    const { router } = makeRouter({ clock: fixedClock({ hour: 7 }) });
    const first = router.reflectQuestion();
    // #when the user re-rolls before answering
    router.newQuestion();
    const second = router.reflectQuestion();
    // #then a different question is drawn (recency advanced past the first)
    expect(second).not.toBeNull();
    expect(second?.id).not.toBe(first?.id);
  });

  it(':skip / new-question via command() re-rolls too', () => {
    const { router } = makeRouter({ clock: fixedClock({ hour: 7 }) });
    const first = router.reflectQuestion()?.id;
    // #when the new-question verb is routed (the chip / :skip / n key path)
    expect(router.command('new-question')).toBe(true);
    expect(router.reflectQuestion()?.id).not.toBe(first);
  });

  it('repeat answers to ONE question append to a single thread (the seam U9 fills)', () => {
    // #given the in-memory store seam (U9 supplies the durable one behind it)
    const store = new InMemoryEntryStore();
    // #when two answers are kept for the SAME question id
    store.keep(reflectEntry('grateful-this-moment', 'first answer.'));
    store.keep(reflectEntry('grateful-this-moment', 'second answer.'));
    // #then both live in the one thread for that question
    const thread = store.thread('grateful-this-moment');
    expect(thread.length).toBe(2);
    expect(thread[0].clean).toBe('first answer.');
    expect(thread[1].clean).toBe('second answer.');
    // #and a different question keeps its own separate thread
    store.keep(reflectEntry('ready-to-let-go', 'other thread.'));
    expect(store.thread('ready-to-let-go').length).toBe(1);
  });

  it('a finished reflect session lands its entry in that question\'s thread', () => {
    const store = new InMemoryEntryStore();
    const { router } = makeRouter({
      store,
      clock: fixedClock({ hour: 7 }),
      builder: () => fakeSession({ clean: 'i am grateful for the quiet.' }),
    });
    const qid = router.reflectQuestion()?.id as string;
    // #when the booted reflect session is finished
    router.complete(false);
    // #then the entry landed in that exact question's thread
    expect(store.thread(qid).length).toBe(1);
    expect(store.thread(qid)[0].clean).toBe('i am grateful for the quiet.');
  });
});

// ── Journal ──────────────────────────────────────────────────────────────────

describe('ModeRouter — journal (R15)', () => {
  it('journal shows no question and appends under today\'s date', () => {
    const store = new InMemoryEntryStore();
    const clock = fixedClock({ date: '2026-06-13', time: '21:30' });
    const { router } = makeRouter({
      store,
      clock,
      builder: () => fakeSession({ clean: 'a free-form journal entry.' }),
    });
    // start at the picker after boot's reflect → switch to journal via the picker
    router.complete(true); // discard the booted reflect session → picker
    router.start('journal');
    // #then journal mode shows no question
    expect(router.mode()).toBe('journal');
    expect(router.questionText()).toBeNull();

    // #when the entry is finished
    router.complete(false);

    // #then it appended under today's date (no question / thread)
    const day = store.journalByDate.get('2026-06-13');
    expect(day?.length).toBe(1);
    expect(day?.[0].clean).toBe('a free-form journal entry.');
    expect(day?.[0].question).toBeNull();
    expect(store.threads.size).toBe(0);
  });
});

// ── Unburden (ephemeral) ─────────────────────────────────────────────────────

describe('ModeRouter — unburden keeps nothing + closure (R16, AE9)', () => {
  it('unburden end stores NOTHING, plays the release closure, wipes, returns to picker', () => {
    const store = new InMemoryEntryStore();
    const { router } = makeRouter({
      store,
      builder: () => fakeSession({ clean: 'something i needed to say out loud.' }),
    });
    router.complete(true); // leave the booted reflect → picker
    router.start('ephemeral');
    expect(router.mode()).toBe('ephemeral');
    const live = router.liveSession() as ModeSession & { ended: boolean };

    // #when the unburden session ends (release)
    router.complete(false);

    // #then nothing was persisted
    expect(store.threads.size).toBe(0);
    expect(store.journalByDate.size).toBe(0);
    // #and the buffers were wiped (this session's end was called)
    expect(live.ended).toBe(true);
    // #and the closure moment is the release line (composeReleased)
    expect(router.currentPhase()).toBe('closing');
    expect(router.closureLines()).toEqual(['Released. Nothing was written.']);

    // #and dismissing the closure returns to the between-session picker
    router.dismissClosure();
    expect(router.currentPhase()).toBe('picker');
  });
});

// ── Reflect / journal closure ────────────────────────────────────────────────

describe('ModeRouter — reflect/journal closure moment', () => {
  it('a kept reflect entry plays a close phrase + landing line (composeClose)', () => {
    const { router } = makeRouter({
      clock: fixedClock({ hour: 7, now: 3 }), // now=3 → CLOSE_PHRASES[3]
      builder: () => fakeSession({ clean: 'kept.' }),
    });
    // #when the booted reflect session is finished
    router.complete(false);
    // #then the closure shows the rotated close phrase
    expect(router.currentPhase()).toBe('closing');
    const lines = router.closureLines().join('\n');
    expect(lines).toContain(CLOSE_PHRASES[3]);
    // #and a landing/provenance line is present (composeClose's first line)
    expect(lines).toContain('reflection kept');
  });
});

// ── Between-session picker + cancel (R17) ────────────────────────────────────

describe('ModeRouter — between-session picker (R17)', () => {
  it('after done, the picker offers reflect / journal / unburden', () => {
    const { router } = makeRouter({ builder: () => fakeSession({ clean: 'x.' }) });
    router.complete(false); // finish the booted reflect
    router.dismissClosure(); // skip the closure → picker
    expect(router.currentPhase()).toBe('picker');
    expect(router.pickerOptions()).toEqual(['reflect', 'journal', 'ephemeral']);
    expect(router.mode()).toBeNull();
  });

  it('cancel discards (no entry kept) and returns straight to the picker', () => {
    const store = new InMemoryEntryStore();
    const { router } = makeRouter({ store, builder: () => fakeSession({ clean: 'unsaved.' }) });
    // #when the booted reflect session is cancelled (the host ran U7's confirm gate)
    router.complete(true);
    // #then nothing was kept and we are at the picker (no closure phrase)
    expect(store.threads.size).toBe(0);
    expect(router.currentPhase()).toBe('picker');
    expect(router.closureLines()).toEqual([]);
  });

  it('the picker starts the next session in the chosen mode', () => {
    const { router, starts } = makeRouter({ builder: () => fakeSession({ clean: 'x.' }) });
    router.complete(true); // boot reflect → picker
    router.start('journal');
    expect(router.mode()).toBe('journal');
    router.complete(false);
    router.dismissClosure();
    router.start('reflect');
    expect(router.mode()).toBe('reflect');
    // boot(reflect) + journal + reflect
    expect(starts).toEqual(['reflect', 'journal', 'reflect']);
  });
});

// ── Deep-link question lookup (U11) ───────────────────────────────────────────

describe('findQuestionInPack — deep-link id resolution', () => {
  it('resolves a known id to its full question from the pack TOML', () => {
    // #given a real spine id
    const q = findQuestionInPack(REFLECT_PACK_TOML, 'grateful-this-moment');
    // #then the question is resolved with its id + text (and the parsed fields)
    expect(q).not.toBeNull();
    expect(q?.id).toBe('grateful-this-moment');
    expect(q?.text).toBe('What are you grateful for in this moment?');
    expect(q?.slot).toBe('morning');
    // #and the defaults mirror parseQuestion when a field is absent
    expect(q?.addressee).toBe('self');
    expect(q?.cadence).toBe('daily');
  });

  it('returns null for an unknown (but alphabet-safe) id', () => {
    // #given an id that simply isn't in the pack
    // #then the lookup fails so the host falls back to normal selection
    expect(findQuestionInPack(REFLECT_PACK_TOML, 'not-a-real-question')).toBeNull();
  });

  it('does not let one block bleed into the next on a partial match', () => {
    // #given a real second id
    const q = findQuestionInPack(REFLECT_PACK_TOML, 'ready-to-let-go');
    // #then it resolves that exact question, not a neighbor's text
    expect(q?.id).toBe('ready-to-let-go');
    expect(q?.text).toBe('What are you ready to let go of?');
  });
});

describe('ModeRouter.startWithQuestion — the #q deep-link seam (U11)', () => {
  it('opens a known question in a reflect session and returns true', () => {
    const store = new InMemoryEntryStore();
    const { router } = makeRouter({ store, builder: () => fakeSession({ clean: 'x.' }) });
    // #when a known id is opened (the host applies this after parseHash)
    const opened = router.startWithQuestion('grateful-this-moment');
    // #then a reflect session runs bound to that exact question
    expect(opened).toBe(true);
    expect(router.mode()).toBe('reflect');
    expect(router.reflectQuestion()?.id).toBe('grateful-this-moment');
    // #and the serve was recorded so the rotation recency advances
    expect(store.selection().servedCount.get('grateful-this-moment')).toBeGreaterThanOrEqual(1);
  });

  it('is a no-op (returns false) for an unknown id — host keeps normal selection', () => {
    const { router } = makeRouter({ builder: () => fakeSession({ clean: 'x.' }) });
    const before = router.reflectQuestion()?.id;
    // #when an unknown id is opened
    const opened = router.startWithQuestion('definitely-not-in-the-pack');
    // #then nothing changed — the booted front-door question still stands
    expect(opened).toBe(false);
    expect(router.reflectQuestion()?.id).toBe(before);
  });
});
