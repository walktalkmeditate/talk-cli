// The two-pass ASR pipeline driver (U6) — the THIN seam tying audio capture →
// recognizer → the shared talk-wasm Settle + off-record Pairing machine.
//
// Ownership boundary (the privacy invariant): this driver owns NO drop/keep
// policy of its own. EVERY recognizer event is first routed through the wasm
// `Pairing.decide(kind)` — the privacy-critical guard shared verbatim with the
// CLI (talk_core::pairing) — and the driver then acts ONLY on the verdict
// ("done" | "drop" | "applyPartial" | "applyCommit" | "applyRevise"). Pause /
// resume / finish-drain are driven through the same machine's pause()/resume()/
// beginFinishDrain(). So off-record (paused) speech never reaches a kept entry,
// and the disarm-paired-Revise / straddling-Revise behavior is the CLI's, not a
// TS re-implementation. This mirrors src/live.rs::apply_event exactly.
//
// Mapping (CLI Event → recognizer event → pairing kind → Settle call):
//   partial  → "partial" → applyPartial → settle.onPartial(text)
//   endpoint → "commit"  → applyCommit  → settle.commit(rawEdge, cleanEdge)
//   finalize → "revise"  → applyRevise  → settle.reviseCommitting(raw, clean)
//
// The hallucination gate (R3 / the Whisper-invents-text-on-silence learning)
// guards ONLY the finalize/revise step: a high no-speech probability (or, absent
// that signal, an energy/duration heuristic over the segment's frames) skips the
// pass-2 swap so the live edge never fabricates words during a pause. The real
// no_speech_prob comes from the live engine, so it is keyed behind a clear seam.

import init, { deterministicLight, Pairing, Settle } from '../wasm/talk_wasm.js';
import type { Finalized, Int16Frame, Recognizer } from './recognizer';

/** A pairing decision string, as `Pairing.decide(kind)` returns it. */
type Verdict = 'done' | 'drop' | 'applyPartial' | 'applyCommit' | 'applyRevise';

/** The known pairing verdicts (the wasm `Pairing.decide` contract). */
const KNOWN_VERDICTS: ReadonlySet<string> = new Set<Verdict>([
  'done',
  'drop',
  'applyPartial',
  'applyCommit',
  'applyRevise',
]);

/**
 * Narrow a raw `Pairing.decide(...)` string to a `Verdict`, defending the wasm↔JS
 * boundary: an unrecognized verdict is treated as `'drop'` (the privacy-safe
 * default — an unknown decision must NEVER be applied, which could land off-record
 * text) and logged, rather than blindly cast. The wasm side already drops unknown
 * KINDS; this is the symmetric guard on the way back out.
 */
function toVerdict(s: string): Verdict {
  if (KNOWN_VERDICTS.has(s)) return s as Verdict;
  console.error(`pipeline: unknown pairing verdict ${JSON.stringify(s)} — dropping`);
  return 'drop';
}

/**
 * The hallucination gate's verdict on a finalize. Behind a clear seam: the real
 * signal is the live engine's `no_speech_prob`; when that is null the driver
 * falls back to an energy/duration heuristic over the segment's frames.
 */
export interface HallucinationGate {
  /** True → KEEP the finalize; false → SKIP it (likely fabricated on silence). */
  plausiblySpeech(result: Finalized, segment: SegmentStats): boolean;
}

/** Cheap stats over the audio that produced a segment (for the heuristic path). */
export interface SegmentStats {
  /** Total samples accumulated for the segment. */
  readonly samples: number;
  /** Mean absolute amplitude (0..1, Int16 normalized) over the segment. */
  readonly meanAbsAmplitude: number;
  /** Segment duration in milliseconds at 16 kHz. */
  readonly durationMs: number;
}

/**
 * Above this no-speech probability, Whisper's text is treated as fabricated on
 * silence and the finalize is skipped. Mirrors the CLI's suspect-hallucination
 * intent; the exact threshold is tuned against the live engine.
 */
const NO_SPEECH_PROB_CEILING = 0.6;
/** Below this mean amplitude over a segment, the audio reads as near-silence. */
const SILENCE_AMPLITUDE_FLOOR = 0.004;
/** Below this duration a "phrase" is too short to be real speech. */
const MIN_SPEECH_MS = 200;

/**
 * The default gate: prefer the engine's `no_speech_prob` when present; otherwise
 * fall back to an energy/duration heuristic. An empty finalize is always
 * skipped (nothing to settle).
 */
