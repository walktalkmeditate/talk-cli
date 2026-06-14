import { describe, it, expect } from 'vitest';
import {
  TransformersRecognizer,
  int16ToFloat32,
  type ModelLoadStatus,
} from './transformers-recognizer';
import type { RecognizerEvents } from './recognizer';
import type { WorkerInbound, WorkerLike, WorkerOutbound } from './transformers-protocol';

// This test injects a FAKE worker — NO real model, no transformers.js inference.
// It exercises the recognizer's own logic: Int16→Float32 conversion, the
// load-on-construct + progress/ready status mapping, the energy VAD/endpoint, and
// the worker-message → seam-event mapping (partial / endpoint / finalize). The
// real transcription is validated live by the user with `npm run dev`.

/** A scriptable fake worker: records what the recognizer posts to it, and lets
 *  the test push `WorkerOutbound` messages back as if from the inference worker. */
class FakeWorker implements WorkerLike {
  onmessage: ((event: MessageEvent<WorkerOutbound>) => void) | null = null;
  onerror: ((event: unknown) => void) | null = null;
  readonly sent: WorkerInbound[] = [];
  terminated = false;

  postMessage(message: WorkerInbound): void {
    this.sent.push(message);
  }

  terminate(): void {
    this.terminated = true;
  }

  /** Deliver a worker→main message to the recognizer's handler. */
  emit(message: WorkerOutbound): void {
    this.onmessage?.({ data: message } as MessageEvent<WorkerOutbound>);
  }
}

/** Collect the seam events the recognizer fires, for assertions. */
function collector(): { events: RecognizerEvents; log: string[] } {
  const log: string[] = [];
  const events: RecognizerEvents = {
    partial: (text) => log.push(`partial:${text}`),
    endpoint: () => log.push('endpoint'),
    finalize: (r) => log.push(`finalize:${r.text}:${r.noSpeechProb}`),
  };
  return { events, log };
}

/** A frame of `samples` Int16s at amplitude `amp` (0 = silence). */
function frame(samples: number, amp: number): Int16Array {
  return new Int16Array(samples).fill(amp);
}

function build(extra?: { now?: () => number; onModelStatus?: (s: ModelLoadStatus) => void }) {
  const worker = new FakeWorker();
  const { events, log } = collector();
  const recognizer = new TransformersRecognizer({
    workerFactory: () => worker,
    now: extra?.now,
    onModelStatus: extra?.onModelStatus,
  }).on(events);
  return { worker, recognizer, log };
}

describe('int16ToFloat32 — PCM conversion', () => {
  it('maps Int16 samples to [-1,1) via i16/32768', () => {
    // #given representative Int16 PCM samples
    const out = int16ToFloat32(new Int16Array([0, 32767, -32768, 16384]));
    // #then each is divided by 32768
    expect(out[0]).toBeCloseTo(0, 6);
    expect(out[1]).toBeCloseTo(32767 / 32768, 6);
    expect(out[2]).toBeCloseTo(-1, 6);
    expect(out[3]).toBeCloseTo(0.5, 6);
  });

  it('returns a Float32Array of the same length', () => {
    const out = int16ToFloat32(new Int16Array(160));
    expect(out).toBeInstanceOf(Float32Array);
    expect(out.length).toBe(160);
  });
});

describe('TransformersRecognizer — worker lifecycle', () => {
  it('posts a load message for the default model on construction', () => {
    // #given a freshly constructed recognizer
    const { worker } = build();
    // #then it asked the worker to load the default model
    const load = worker.sent.find((m) => m.type === 'load');
    expect(load).toEqual({ type: 'load', modelId: 'onnx-community/whisper-base.en' });
  });

  it('honors a custom model id', () => {
    const worker = new FakeWorker();
    new TransformersRecognizer({ workerFactory: () => worker, modelId: 'onnx-community/whisper-tiny.en' });
    expect(worker.sent[0]).toEqual({ type: 'load', modelId: 'onnx-community/whisper-tiny.en' });
  });

  it('terminates the worker on free()', () => {
    const { worker, recognizer } = build();
    recognizer.free();
    expect(worker.terminated).toBe(true);
  });
});

describe('TransformersRecognizer — model status mapping', () => {
  it('reports loading, then downloading with a fraction, then ready with device', () => {
    // #given a status collector
    const statuses: ModelLoadStatus[] = [];
    const { worker } = build({ onModelStatus: (s) => statuses.push(s) });
    // #when the worker reports progress then ready
    worker.emit({ type: 'progress', file: 'decoder.onnx', loaded: 50, total: 100, fraction: 0.5 });
    worker.emit({ type: 'ready', modelId: 'onnx-community/whisper-base.en', device: 'webgpu' });
    // #then the phases walk loading → downloading(0.5) → ready(webgpu)
    expect(statuses.map((s) => s.phase)).toEqual(['loading', 'downloading', 'ready']);
    expect(statuses[1].fraction).toBe(0.5);
    expect(statuses[2].device).toBe('webgpu');
  });
});

