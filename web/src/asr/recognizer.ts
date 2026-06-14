// The recognizer seam (R2) — the clean boundary between the two-pass sherpa-onnx
// ASR engine and the rest of the pipeline.
//
// The pipeline driver (pipeline.ts) talks ONLY to this interface. That keeps the
// privacy-critical settle/pairing driver, the hallucination gate, and the idle
// logic fully unit-testable today against a scripted `MockRecognizer`, while the
// real on-device engine plugs in behind `WireSherpaRecognizer` and is validated
// live. The mock is unambiguously a test/demo driver — it does NOT fake
// transcription QUALITY; it replays a supplied script so the pipeline animates
// end-to-end now.
//
// Two-pass shape (mirrors the CLI): a streaming Zipformer emits live `partial`
// hypotheses; a silence boundary fires `endpoint`; an offline Whisper base.en
// pass produces the higher-quality `finalize` text for the same audio segment.

/** A 16 kHz mono PCM frame (Int16), as the AudioWorklet emits it. */
export type Int16Frame = Int16Array;

/**
 * The result of the offline (pass-2) finalize over a settled audio segment.
 * `noSpeechProb` is Whisper's per-segment silence probability, when the engine
 * exposes it — the hallucination gate keys on it (high → the segment was likely
 * silence and the text is fabricated, so it is skipped). `null` means the signal
 * is unavailable and the driver falls back to its energy/duration heuristic.
 */
export interface Finalized {
  readonly text: string;
  readonly noSpeechProb: number | null;
}

/**
 * The recognizer events the driver subscribes to. A `partial` updates the live
 * edge; `endpoint` marks a pause/silence boundary (the live edge becomes the
 * committing block); `finalize` delivers the pass-2 upgrade for that segment.
 */
export interface RecognizerEvents {
  /** A new/revised live-edge hypothesis (pass-1 streaming Zipformer). */
  partial(text: string): void;
  /** A pause/silence boundary: the current edge is ready to commit. */
  endpoint(): void;
  /** The pass-2 (Whisper base.en offline) upgrade for the last segment. */
  finalize(result: Finalized): void;
}

/**
 * The recognizer seam. The driver pushes audio frames and receives partial /
 * endpoint / finalize events. `reset` clears in-flight state (used when the
 * session pauses off-record so pre-pause audio never reaches a kept entry).
 */
export interface Recognizer {
  /** Subscribe the driver's handlers. Returns `this` for chaining. */
  on(events: RecognizerEvents): Recognizer;
  /** Feed one 16 kHz Int16 frame to the streaming pass. */
  pushAudio(frame: Int16Frame): void;
  /** Discard the in-flight hypothesis + any segment audio (off-record pause). */
  reset(): void;
  /** Flush any trailing audio into a final endpoint+finalize (session end). */
  flush(): void;
  /** Release engine resources. */
  free(): void;
}

// ---------------------------------------------------------------------------
// (1) WireSherpaRecognizer — the real engine seam (NOT wired yet).
// ---------------------------------------------------------------------------

/**
 * The production recognizer: sherpa-onnx's Emscripten/WASM build running the
 * streaming Zipformer (pass-1 partials) + Whisper base.en offline (pass-2
 * finalize) on the cached, integrity-verified model files (U5).
 *
 * WIRE: this is the single integration seam. Completing it means:
 *
 *   1. Load the sherpa-onnx WASM module + stage the seven extracted model files
 *      (integrity.ts MODEL_FILES) into its Emscripten FS (or hand it the cached
 *      ArrayBuffers — U6 open question, confirmed at integration time).
 *        - streaming Zipformer-20M: `OnlineRecognizer` (encoder/decoder/joiner
 *          int8 + tokens) with endpoint detection enabled.
 *        - Whisper base.en: `OfflineRecognizer` (encoder/decoder int8 + tokens).
 *   2. In `pushAudio`: `online.acceptWaveform(16000, frame)`, drain
 *      `while (online.isReady()) online.decode()`, emit `partial(online.getResult().text)`.
 *      When sherpa reports an endpoint (`online.isEndpoint()`), emit `endpoint()`,
 *      reset the online stream, then run the offline pass over the buffered
 *      segment and emit `finalize({ text, noSpeechProb })`.
 *   3. `noSpeechProb`: forward Whisper's per-segment `no_speech_prob` IF the WASM
 *      build exposes it (the U6 open question). If not, return `null` so the
 *      driver's energy/duration heuristic gates instead.
 *
 * Reference loaders to copy (confirmed maintained sherpa-onnx WASM demos):
 *   - streaming Zipformer EN:
 *       https://huggingface.co/spaces/k2-fsa/web-assembly-asr-sherpa-onnx-en
 *   - VAD + offline (Whisper/Moonshine) — the finalize + endpoint pattern:
 *       https://huggingface.co/spaces/k2-fsa/web-assembly-vad-asr-sherpa-onnx-moonshine
 *
 * On mobile, the U1 decision selects base.en vs the Moonshine tiny finalizer
 * (R27) — that swap is a different OfflineRecognizer behind this same seam.
 *
 * Until wired, every method throws/logs "not wired" so it can never silently
 * pass empty transcription into a kept entry. `MockRecognizer` is what the page
 * and the tests use today.
 */
