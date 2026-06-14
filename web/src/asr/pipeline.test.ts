import { describe, it, expect, beforeAll } from 'vitest';
import { deterministicLight } from '../wasm/talk_wasm.js';
import { initWasmForTest } from '../wasm-test-init';
import { Pipeline, defaultHallucinationGate, type SegmentStats } from './pipeline';
import { MockRecognizer, type Finalized, type ScriptStep } from './recognizer';

// These tests drive the REAL talk-wasm Pairing + Settle through the TS pipeline
// driver — the privacy-critical path. The recognizer is a scripted MockRecognizer
// (no real engine), so the settle/render/off-record behavior is fully exercised.

beforeAll(async () => {
  await initWasmForTest();
});

/** Build a pipeline over a script the test drives via `recognizer.run()`/`step()`. */
function pipelineWith(
  script: readonly ScriptStep[],
  clock: () => number = () => 0,
): { pipeline: Pipeline; recognizer: MockRecognizer } {
  const recognizer = new MockRecognizer(script, { advanceOnAudio: false });
  const pipeline = new Pipeline({ recognizer }, clock);
  return { pipeline, recognizer };
}

/** The settled clean text, joined by spaces (what a kept entry would contain). */
function settled(pipeline: Pipeline): string {
  return pipeline.settle
    .settledText()
    .split('\n')
    .filter((s) => s.length > 0)
    .join(' ');
}

describe('deterministicLight — Whisper sound-tag stripping (CLI parity)', () => {
  it('strips non-speech tags the live engine emits, but keeps real words', () => {
    // #given Whisper's verbatim transcript with non-speech annotations
    // #when the wasm Light cleanup runs (the web clean path)
    // #then the tags are gone and the spoken words survive, cased + terminated
    expect(deterministicLight('i woke up [BLANK_AUDIO] early')).toBe('I woke up early.');
    expect(deterministicLight('(Keyboard clicking) i kept typing')).toBe('I kept typing.');
    // a tag-only segment cleans to nothing — never a lone terminal period
    expect(deterministicLight('[BLANK_AUDIO]')).toBe('');
  });
});

describe('Pipeline — partial → endpoint → finalize (the happy path, AE1)', () => {
  it('updates the live edge on a partial', () => {
    // #given a pipeline with a single partial
    const { pipeline, recognizer } = pipelineWith([{ kind: 'partial', text: 'i think i have been' }]);
    // #when the recognizer emits the partial
    recognizer.step();
    // #then the live edge shows it (dim, not yet settled)
    expect(pipeline.settle.edgeText()).toBe('i think i have been');
    expect(settled(pipeline)).toBe('');
    pipeline.free();
  });

  it('settles to bright final text on endpoint + finalize', () => {
    // #given partials, an endpoint, then a pass-2 finalize
    const { pipeline, recognizer } = pipelineWith([
      { kind: 'partial', text: 'um i think i have been holding my breath' },
      { kind: 'endpoint' },
      { kind: 'finalize', text: 'I think I have been holding my breath all week.', noSpeechProb: 0.02 },
    ]);
    // #when the full script plays
    recognizer.run();
    // #then the committing block is revised (final-quality → bright) and reads cleanly
    expect(pipeline.settle.committingRevised()).toBe(true);
    expect(pipeline.settle.committingText()).toBe('I think I have been holding my breath all week.');
    pipeline.free();
  });
});

