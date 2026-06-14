import { describe, it, expect, beforeAll } from 'vitest';
import { SessionControls, type SessionActions } from './controls';
import { chipsFor, cleanupForMode, type Chip } from '../mobile';
import { shapeEntry } from '../wasm/talk_wasm.js';
import { initWasmForTest } from '../wasm-test-init';

// These tests drive the control LOGIC DOM-free: the Pipeline is replaced by a
// fake SessionActions that records the calls, so space→finish, p→pause/resume,
// u→toggle raw/clean, and esc→confirm-then-cancel are asserted without xterm or
// a browser. (Real key events in xterm + matchMedia/touch are browser-only and
// deferred to an e2e pass; the verbs they dispatch are exercised here.)

/** A recording fake pipeline — counts the actions the controller drives. */
function fakePipeline(): SessionActions & {
  paused: boolean;
  calls: { pause: number; resume: number; finish: number };
} {
  const calls = { pause: 0, resume: 0, finish: 0 };
  let paused = false;
  return {
    calls,
    get paused() {
      return paused;
    },
    set paused(v: boolean) {
      paused = v;
    },
    pause() {
      paused = true;
      calls.pause++;
    },
    resume() {
      paused = false;
      calls.resume++;
    },
    finish() {
      calls.finish++;
    },
    isPaused() {
      return paused;
    },
  };
}

const ESC = '\x1b';

describe('SessionControls — keys → pipeline actions', () => {
  it('space → finish', () => {
    // #given a controller over a fake pipeline
    const pipeline = fakePipeline();
    const controls = new SessionControls({ pipeline });
    // #when space is pressed
    controls.onKey(' ');
    // #then the pipeline finishes and the session is finished (not cancelled)
    expect(pipeline.calls.finish).toBe(1);
    expect(controls.state().finished).toBe(true);
    expect(controls.state().cancelled).toBe(false);
  });

  it('p → pause, then p → resume (drives pipeline.pause / resume)', () => {
    // #given a controller over a fake pipeline
    const pipeline = fakePipeline();
    const controls = new SessionControls({ pipeline });
    // #when p is pressed once
    controls.onKey('p');
    // #then the pipeline pauses and the state reflects off-record
    expect(pipeline.calls.pause).toBe(1);
    expect(controls.state().paused).toBe(true);
    // #when p is pressed again
    controls.onKey('p');
    // #then the pipeline resumes and listening is restored
    expect(pipeline.calls.resume).toBe(1);
    expect(controls.state().paused).toBe(false);
  });

  it('u → toggles raw ⇄ clean state', () => {
    // #given a controller (clean by default)
    const pipeline = fakePipeline();
    const controls = new SessionControls({ pipeline });
    expect(controls.state().showRaw).toBe(false);
    // #when u is pressed
    controls.onKey('u');
    // #then it shows raw
    expect(controls.state().showRaw).toBe(true);
    // #when u is pressed again
    controls.onKey('u');
    // #then it shows clean again
    expect(controls.state().showRaw).toBe(false);
  });
});

describe('SessionControls — esc → confirm-cancel → cancel', () => {
  it('esc arms the discard prompt; a second esc confirms the cancel', () => {
    // #given a (non-ephemeral) controller
    const pipeline = fakePipeline();
    const controls = new SessionControls({ pipeline });
    // #when esc is pressed once
    controls.onKey(ESC);
    // #then the discard prompt is showing — NOT yet cancelled
    expect(controls.state().confirmCancel).toBe(true);
    expect(controls.state().finished).toBe(false);
    // #when esc is pressed again (confirm)
    controls.onKey(ESC);
    // #then the session is cancelled (entry discarded)
    expect(controls.state().cancelled).toBe(true);
    expect(controls.state().finished).toBe(true);
    expect(controls.state().confirmCancel).toBe(false);
  });

  it('y confirms the discard from the confirm-cancel prompt', () => {
    const pipeline = fakePipeline();
    const controls = new SessionControls({ pipeline });
    controls.onKey(ESC); // arm
    controls.onKey('y'); // confirm
    expect(controls.state().cancelled).toBe(true);
    expect(controls.state().finished).toBe(true);
  });

  it('any other key during confirm-cancel resumes the session (no discard)', () => {
    const pipeline = fakePipeline();
    const controls = new SessionControls({ pipeline });
    controls.onKey(ESC); // arm the prompt
    expect(controls.state().confirmCancel).toBe(true);
    controls.onKey('n'); // decline
    expect(controls.state().confirmCancel).toBe(false);
    expect(controls.state().cancelled).toBe(false);
    expect(controls.state().finished).toBe(false);
  });

  it('ephemeral (unburden) cancels IMMEDIATELY — no confirm prompt', () => {
    // #given an ephemeral controller
    const pipeline = fakePipeline();
    const controls = new SessionControls({ pipeline, ephemeral: true });
    // #when esc is pressed
    controls.onKey(ESC);
    // #then it cancels at once (nothing was at risk)
    expect(controls.state().confirmCancel).toBe(false);
    expect(controls.state().cancelled).toBe(true);
    expect(controls.state().finished).toBe(true);
  });
});