describe('TransformersRecognizer — worker message → seam event mapping', () => {
  it('a partial for the current segment fires partial(text)', () => {
    const { worker, recognizer, log } = build();
    worker.emit({ type: 'ready', modelId: 'x', device: 'wasm' });
    // #when a partial arrives for the live segment (id 0)
    worker.emit({ type: 'partial', id: 0, text: 'i think i have been' });
    // #then the edge updates
    expect(log).toContain('partial:i think i have been');
    recognizer.free();
  });

  it('drops an empty partial (nothing to settle)', () => {
    const { worker, recognizer, log } = build();
    worker.emit({ type: 'partial', id: 0, text: '   ' });
    expect(log).toHaveLength(0);
    recognizer.free();
  });

  it('ignores a stale partial whose segment has already closed', () => {
    const { worker, recognizer, log } = build();
    // #given a reset bumped the segment id off 0
    recognizer.reset();
    // #when a partial for the OLD segment 0 arrives late
    worker.emit({ type: 'partial', id: 0, text: 'stale edge' });
    // #then it is ignored (no partial fired)
    expect(log).toEqual([]);
    recognizer.free();
  });

  it('a final-pass result fires finalize with noSpeechProb null', () => {
    const { worker, recognizer, log } = build();
    // #when an endpoint pass-2 result arrives
    worker.emit({ type: 'final', id: 0, text: 'I think I have been holding my breath.', isFinal: true });
    // #then it maps to finalize (transformers exposes no no_speech_prob)
    expect(log).toContain('finalize:I think I have been holding my breath.:null');
    recognizer.free();
  });

  it('a non-final completed pass surfaces as a partial for the live segment', () => {
    const { worker, recognizer, log } = build();
    worker.emit({ type: 'final', id: 0, text: 'live window decode', isFinal: false });
    expect(log).toContain('partial:live window decode');
    expect(log.some((l) => l.startsWith('finalize'))).toBe(false);
    recognizer.free();
  });
});

describe('TransformersRecognizer — energy VAD endpoint', () => {
  it('fires endpoint after ~700ms of silence following real speech', () => {
    // #given a controllable clock
    let t = 0;
    const { worker, recognizer, log } = build({ now: () => t });
    worker.emit({ type: 'ready', modelId: 'x', device: 'wasm' });

    // #when ~300ms of loud speech is fed (frames of 1600 samples = 100ms each)
    for (let i = 0; i < 3; i++) {
      recognizer.pushAudio(frame(1600, 8000));
      t += 100;
    }
    // #and then silence past the endpoint window
    recognizer.pushAudio(frame(1600, 0));
    t += ENDPOINT_SILENCE_PLUS;
    recognizer.pushAudio(frame(1600, 0));

    // #then an endpoint fired and a final transcription was requested
    expect(log).toContain('endpoint');
    const finalReq = worker.sent.find((m) => m.type === 'transcribe' && m.isFinal);
    expect(finalReq).toBeDefined();
    recognizer.free();
  });

  it('does NOT fire an endpoint for a too-short blip of speech', () => {
    let t = 0;
    const { worker, recognizer, log } = build({ now: () => t });
    worker.emit({ type: 'ready', modelId: 'x', device: 'wasm' });
    // #given only ~50ms of speech (under MIN_SPEECH_MS), then silence
    recognizer.pushAudio(frame(800, 8000)); // 50ms
    t += 50;
    recognizer.pushAudio(frame(1600, 0));
    t += ENDPOINT_SILENCE_PLUS;
    recognizer.pushAudio(frame(1600, 0));
    // #then no endpoint (the blip is dropped, not committed)
    expect(log).not.toContain('endpoint');
    recognizer.free();
  });

  it('reset() is silent — it emits no events', () => {
    let t = 0;
    const { worker, recognizer, log } = build({ now: () => t });
    worker.emit({ type: 'ready', modelId: 'x', device: 'wasm' });
    recognizer.pushAudio(frame(1600, 8000));
    t += 100;
    // #when reset is called mid-speech (off-record pause)
    recognizer.reset();
    // #then it produced no seam events (the contract: reset is a silent clear)
    expect(log).toEqual([]);
    recognizer.free();
  });
});

/** A clock advance comfortably past the endpoint-silence window. */
const ENDPOINT_SILENCE_PLUS = 800;