describe('Pipeline — off-record privacy invariant (AE2)', () => {
  it('drops a paused commit + its paired revise; the settled text is unchanged', () => {
    // #given a settled on-record phrase
    const { pipeline, recognizer } = pipelineWith([
      { kind: 'partial', text: 'this is on the record' },
      { kind: 'endpoint' },
      { kind: 'finalize', text: 'This is on the record.', noSpeechProb: 0.01 },
      // ── off-record from here ──
      { kind: 'partial', text: 'a private aside i never want kept' },
      { kind: 'endpoint' },
      { kind: 'finalize', text: 'A private aside I never want kept.', noSpeechProb: 0.01 },
    ]);
    // play the on-record portion + finalize, settle it
    recognizer.step(); // partial
    recognizer.step(); // endpoint
    recognizer.step(); // finalize
    pipeline.settle.finalize(); // promote the committing block to settled
    const before = settled(pipeline);
    expect(before).toBe('This is on the record.');

    // #when the session goes off-record and the paused commit + paired revise arrive
    pipeline.pause();
    recognizer.step(); // off-record partial — dropped
    recognizer.step(); // off-record endpoint (commit) — dropped, disarms the revise
    recognizer.step(); // off-record finalize (revise) — dropped (paired commit was off-record)

    // #then nothing off-record reached the kept entry
    pipeline.settle.finalize();
    expect(settled(pipeline)).toBe('This is on the record.');
    expect(pipeline.commitDropped()).toBe(true);
    pipeline.free();
  });

  it('upgrades a STRADDLING revise (on-record commit, pass-2 lands during pause)', () => {
    // #given an on-record commit, then a pause BEFORE its pass-2 finalize lands
    const { pipeline, recognizer } = pipelineWith([
      { kind: 'partial', text: 'a straddling phrase committed on the record' },
      { kind: 'endpoint' },
      { kind: 'finalize', text: 'A straddling phrase committed on the record.', noSpeechProb: 0.01 },
    ]);
    recognizer.step(); // partial
    recognizer.step(); // endpoint → on-record commit (re-arms the pairing)

    // #when the session pauses, THEN the straddling pass-2 revise lands
    pipeline.pause();
    recognizer.step(); // finalize → revise: NOT disarmed (its commit was on-record)

    // #then the straddling revise still upgrades the committing block
    expect(pipeline.settle.committingRevised()).toBe(true);
    expect(pipeline.settle.committingText()).toBe('A straddling phrase committed on the record.');
    pipeline.free();
  });

  it('resumes listening after a pause (an accepted commit re-arms the guard)', () => {
    const { pipeline, recognizer } = pipelineWith([
      { kind: 'partial', text: 'before the pause' },
      { kind: 'endpoint' },
      { kind: 'partial', text: 'after resume' },
      { kind: 'endpoint' },
    ]);
    recognizer.step(); // partial
    recognizer.step(); // endpoint → on-record commit
    pipeline.pause();
    // an off-record commit would set commit_dropped; here we just resume cleanly
    pipeline.resume();
    expect(pipeline.isPaused()).toBe(false);
    recognizer.step(); // partial (on-record again)
    recognizer.step(); // endpoint → accepted commit re-arms the guard
    expect(pipeline.commitDropped()).toBe(false);
    pipeline.free();
  });
});

describe('Pipeline — hallucination gate (R3)', () => {
  it('skips a finalize with a high no-speech probability', () => {
    // #given an endpoint then a finalize the engine flags as silence
    const { pipeline, recognizer } = pipelineWith([
      { kind: 'partial', text: 'real speech here' },
      { kind: 'endpoint' },
      { kind: 'finalize', text: 'Thanks for watching!', noSpeechProb: 0.97 },
    ]);
    // #when the script plays
    recognizer.run();
    // #then the dim pass-1 edge is kept (no revise) — the fabricated text is skipped
    expect(pipeline.settle.committingRevised()).toBe(false);
    expect(pipeline.settle.committingText()).toBe(deterministicLight('real speech here'));
    pipeline.free();
  });

  it('keeps a finalize with a low no-speech probability', () => {
    const { pipeline, recognizer } = pipelineWith([
      { kind: 'partial', text: 'real speech here' },
      { kind: 'endpoint' },
      { kind: 'finalize', text: 'Real speech here.', noSpeechProb: 0.05 },
    ]);
    recognizer.run();
    expect(pipeline.settle.committingRevised()).toBe(true);
    pipeline.free();
  });

  it('default gate falls back to energy/duration when no-speech-prob is absent', () => {
    // #given the engine exposes no no_speech_prob
    const silentShort: Finalized = { text: 'Hi.', noSpeechProb: null };
    const realSpeech: Finalized = { text: 'A real reflection.', noSpeechProb: null };
    const silentSegment: SegmentStats = { samples: 1600, meanAbsAmplitude: 0.0005, durationMs: 100 };
    const speechSegment: SegmentStats = { samples: 48000, meanAbsAmplitude: 0.05, durationMs: 3000 };
    // #then near-silence/too-short is gated out, real speech is kept
    expect(defaultHallucinationGate.plausiblySpeech(silentShort, silentSegment)).toBe(false);
    expect(defaultHallucinationGate.plausiblySpeech(realSpeech, speechSegment)).toBe(true);
  });

  it('default gate always skips an empty finalize', () => {
    const empty: Finalized = { text: '   ', noSpeechProb: 0.01 };
    const loud: SegmentStats = { samples: 48000, meanAbsAmplitude: 0.2, durationMs: 3000 };
    expect(defaultHallucinationGate.plausiblySpeech(empty, loud)).toBe(false);
  });

  it('heuristic path (no no-speech-prob) reads the segment snapshotted AT the endpoint', () => {
    // #given a gate that records the segment stats it was handed — and a finalize
    // with NO engine signal, so the heuristic (segment) path is taken.
    let seen: SegmentStats | null = null;
    const recordingGate = {
      plausiblySpeech: (_r: Finalized, segment: SegmentStats): boolean => {
        seen = segment;
        return true;
      },
    };
    const recognizer = new MockRecognizer(
      [
        { kind: 'partial', text: 'real speech here' },
        { kind: 'endpoint' },
        { kind: 'finalize', text: 'Real speech here.', noSpeechProb: null },
      ],
      { advanceOnAudio: false },
    );
    const pipeline = new Pipeline({ recognizer, gate: recordingGate });

    // #when speech audio is fed, then the partial → endpoint → finalize plays
    pipeline.pushAudio(new Int16Array(16000).fill(8000)); // ~1s of loud audio
    recognizer.step(); // partial
    recognizer.step(); // endpoint → snapshots the segment, then resets the accumulator
    recognizer.step(); // finalize → reads the SNAPSHOT, not the post-reset zero

    // #then the gate saw the real (pre-reset) segment, so the upgrade landed
    expect(seen).not.toBeNull();
    expect(seen!.samples).toBe(16000);
    expect(pipeline.settle.committingRevised()).toBe(true);
    expect(pipeline.settle.committingText()).toBe(deterministicLight('Real speech here.'));
    pipeline.free();
  });
});