describe('SessionControls — focus-independent + chip parity', () => {
  it('spacebar with the edge area unfocused still triggers done', () => {
    // The controller takes the raw key chunk via onKey regardless of which DOM
    // element is focused — main.ts feeds it from term.onData with the terminal
    // kept focused. So a space always reaches `done`, focus notwithstanding.
    const pipeline = fakePipeline();
    const controls = new SessionControls({ pipeline });
    controls.onKey(' ');
    expect(pipeline.calls.finish).toBe(1);
    expect(controls.state().finished).toBe(true);
  });

  it('a chip command drives the same action as its key (done)', () => {
    const pipeline = fakePipeline();
    const controls = new SessionControls({ pipeline });
    // #when the 'done' chip command is dispatched
    controls.command('done');
    // #then it finishes exactly like the space key
    expect(pipeline.calls.finish).toBe(1);
  });

  it('a chip command drives pause/resume like the p key', () => {
    const pipeline = fakePipeline();
    const controls = new SessionControls({ pipeline });
    controls.command('pause');
    expect(controls.state().paused).toBe(true);
    controls.command('pause');
    expect(controls.state().paused).toBe(false);
  });

  it('ignores keys once the session is finished', () => {
    const pipeline = fakePipeline();
    const controls = new SessionControls({ pipeline });
    controls.onKey(' '); // finish
    controls.onKey('p'); // should be ignored
    expect(pipeline.calls.pause).toBe(0);
    expect(controls.state().paused).toBe(false);
  });
});

describe('chipsFor — per-mode chip sets (U7 enumeration)', () => {
  const commands = (chips: readonly Chip[]): string[] => chips.map((c) => c.command);

  it('reflect-session — done · pause · raw⇄clean · new-question · cancel', () => {
    expect(commands(chipsFor('reflect-session'))).toEqual([
      'done',
      'pause',
      'toggle-raw',
      'new-question',
      'cancel',
    ]);
  });

  it('journal-session — done · pause · raw⇄clean · cancel', () => {
    expect(commands(chipsFor('journal-session'))).toEqual(['done', 'pause', 'toggle-raw', 'cancel']);
  });

  it('unburden-session — done(release) · pause · cancel', () => {
    const chips = chipsFor('unburden-session');
    expect(commands(chips)).toEqual(['done', 'pause', 'cancel']);
    // the done chip reads "release" in unburden, but dispatches the same command
    expect(chips[0].label).toBe('release');
  });

  it('journal-view — new-entry · export · back', () => {
    expect(commands(chipsFor('journal-view'))).toEqual(['new-entry', 'export', 'back']);
  });

  it('every session chip set contains a done and a cancel', () => {
    for (const screen of ['reflect-session', 'journal-session', 'unburden-session'] as const) {
      const cmds = commands(chipsFor(screen));
      expect(cmds).toContain('done');
      expect(cmds).toContain('cancel');
    }
  });
});

describe('cleanupForMode — journal=High, reflect/unburden=Light (clean/raw parity)', () => {
  beforeAll(async () => {
    await initWasmForTest();
  });

  it('journal defaults to High, reflect + unburden default to Light', () => {
    expect(cleanupForMode('journal')).toBe('High');
    expect(cleanupForMode('reflect')).toBe('Light');
    expect(cleanupForMode('ephemeral')).toBe('Light');
  });

  it("a journal entry's clean view matches shapeEntry('High', …) — paragraph parity", () => {
    // #given a multi-sentence journal entry
    const text =
      'i went for a walk this morning. the air was cold and bright. ' +
      'i kept thinking about the deadline. but the walk helped me let it go.';
    // #then the clean view (the cleanup level the u-toggle renders) paragraphizes
    // exactly as the CLI export does for High.
    const clean = shapeEntry(cleanupForMode('journal'), text);
    expect(clean).toBe(shapeEntry('High', text));
    // #and High actually transforms the text (paragraphs), unlike Light pass-through
    expect(shapeEntry('High', text)).not.toBe(shapeEntry('Light', text));
  });
});
