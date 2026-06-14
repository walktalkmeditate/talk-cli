// The transformers.js inference worker — a real browser-native Whisper engine,
// run off the main thread so generation never blocks the terminal render loop.
//
// This is the SPIKE engine behind the `Recognizer` seam (recognizer.ts). It is
// "Pattern A" from the realtime-whisper-webgpu example: it bypasses the
// `pipeline()` wrapper and drives the tokenizer / processor / model directly so
// it can stream partial text via a `TextStreamer` while a generation is still in
// flight. The recognizer (transformers-recognizer.ts) owns the audio buffer +
// VAD/endpoint heuristic and tells this worker WHEN to transcribe; the worker
// owns only "load the model" + "transcribe this Float32 window".
//
// Privacy posture (SPIKE): the model files fetch from the Hugging Face CDN by
// default. That dents the zero-egress story for the spike — production must
// self-host the model + the ort-wasm runtime on cdn.pilgrimapp.org (see the CSP
// note in index.html) and point `env.remoteHost` / `env.localModelPath` at it,
// so the net-silence canary's allowlist still holds.

import {
  AutoTokenizer,
  AutoProcessor,
  WhisperForConditionalGeneration,
  TextStreamer,
  env,
  type PreTrainedTokenizer,
  type Processor,
  type PreTrainedModel,
  type ProgressInfo,
  type Tensor,
} from '@huggingface/transformers';

import {
  type WorkerInbound,
  type WorkerOutbound,
  type EnginePlacement,
} from './transformers-protocol';

// Single-threaded ORT runtime (no SharedArrayBuffer ⇒ no COOP/COEP headers
// needed — the privacy posture stays on a header-less static host). Set
// explicitly so a future ORT default flip can't silently pull in threads. The
// `wasm` backend object is present + mutable at runtime; the typing marks it
// read-only, so assign the field through the existing object.
if (env.backends.onnx.wasm) {
  env.backends.onnx.wasm.numThreads = 1;
}

/** The dtype split transformers.js wants per device (encoder vs decoder). */
const DTYPE_BY_DEVICE = {
  webgpu: { encoder_model: 'fp32', decoder_model_merged: 'q4' },
  wasm: { encoder_model: 'fp32', decoder_model_merged: 'q8' },
} as const;

/** Hard cap on tokens per generate so a long window can't run away. base.en's
 *  natural ceiling is 448; we stay under it and rely on the rolling window. */
const MAX_NEW_TOKENS = 128;

/** The minimal shape of the WebGPU adapter probe we need — the TS DOM lib
 *  (5.6) has no `navigator.gpu`, and pulling in `@webgpu/types` for one feature
 *  check is not worth it. This narrows ONLY what we touch. */
interface GpuLike {
  requestAdapter(): Promise<unknown | null>;
}
interface NavigatorWithGpu {
  gpu?: GpuLike;
}

/** Probe WebGPU: present AND able to hand back an adapter ⇒ use it, else WASM. */
async function selectDevice(): Promise<EnginePlacement> {
  const nav = navigator as unknown as NavigatorWithGpu;
  if (!nav.gpu) return 'wasm';
  try {
    const adapter = await nav.gpu.requestAdapter();
    return adapter ? 'webgpu' : 'wasm';
  } catch {
    return 'wasm';
  }
}

/** The loaded engine triple, instantiated once and reused across generations. */
interface LoadedEngine {
  readonly tokenizer: PreTrainedTokenizer;
  readonly processor: Processor;
  readonly model: PreTrainedModel;
  readonly device: EnginePlacement;
}

let engine: LoadedEngine | null = null;
/** Guards against overlapping `transcribe` requests: the recognizer debounces,
 *  but a final pass can still arrive while a partial pass is mid-flight. We run
 *  generations strictly one-at-a-time and drop a partial that lands while busy. */
let busy = false;

function post(message: WorkerOutbound): void {
  // The Float32 window is owned by the recognizer; nothing we post back is
  // transferable (text + small numbers), so a plain postMessage is correct.
  self.postMessage(message);
}

/** Load the tokenizer / processor / model for `modelId` on the best device,
 *  forwarding HF's per-file download progress to the main thread for the UI. */
async function load(modelId: string): Promise<void> {
  if (engine) {
    post({ type: 'ready', modelId, device: engine.device });
    return;
  }

  const device = await selectDevice();
  const dtype = DTYPE_BY_DEVICE[device];

  const progress_callback = (info: ProgressInfo): void => {
    if (info.status === 'progress') {
      post({
        type: 'progress',
        file: info.file,
        loaded: info.loaded,
        total: info.total,
        fraction: info.total > 0 ? info.loaded / info.total : null,
      });
    }
  };

  const [tokenizer, processor, model] = await Promise.all([
    AutoTokenizer.from_pretrained(modelId, { progress_callback }),
    AutoProcessor.from_pretrained(modelId, { progress_callback }),
    WhisperForConditionalGeneration.from_pretrained(modelId, {
      dtype,
      device,
      progress_callback,
    }),
  ]);

  engine = { tokenizer, processor, model, device };
  post({ type: 'ready', modelId, device });
}

/**
 * Run one generation over a Float32 16 kHz window. While it streams, each
 * decoded chunk is posted as a `partial`; on completion the full decode is
 * posted as the `final` for this `id`. `isFinal` only tags which message the
 * recognizer should treat as the segment's pass-2 result — the inference path is
 * identical either way (Whisper has no separate "streaming" mode here; the
 * recognizer re-runs on the growing window for live partials).
 */
async function transcribe(id: number, audio: Float32Array, isFinal: boolean): Promise<void> {
  if (!engine) {
    post({ type: 'error', message: 'transcribe before ready', fatal: false });
    return;
  }
  if (busy && !isFinal) {
    // A partial pass while another generation runs: drop it (the next debounce
    // tick will re-run on a fresher window). A final pass waits its turn below.
    return;
  }
  while (busy) await new Promise((r) => setTimeout(r, 5));
  busy = true;

  const { tokenizer, processor, model } = engine;
  try {
    const streamer = new TextStreamer(tokenizer, {
      skip_prompt: true,
      skip_special_tokens: true,
      callback_function: (text: string) => {
        post({ type: 'partial', id, text });
      },
    });

    const inputs = await processor(audio);
    const outputs = await model.generate({
      ...inputs,
      max_new_tokens: MAX_NEW_TOKENS,
      language: 'en',
      streamer,
    });

    const decoded = tokenizer.batch_decode(outputs as Tensor, { skip_special_tokens: true });
    const text = (decoded[0] ?? '').trim();
    post({ type: 'final', id, text, isFinal });
  } catch (err) {
    post({
      type: 'error',
      message: err instanceof Error ? err.message : String(err),
      fatal: false,
    });
  } finally {
    busy = false;
  }
}

self.onmessage = (event: MessageEvent<WorkerInbound>): void => {
  const msg = event.data;
  switch (msg.type) {
    case 'load':
      void load(msg.modelId);
      return;
    case 'transcribe':
      void transcribe(msg.id, msg.audio, msg.isFinal);
      return;
  }
};