describe('Pipeline — idle / session end (R3)', () => {
  it('latches "listening" briefly past the last edge update, then goes calm', () => {
    let t = 0;
    const { pipeline, recognizer } = pipelineWith([{ kind: 'partial', text: 'speaking now' }], () => t);
    recognizer.step(); // partial at t=0
    // #then within the hangover window it reads as listening
    t = 100;
    expect(pipeline.idleStatus().listening).toBe(true);
    // #and past the hangover latch it goes calm
    t = 500;
    expect(pipeline.idleStatus().listening).toBe(false);
    pipeline.free();
  });

  it('finalize-or-drop at session end: a dangling partial is committed (on-record)', () => {
    // #given a live partial with no endpoint (the user pressed done mid-phrase)
    const { pipeline, recognizer } = pipelineWith([{ kind: 'partial', text: 'an unfinished thought' }]);
    recognizer.step();
    expect(pipeline.settle.edgeText()).toBe('an unfinished thought');
    // #when the session ends
    pipeline.finish();
    // #then the dangling partial finalized into settled text (on-record at finish)
    expect(settled(pipeline)).toBe(deterministicLight('an unfinished thought'));
    pipeline.free();
  });

  it('finalize-or-drop at session end: an empty edge drops cleanly (nothing kept)', () => {
    const { pipeline } = pipelineWith([]);
    pipeline.finish();
    expect(settled(pipeline)).toBe('');
    pipeline.free();
  });

  it('not listening once the session has ended', () => {
    const { pipeline } = pipelineWith([]);
    pipeline.finish();
    expect(pipeline.idleStatus().listening).toBe(false);
    pipeline.free();
  });
});

describe('Pipeline — pushAudio gating', () => {
  it('drops audio frames while paused (off-record at the source)', () => {
    let stepped = 0;
    const recognizer = new MockRecognizer([{ kind: 'partial', text: 'x' }], { advanceOnAudio: true });
    const origStep = recognizer.step.bind(recognizer);
    recognizer.step = () => {
      stepped++;
      return origStep();
    };
    const pipeline = new Pipeline({ recognizer });
    pipeline.pause();
    pipeline.pushAudio(new Int16Array(160));
    expect(stepped).toBe(0); // paused → no frame reached the recognizer
    pipeline.resume();
    pipeline.pushAudio(new Int16Array(160));
    expect(stepped).toBe(1);
    pipeline.free();
  });
});