export const defaultHallucinationGate: HallucinationGate = {
  plausiblySpeech(result: Finalized, segment: SegmentStats): boolean {
    if (result.text.trim().length === 0) return false;
    if (result.noSpeechProb !== null) {
      return result.noSpeechProb < NO_SPEECH_PROB_CEILING;
    }
    // Heuristic seam: no engine signal — gate on energy + duration.
    return segment.durationMs >= MIN_SPEECH_MS && segment.meanAbsAmplitude >= SILENCE_AMPLITUDE_FLOOR;
  },
};

export interface PipelineOptions {
  readonly recognizer: Recognizer;
  /** The hallucination gate (defaults to the no-speech-prob/energy gate). */
  readonly gate?: HallucinationGate;
  /** Called after any state change so the host can repaint the live edge. */
  readonly onChange?: () => void;
}

/** Idle/listening status for the indicator (R3). */
export interface IdleStatus {
  /** True while the speech-hangover latch holds (suppresses indicator flicker). */
  readonly listening: boolean;
  /** True when there is a live-edge partial showing. */
  readonly hasEdge: boolean;
}

/** Speech-hangover latch (R3): keep "listening" lit briefly past the last edge
 *  update so a momentary gap between partials doesn't flicker the indicator.
 *  Mirrors src/live.rs SPEECH_HANGOVER (350 ms). */
const SPEECH_HANGOVER_MS = 350;

/**
 * The two-pass pipeline driver. Construct it with a recognizer (Mock today, the
 * sherpa seam once wired) and a Settle/Pairing pair; feed it audio frames; read
 * back the composed screen through `settle`.
 */
export class Pipeline {
  readonly settle: Settle;
  private readonly pairing: Pairing;
  private readonly recognizer: Recognizer;
  private readonly gate: HallucinationGate;
  private readonly onChange: (() => void) | undefined;

  // Per-segment accumulation for the heuristic gate + the raw/clean commit text.
  private segmentSamples = 0;
  private segmentAbsSum = 0;
  // Snapshot of the segment's stats taken AT the endpoint, BEFORE resetSegment()
  // wipes them — the pass-2 finalize's hallucination gate reads this (the live
  // edge has already moved on). Null until the first endpoint of a segment.
  private lastSegmentStats: SegmentStats | null = null;
  private edgeText = '';
  private lastEdgeAt = -Infinity;
  private ended = false;
  private now: () => number;

  constructor(opts: PipelineOptions, clock: () => number = () => Date.now()) {
    this.settle = new Settle();
    this.pairing = new Pairing();
    this.recognizer = opts.recognizer;
    this.gate = opts.gate ?? defaultHallucinationGate;
    this.onChange = opts.onChange;
    this.now = clock;

    this.recognizer.on({
      partial: (text) => this.onPartial(text),
      endpoint: () => this.onEndpoint(),
      finalize: (result) => this.onFinalize(result),
    });
  }

  /** Feed a 16 kHz Int16 frame: accumulates segment stats, then drives the
   *  recognizer. While paused the frame is dropped at the source (off-record). */
  pushAudio(frame: Int16Frame): void {
    if (this.pairing.isPaused() || this.ended) return;
    this.accumulate(frame);
    this.recognizer.pushAudio(frame);
  }

  private accumulate(frame: Int16Frame): void {
    this.segmentSamples += frame.length;
    let absSum = 0;
    for (let i = 0; i < frame.length; i++) {
      absSum += Math.abs(frame[i]);
    }
    this.segmentAbsSum += absSum;
  }

  private segmentStats(): SegmentStats {
    const samples = this.segmentSamples;
    const meanAbs = samples > 0 ? this.segmentAbsSum / samples / 0x8000 : 0;
    return { samples, meanAbsAmplitude: meanAbs, durationMs: (samples / 16000) * 1000 };
  }

  private resetSegment(): void {
    this.segmentSamples = 0;
    this.segmentAbsSum = 0;
  }

  // ── Recognizer event handlers — each routed through the Pairing guard. ──────

  private onPartial(text: string): void {
    const verdict = toVerdict(this.pairing.decide('partial'));
    if (verdict === 'applyPartial') {
      this.edgeText = text;
      this.lastEdgeAt = this.now();
      this.settle.onPartial(text);
    }
    // "drop" → off-record partial: do not advance the edge.
    this.onChange?.();
  }

