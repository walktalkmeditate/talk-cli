// TransformersRecognizer — a real browser-native speech engine behind the
// `Recognizer` seam (recognizer.ts), backed by transformers.js Whisper running
// in a Web Worker (transformers-worker.ts).
//
// Division of labor:
//   - The WORKER owns the model + raw inference (load + transcribe a window).
//   - THIS class owns the audio buffer, the energy-based VAD/endpoint, the
//     debounced "re-transcribe the live window" cadence, and the mapping from
//     worker messages → the seam's partial / endpoint / finalize events.
//
// Why an energy VAD here (not in the worker): transformers.js Whisper exposes no
// streaming endpoint signal and no `no_speech_prob`, so the driver's
// hallucination gate falls back to its energy/duration heuristic (pipeline.ts).
// We mirror that idea at the source: speech accumulates a window; ~700 ms of
// silence after speech fires an endpoint; the live window is re-transcribed on a
// debounce so the edge updates while you talk. `finalize` carries
// `noSpeechProb: null` — the engine has no such signal, the heuristic gates.

import type { Finalized, Int16Frame, Recognizer, RecognizerEvents } from './recognizer';
import {
  type EnginePlacement,
  type WorkerLike,
  type WorkerOutbound,
} from './transformers-protocol';

/** Default English base model (~145 MB). Swap to `whisper-tiny.en` for a
 *  smaller/faster engine via the recognizer option / the `?model=` query param. */
export const DEFAULT_MODEL_ID = 'onnx-community/whisper-base.en';

/** 16 kHz, so 1 ms = 16 samples. */
const SAMPLE_RATE = 16000;

/** Mean |amplitude| (Int16-normalized, 0..1) below which a frame reads as
 *  silence. Mirrors pipeline.ts SILENCE_AMPLITUDE_FLOOR. */
const SILENCE_AMPLITUDE_FLOOR = 0.004;

/** Silence after speech that closes a segment (endpoint), in ms. */
const ENDPOINT_SILENCE_MS = 700;

/** Re-transcribe the live window at most this often while speaking, in ms. */
const PARTIAL_DEBOUNCE_MS = 500;

/** Cap the rolling window Whisper sees (its receptive field is ~30 s). Older
 *  audio is evicted so a long monologue can't grow the buffer unbounded. */
const MAX_WINDOW_MS = 30000;
const MAX_WINDOW_SAMPLES = (MAX_WINDOW_MS / 1000) * SAMPLE_RATE;

/** A speech segment that never produced this many ms of audio is too short to be
 *  real speech — its endpoint is dropped (mirrors pipeline.ts MIN_SPEECH_MS). */
const MIN_SPEECH_MS = 200;

/** Surfaced to the host so the boot flow can show a "loading model…" line. */
export interface ModelLoadStatus {
  readonly phase: 'loading' | 'downloading' | 'ready' | 'error';
  /** A user-facing line for the status surface. */
  readonly message: string;
  /** Download fraction in [0,1] when known (the `downloading` phase). */
  readonly fraction: number | null;
  /** Where the model runs, once known (the `ready` phase). */
  readonly device?: EnginePlacement;
}

export interface TransformersRecognizerOptions {
  /** HF model id. Defaults to `whisper-base.en`. */
  readonly modelId?: string;
  /** Model-load + download progress, for the host's loading UI. */
  readonly onModelStatus?: (status: ModelLoadStatus) => void;
  /** Injected worker factory — the test seam (a fake worker). Defaults to
   *  spawning the real transformers worker via Vite's worker-URL form. */
  readonly workerFactory?: () => WorkerLike;
  /** Injected clock (defaults to performance.now) for the debounce/endpoint. */
  readonly now?: () => number;
}

/** Spawn the real inference worker (the browser/default path). Kept out of the
 *  class so the `new Worker(new URL(...))` form (which Vite statically rewrites)
 *  is never reached under vitest, where it has no module graph for the worker. */
function spawnDefaultWorker(): WorkerLike {
  return new Worker(new URL('./transformers-worker.ts', import.meta.url), {
    type: 'module',
  }) as unknown as WorkerLike;
}

export class TransformersRecognizer implements Recognizer {
  private handlers: RecognizerEvents | null = null;
  private readonly worker: WorkerLike;
  private readonly modelId: string;
  private readonly onModelStatus: ((status: ModelLoadStatus) => void) | undefined;
  private readonly now: () => number;

  // The rolling Float32 window for the current segment + a running energy read.
  private window: Float32Array[] = [];
  private windowSamples = 0;
  private segmentAbsSum = 0;
  private segmentSamples = 0;
  private hadSpeech = false;
  private lastSpeechAt = -Infinity;
  private lastPartialAt = -Infinity;
  private ready = false;
  // A monotonically increasing id so a `final` response is paired to the segment
  // that requested it (a stale partial that lands after an endpoint is ignored).
  private segmentId = 0;

  constructor(opts: TransformersRecognizerOptions = {}) {
    this.modelId = opts.modelId ?? DEFAULT_MODEL_ID;
    this.onModelStatus = opts.onModelStatus;
    this.now = opts.now ?? (() => performance.now());
    this.worker = (opts.workerFactory ?? spawnDefaultWorker)();
    this.worker.onmessage = (event) => this.onWorkerMessage(event.data);
    this.worker.onerror = (event) => {
      this.onModelStatus?.({
        phase: 'error',
        message: 'speech engine worker crashed',
        fraction: null,
      });
      console.error('transformers worker error', event);
    };
    this.onModelStatus?.({ phase: 'loading', message: 'loading speech model…', fraction: null });
    this.worker.postMessage({ type: 'load', modelId: this.modelId });
  }

  on(events: RecognizerEvents): Recognizer {
    // Single handler set, replace-not-stack (the seam contract).
    this.handlers = events;
    return this;
  }

