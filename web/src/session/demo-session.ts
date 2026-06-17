// The demo-backed live session (U10) — extracted from main.ts so the engine swap
// (the U6 WIRE seam) is a contained change behind a narrow interface, not a tangle
// inside the boot wiring.
//
// `DemoModeSession` is a `ModeSession` the `ModeRouter` drives: a `Pipeline` over a
// `MockRecognizer` walked on a timer, wrapped with the U7 `SessionControls`. The
// host renders + drives it through the `LiveSessionView` interface below, so
// `composeSession` / the key+chip wiring depend on the SHAPE, not the concrete
// class. The real session (mic frames + `WireSherpaRecognizer`) implements the
// same `LiveSessionView` and slots in with no host change.

import { Pipeline } from '../asr/pipeline';
import { MockRecognizer, type ScriptStep } from '../asr/recognizer';
import { SessionControls, type ControlsState } from './controls';
import { isEphemeralMode, type SessionMode } from '../mobile';
import type { ModeSession } from './modes';

/**
 * The taste-it preview (R10) script: a canned partials→settle sequence the
 * `MockRecognizer` replays into the REAL settle → render path so the live edge
 * animates — dim partials jittering, an endpoint settling them, the pass-2
 * finalize upgrading to bright final text. Scripted playback, NOT real
 * transcription; the one-line swap to `WireSherpaRecognizer` (the WIRE seam) lights
 * up the on-device engine behind the same `Pipeline`.
 */
export const DEMO_SCRIPT: readonly ScriptStep[] = [
  { kind: 'partial', text: 'i think' },
  { kind: 'partial', text: 'i think i have been' },
  { kind: 'partial', text: 'um i think i have been holding my breath' },
  { kind: 'partial', text: 'um i think i have been holding my breath all week' },
  { kind: 'endpoint' },
  { kind: 'finalize', text: 'I think I have been holding my breath all week.', noSpeechProb: 0.02 },
  { kind: 'partial', text: 'and what i want' },
  { kind: 'partial', text: 'and what i want is to slow down' },
  { kind: 'partial', text: 'and what i want is to slow down and notice' },
];

export const DEMO_STEP_MS = 700;

/**
 * The narrow live-session surface the HOST consumes (compose + key/chip drive +
 * the once-guard's cancel mark). It is a `ModeSession` plus the rendering/control
 * accessors `main.ts` needs — deliberately small so the engine swap only has to
 * satisfy this, not the concrete `DemoModeSession`.
 */
export interface LiveSessionView extends ModeSession {
  /** Start capture/playback — MUST be called from a user gesture (keypress/click).
   *  getUserMedia + AudioContext are gesture-gated by browsers; starting in the
   *  constructor (which the router runs at page load) leaves the AudioContext
   *  suspended and no audio is ever captured. Idempotent. */
  begin(): void;
  /** Resume capture after the page was backgrounded / the screen locked (iOS
   *  suspends the audio context). Idempotent; a no-op before begin / after end. */
  resumeCapture(): void;
  /** Compose the session's screen as a wasm JSON string for the given view fields. */
  compose(args: {
    readonly mode: SessionMode;
    readonly question: string;
    readonly heldLabel: string;
    readonly elapsed: string;
    readonly cleanup: string;
  }): string;
  /** The U7 controls state (show-raw / paused / confirm-cancel / finished). */
  controlsState(): ControlsState;
  /** Run a normalized control command (chip taps + the host's command router). */
  controlsCommand(cmd: 'done' | 'pause' | 'toggle-raw' | 'cancel'): void;
  /** Feed a raw key chunk to the controls; true if it was a recognized control. */
  controlsKey(data: string): boolean;
  /** Mark the session cancelled (the host's once-guard, before routing cancel). */
  markCancelled(): void;
}

/**
 * A demo-backed session the `ModeRouter` drives. The real session (U6 WIRE seam)
 * replaces the timer with mic frames and the mock with `WireSherpaRecognizer`
 * behind this same `LiveSessionView` shape.
 */
export class DemoModeSession implements LiveSessionView {
  private readonly pipeline: Pipeline;
  private readonly controls: SessionControls;
  private readonly recognizer: MockRecognizer;
  private timer: ReturnType<typeof setInterval> | null = null;
  private started = false;
  private cancelled = false;

  constructor(mode: SessionMode, onControlsChange: () => void) {
    this.recognizer = new MockRecognizer(DEMO_SCRIPT, { advanceOnAudio: false });
    this.pipeline = new Pipeline({ recognizer: this.recognizer });
    this.controls = new SessionControls({
      pipeline: this.pipeline,
      ephemeral: isEphemeralMode(mode),
      onChange: onControlsChange,
    });
  }

  begin(): void {
    if (this.started) return;
    this.started = true;
    this.timer = setInterval(() => {
      if (!this.recognizer.step() && this.timer) clearInterval(this.timer);
    }, DEMO_STEP_MS);
  }

  /** No real mic to resume in the scripted demo. */
  resumeCapture(): void {}

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
      ctl.showRaw, // `u` flips raw verbatim ⇄ cleaned text
      ctl.paused,
      ctl.confirmCancel,
    );
  }

  controlsState(): ControlsState {
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
    if (this.timer) clearInterval(this.timer);
    this.pipeline.free();
  }
}