  private onEndpoint(): void {
    // Snapshot the segment stats BEFORE the reset so the pass-2 finalize's
    // hallucination gate (heuristic path) reads the audio that produced THIS
    // segment, not the empty post-reset accumulator. Same root fixes the
    // straddling-revise-after-pause case (the finalize lands after the edge moved).
    this.lastSegmentStats = this.segmentStats();
    const verdict = toVerdict(this.pairing.decide('commit'));
    if (verdict === 'applyCommit') {
      const raw = this.edgeText;
      this.settle.commit(raw, deterministicLight(raw));
    }
    // "drop" → off-record commit: the pairing disarms its paired pass-2 Revise.
    this.edgeText = '';
    this.resetSegment();
    this.onChange?.();
  }

  private onFinalize(result: Finalized): void {
    const verdict = toVerdict(this.pairing.decide('revise'));
    if (verdict === 'applyRevise') {
      // Hallucination gate (R3): skip a finalize that reads as silence — keep the
      // dim pass-1 edge rather than letting the engine fabricate words. Use the
      // segment stats snapshotted at the endpoint (the live accumulator has been
      // reset for the next segment); fall back to a live read if no endpoint fired.
      const segment = this.lastSegmentStats ?? this.segmentStats();
      if (this.gate.plausiblySpeech(result, segment)) {
        this.settle.reviseCommitting(result.text, deterministicLight(result.text));
      }
    }
    // "drop" → the paired Commit was an off-record drop; the upgrade is discarded.
    this.onChange?.();
  }

  // ── Session controls — driven through the shared Pairing machine. ──────────

  /** Enter off-record (R4): clear the live edge, drop in-flight segment audio,
   *  and tell the recognizer to discard its hypothesis. Mirrors live.rs pause. */
  pause(): void {
    this.pairing.pause();
    this.recognizer.reset();
    this.edgeText = '';
    this.resetSegment();
    this.lastSegmentStats = null; // off-record audio's stats must not gate a later finalize
    this.settle.onPartial(''); // the change-only edge must stop advertising the stale partial
    this.onChange?.();
  }

  /** Leave off-record. Does NOT re-arm the pairing guard — only the next accepted
   *  commit does (an off-record Revise can still be in flight). */
  resume(): void {
    this.pairing.resume();
    this.onChange?.();
  }

  /** Whether the session is off-record (paused). Exposed so the Pipeline directly
   *  satisfies the session-controls surface without reaching into `pairing`. */
  isPaused(): boolean {
    return this.pairing.isPaused();
  }

  /** Whether the next pass-2 Revise will be dropped because its paired Commit was
   *  an off-record drop (mirrors `Pairing.commitDropped`). Exposed so tests +
   *  diagnostics read the privacy state through the Pipeline, not its private
   *  `pairing`. */
  commitDropped(): boolean {
    return this.pairing.commitDropped();
  }

  /**
   * End the session (R3): finish-drain through the pairing guard, flush any
   * trailing recognizer output, then finalize-or-drop the dim partial.
   *
   * Finalize-or-drop rule: an in-flight live-edge partial at session end is
   * committed (it is on-record), so it lands as settled text; if the edge is
   * empty there is nothing to drop. The finish-drain carries commit_dropped
   * forward so an in-flight Revise of a pause-dropped Commit is still dropped.
   */
  finish(): void {
    if (this.ended) return;
    this.pairing.beginFinishDrain();
    this.recognizer.flush();
    // Finalize-or-drop the dim partial: a non-empty edge is on-record at finish.
    if (this.edgeText.trim().length > 0) {
      const verdict = toVerdict(this.pairing.decide('commit'));
      if (verdict === 'applyCommit') {
        this.settle.commit(this.edgeText, deterministicLight(this.edgeText));
      }
      this.edgeText = '';
    }
    this.settle.finalize();
    this.ended = true;
    this.onChange?.();
  }

  /** The idle/listening status for the indicator (R3): the hangover latch holds
   *  "listening" briefly past the last edge update; a calm absence otherwise. */
  idleStatus(): IdleStatus {
    const sinceEdge = this.now() - this.lastEdgeAt;
    const listening = !this.pairing.isPaused() && !this.ended && sinceEdge < SPEECH_HANGOVER_MS;
    return { listening, hasEdge: this.edgeText.trim().length > 0 };
  }

  /** Release the recognizer + wasm resources. */
  free(): void {
    this.recognizer.free();
    this.settle.free();
    this.pairing.free();
  }
}

/** Ensure the talk-wasm module is instantiated (browser path). Tests use
 *  `initWasmForTest`; the app calls `init()` in main.ts before constructing a
 *  Pipeline. Re-exported here so a caller can `await ensureWasm()` standalone. */
export async function ensureWasm(): Promise<void> {
  await init();
}