  /** Feed one 16 kHz Int16 frame: convert to Float32, append to the window,
   *  track energy, and drive the partial-debounce + silence-endpoint. */
  pushAudio(frame: Int16Frame): void {
    const f32 = int16ToFloat32(frame);
    this.appendToWindow(f32);

    const meanAbs = frameMeanAbs(frame);
    const speaking = meanAbs >= SILENCE_AMPLITUDE_FLOOR;
    const t = this.now();

    if (speaking) {
      this.hadSpeech = true;
      this.lastSpeechAt = t;
      this.segmentAbsSum += meanAbs * frame.length;
      this.segmentSamples += frame.length;
      if (this.ready && t - this.lastPartialAt >= PARTIAL_DEBOUNCE_MS) {
        this.lastPartialAt = t;
        this.requestTranscribe(false);
      }
      return;
    }

    // Silence: once we've heard speech, a long-enough gap closes the segment.
    if (this.hadSpeech && t - this.lastSpeechAt >= ENDPOINT_SILENCE_MS) {
      this.closeSegment();
    }
  }

  /** Discard the in-flight window + segment state — SILENTLY (the seam contract:
   *  `reset` must not emit events). Used on off-record pause so pre-pause audio
   *  never reaches a kept entry. The segment id advances so any in-flight worker
   *  response for the dropped segment is ignored. */
  reset(): void {
    this.clearSegment();
    this.segmentId++;
  }

  /** Force a final pass over whatever is buffered (session end). Emits an
   *  endpoint + finalize iff there is real speech buffered; otherwise nothing. */
  flush(): void {
    if (this.hadSpeech) this.closeSegment();
  }

  /** Terminate the worker (release the model + GPU/WASM resources). */
  free(): void {
    this.handlers = null;
    this.worker.terminate();
  }

  // ── internals ──────────────────────────────────────────────────────────────

  private appendToWindow(f32: Float32Array): void {
    this.window.push(f32);
    this.windowSamples += f32.length;
    // Evict oldest chunks past the ~30 s receptive field.
    while (this.windowSamples > MAX_WINDOW_SAMPLES && this.window.length > 1) {
      const dropped = this.window.shift();
      if (dropped) this.windowSamples -= dropped.length;
    }
  }

  /** Close the current speech segment: emit `endpoint()`, request the pass-2
   *  final transcription over the buffered window, then clear for the next one.
   *  A segment with too little real-speech audio is dropped (no endpoint). */
  private closeSegment(): void {
    const speechMs = (this.segmentSamples / SAMPLE_RATE) * 1000;
    if (speechMs < MIN_SPEECH_MS) {
      this.clearSegment();
      return;
    }
    this.handlers?.endpoint();
    if (this.ready) this.requestTranscribe(true);
    // Bump the id AFTER the final request so its response still pairs; the next
    // segment's partials use the new id and a late stale partial is ignored.
    this.clearSegment();
  }

  private clearSegment(): void {
    this.window = [];
    this.windowSamples = 0;
    this.segmentAbsSum = 0;
    this.segmentSamples = 0;
    this.hadSpeech = false;
    this.lastSpeechAt = -Infinity;
    this.lastPartialAt = -Infinity;
    this.segmentId++;
  }

  private requestTranscribe(isFinal: boolean): void {
    const audio = flatten(this.window, this.windowSamples);
    if (audio.length === 0) return;
    this.worker.postMessage({ type: 'transcribe', id: this.segmentId, audio, isFinal });
  }

  private onWorkerMessage(msg: WorkerOutbound): void {
    switch (msg.type) {
      case 'ready':
        this.ready = true;
        this.onModelStatus?.({
          phase: 'ready',
          message: `speech model ready (${msg.device})`,
          fraction: null,
          device: msg.device,
        });
        return;
      case 'progress':
        this.onModelStatus?.({
          phase: 'downloading',
          message: 'downloading speech model…',
          fraction: msg.fraction,
        });
        return;
      case 'partial':
        // Ignore a partial whose segment has already closed (id moved on).
        if (msg.id !== this.segmentId) return;
        if (msg.text.trim().length > 0) this.handlers?.partial(msg.text);
        return;
      case 'final': {
        const result: Finalized = { text: msg.text, noSpeechProb: null };
        // The pass-2 (endpoint) result → finalize. A non-final `final` is the
        // completion of a live partial pass; surface it as a partial so the edge
        // still reflects the best decode of the current window.
        if (msg.isFinal) {
          this.handlers?.finalize(result);
        } else if (msg.id === this.segmentId && msg.text.trim().length > 0) {
          this.handlers?.partial(msg.text);
        }
        return;
      }
      case 'error':
        this.onModelStatus?.({
          phase: 'error',
          message: msg.fatal ? 'speech model failed to load' : 'transcription error',
          fraction: null,
        });
        console.error('transformers worker:', msg.message);
        return;
    }
  }
}

/** Convert an Int16 PCM frame to Float32 in [-1,1] (i16 / 32768). */
export function int16ToFloat32(frame: Int16Frame): Float32Array {
  const out = new Float32Array(frame.length);
  for (let i = 0; i < frame.length; i++) out[i] = frame[i] / 0x8000;
  return out;
}

/** Mean absolute amplitude of an Int16 frame, normalized to 0..1. */
function frameMeanAbs(frame: Int16Frame): number {
  if (frame.length === 0) return 0;
  let sum = 0;
  for (let i = 0; i < frame.length; i++) sum += Math.abs(frame[i]);
  return sum / frame.length / 0x8000;
}

/** Concatenate the window chunks into one contiguous Float32Array. */
function flatten(chunks: readonly Float32Array[], total: number): Float32Array {
  const out = new Float32Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}
