// The REAL live session (the U6 WIRE seam, completed for the transformers.js
// spike) — the engine-backed twin of `DemoModeSession`. It drives the same
// `Pipeline` + `SessionControls` behind the identical `LiveSessionView` shape, so
// the host (main.ts) and the `ModeRouter` are unchanged: the engine swap is a
// one-line change in the session factory (Demo vs Live).
//
// What's real here vs the demo: instead of a `MockRecognizer` walked on a timer,
// this feeds the `Pipeline` with a `TransformersRecognizer` (browser-native
// Whisper in a worker) and real mic frames from `AudioCapture`. The mic
// permission states + the model-load status are surfaced through callbacks so the
// boot flow can show "asking for the mic…" / "loading model…".
//
// SPIKE caveat: real transcription quality + perf are unverified until run live
// (`npm run dev`); the model fetches from the HF CDN until self-hosted.

import { Pipeline } from '../asr/pipeline';
import { AudioCapture, type MicStateDetail } from '../asr/audio';
import {
  TransformersRecognizer,
  type ModelLoadStatus,
} from '../asr/transformers-recognizer';
import { SessionControls } from './controls';
import { isEphemeralMode, type SessionMode } from '../mobile';
import type { LiveSessionView } from './demo-session';

export interface LiveModeSessionOptions {
  /** HF model id override (e.g. `onnx-community/whisper-tiny.en`). */
  readonly modelId?: string;
  /** Model load + download status, for the boot/status surface. */
  readonly onModelStatus?: (status: ModelLoadStatus) => void;
  /** Mic permission state transitions (R8), for the status surface. */
  readonly onMicState?: (detail: MicStateDetail) => void;
}

/**
 * A mic-and-engine-backed session the `ModeRouter` drives. Implements the same
 * `LiveSessionView` as `DemoModeSession`, so nothing downstream changes.
 */
export class LiveModeSession implements LiveSessionView {
  private readonly pipeline: Pipeline;
  private readonly controls: SessionControls;
  private readonly recognizer: TransformersRecognizer;
  private readonly capture: AudioCapture;
  private started = false;
  private cancelled = false;
  private ended = false;

  constructor(
    mode: SessionMode,
    onControlsChange: () => void,
    opts: LiveModeSessionOptions = {},
  ) {
    // The model can begin loading at construction (no gesture needed for a fetch),
    // but the MIC must wait for begin() — getUserMedia + AudioContext are
    // gesture-gated; starting them here (the router constructs sessions at page
    // load) leaves the AudioContext suspended and captures no audio.
    this.recognizer = new TransformersRecognizer({
      modelId: opts.modelId,
      onModelStatus: opts.onModelStatus,
    });
    this.pipeline = new Pipeline({ recognizer: this.recognizer });
    this.controls = new SessionControls({
      pipeline: this.pipeline,
      ephemeral: isEphemeralMode(mode),
      onChange: onControlsChange,
    });
    this.capture = new AudioCapture({
      onFrame: (frame) => this.pipeline.pushAudio(frame),
      onState: (detail) => opts.onMicState?.(detail),
    });
  }

  begin(): void {
    if (this.started) return;
    this.started = true;
    // Fire-and-forget: the permission outcome is surfaced through onMicState; a
    // rejection never throws here (AudioCapture carries the state, not an error).
    void this.capture.start();
  }

  compose(args: {
    readonly mode: SessionMode;
    readonly question: string;
    readonly heldLabel: string;
    readonly elapsed: string;
    readonly cleanup: string;
  }): string {
    const idle = this.pipeline.idleStatus();
    const ctl = this.controls.state();
    return this.pipeline.settle.compose(
      args.mode,
      args.question,
      args.heldLabel,
      idle.listening,
      args.elapsed,
      args.cleanup,
      ctl.showRaw,
      ctl.paused,
      ctl.confirmCancel,
    );
  }

  controlsState() {
    return this.controls.state();
  }

  controlsCommand(cmd: 'done' | 'pause' | 'toggle-raw' | 'cancel'): void {
    this.controls.command(cmd);
  }

  controlsKey(data: string): boolean {
    return this.controls.onKey(data);
  }

  finalClean(): string {
    return this.pipeline.settle
      .settledText()
      .split('\n')
      .filter((s) => s.length > 0)
      .join(' ');
  }

  finalRaw(): string | null {
    const raw = this.pipeline.settle
      .settledRaw()
      .split('\n')
      .filter((s) => s.length > 0)
      .join(' ');
    return raw.length > 0 ? raw : null;
  }

  wasCancelled(): boolean {
    return this.cancelled;
  }

  markCancelled(): void {
    this.cancelled = true;
  }

  end(): void {
    if (this.ended) return;
    this.ended = true;
    this.capture.stop();
    this.pipeline.free(); // also frees the recognizer (terminates the worker)
  }
}
