// The message protocol between the recognizer (main thread) and the
// transformers.js inference worker. Split into its own module so the recognizer
// and its test can import the message TYPES without importing the worker source
// (which touches `self` / `navigator` at module scope and only loads in a
// Worker context).

/** Where the model actually runs once the worker probes the environment. */
export type EnginePlacement = 'webgpu' | 'wasm';

// ── Main thread → worker ────────────────────────────────────────────────────

/** Ask the worker to lazy-load the tokenizer / processor / model. */
export interface LoadMessage {
  readonly type: 'load';
  readonly modelId: string;
}

/** Ask the worker to transcribe a Float32 16 kHz mono window. `isFinal` marks
 *  the segment's pass-2 (endpoint) pass vs a live partial re-run. `id` pairs the
 *  response back to the segment the recognizer is tracking. */
export interface TranscribeMessage {
  readonly type: 'transcribe';
  readonly id: number;
  readonly audio: Float32Array;
  readonly isFinal: boolean;
}

export type WorkerInbound = LoadMessage | TranscribeMessage;

// ── Worker → main thread ────────────────────────────────────────────────────

/** The model is loaded and ready to transcribe. */
export interface ReadyMessage {
  readonly type: 'ready';
  readonly modelId: string;
  readonly device: EnginePlacement;
}

/** A per-file download progress tick (forwarded from HF's progress_callback). */
export interface ProgressMessage {
  readonly type: 'progress';
  readonly file: string;
  readonly loaded: number;
  readonly total: number;
  /** loaded/total in [0,1], or null when total is unknown. */
  readonly fraction: number | null;
}

/** A streamed chunk of decoded text for an in-flight generation. */
export interface PartialMessage {
  readonly type: 'partial';
  readonly id: number;
  readonly text: string;
}

/** The completed decode for a generation. `isFinal` echoes the request so the
 *  recognizer routes it to `finalize` (endpoint pass) vs a live `partial`. */
export interface FinalMessage {
  readonly type: 'final';
  readonly id: number;
  readonly text: string;
  readonly isFinal: boolean;
}

/** A worker-side failure. `fatal` distinguishes a load failure (the engine is
 *  unusable) from a transient per-generation error (the next pass may succeed). */
export interface ErrorMessage {
  readonly type: 'error';
  readonly message: string;
  readonly fatal: boolean;
}

export type WorkerOutbound =
  | ReadyMessage
  | ProgressMessage
  | PartialMessage
  | FinalMessage
  | ErrorMessage;

/** The minimal worker surface the recognizer uses — `postMessage`, the two event
 *  handlers, and `terminate`. A real `Worker` satisfies it; the test injects a
 *  fake that posts scripted `WorkerOutbound` messages. */
export interface WorkerLike {
  postMessage(message: WorkerInbound): void;
  onmessage: ((event: MessageEvent<WorkerOutbound>) => void) | null;
  onerror: ((event: unknown) => void) | null;
  terminate(): void;
}