export class WireSherpaRecognizer implements Recognizer {
  private static readonly NOT_WIRED =
    'WireSherpaRecognizer is not wired — the sherpa-onnx WASM engine plugs in at ' +
    'the WIRE: seam in recognizer.ts. Use MockRecognizer until the engine is integrated.';

  on(_events: RecognizerEvents): Recognizer {
    return this;
  }

  pushAudio(_frame: Int16Frame): void {
    throw new Error(WireSherpaRecognizer.NOT_WIRED);
  }

  reset(): void {
    throw new Error(WireSherpaRecognizer.NOT_WIRED);
  }

  flush(): void {
    throw new Error(WireSherpaRecognizer.NOT_WIRED);
  }

  free(): void {
    // Idempotent no-op: freeing an un-wired engine is harmless.
  }
}

// ---------------------------------------------------------------------------
// (2) MockRecognizer — a scripted test/demo driver (NOT a real engine).
// ---------------------------------------------------------------------------

/** One scripted recognizer step. */
export type ScriptStep =
  | { readonly kind: 'partial'; readonly text: string }
  | { readonly kind: 'endpoint' }
  | { readonly kind: 'finalize'; readonly text: string; readonly noSpeechProb?: number | null };

/**
 * A scripted recognizer for tests + the live demo. It does NOT transcribe — it
 * replays the supplied `script` so the pipeline + settle + render path animate
 * and assert end-to-end without a mic or the real engine.
 *
 * Driving model: by default each `pushAudio` call advances ONE script step (so a
 * frame stream walks the script). Tests that want deterministic control over
 * timing call `step()`/`run()` directly and ignore `pushAudio`. `flush()` drains
 * the remaining steps (session end).
 */
export class MockRecognizer implements Recognizer {
  private handlers: RecognizerEvents | null = null;
  private index = 0;
  private readonly advanceOnAudio: boolean;

  constructor(
    private readonly script: readonly ScriptStep[],
    options: { readonly advanceOnAudio?: boolean } = {},
  ) {
    this.advanceOnAudio = options.advanceOnAudio ?? true;
  }

  on(events: RecognizerEvents): Recognizer {
    this.handlers = events;
    return this;
  }

  pushAudio(_frame: Int16Frame): void {
    if (this.advanceOnAudio) this.step();
  }

  /** Emit the next scripted step, if any. Returns false once the script is done. */
  step(): boolean {
    if (this.index >= this.script.length) return false;
    const s = this.script[this.index++];
    this.emit(s);
    return true;
  }

  /** Emit every remaining step in order (deterministic full-script playback). */
  run(): void {
    while (this.step()) {
      /* drain */
    }
  }

  reset(): void {
    // Off-record: drop any in-flight partial by clearing the live edge. The
    // script index is NOT rewound — a real engine drops audio, not the future.
    this.handlers?.partial('');
  }

  flush(): void {
    this.run();
  }

  free(): void {
    this.handlers = null;
  }

  private emit(step: ScriptStep): void {
    const h = this.handlers;
    if (!h) return;
    switch (step.kind) {
      case 'partial':
        h.partial(step.text);
        break;
      case 'endpoint':
        h.endpoint();
        break;
      case 'finalize':
        h.finalize({ text: step.text, noSpeechProb: step.noSpeechProb ?? null });
        break;
    }
  }
}
