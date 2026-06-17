// Mic capture at 16 kHz mono + the full mic-permission state machine (R8).
//
// Capture: an AudioContext forced to 16 kHz (the rate both sherpa-onnx models
// want — the browser does the device→16k conversion) feeds an AudioWorklet that
// converts Float32→Int16 on the audio thread (web/public/mic-worklet.js); each
// Int16 frame is handed to a callback. A ScriptProcessorNode fallback covers old
// Safari (< 14.5, no AudioWorklet).
//
// Permission (R8): FIVE distinct states, each surfaced on its own — never a hang.
//   pending            — the browser dialog is open, waiting on the user
//   granted            — capture is live
//   denied             — the user (or a remembered choice) refused; show the
//                         browser-specific re-enable guidance + a retry
//   denied-suppressed  — NO dialog appeared (the browser blocked it after prior
//                         refusals); detected via an immediate reject WITHOUT a
//                         visible prompt — same NotAllowedError, but it resolves
//                         far faster than a human could click "Block"
//   device-unavailable — NotReadableError: the mic exists but is in use / a
//                         hardware fault; distinct "mic in use" message
//
// The engine + ASR run elsewhere; this module only emits frames + the state.

/** The five mic-permission states (R8), each surfaced distinctly in the UI. */
export type MicState =
  | 'pending'
  | 'granted'
  | 'denied'
  | 'denied-suppressed'
  | 'device-unavailable';

/** A 16 kHz mono Int16 PCM frame from the AudioWorklet / ScriptProcessor. */
export type Int16Frame = Int16Array;

/** Re-enable guidance + the distinguishing detail for a non-granted state. */
export interface MicStateDetail {
  readonly state: MicState;
  /** Human-readable guidance for the UI (re-enable steps, retry hint). */
  readonly message: string;
  /** True when a retry (re-calling `start()`) is worth offering. */
  readonly retryable: boolean;
}

/** The target capture rate — both recognizers consume 16 kHz mono. */
export const TARGET_SAMPLE_RATE = 16000;

/** Thrown when the AudioContext stays `suspended` after `resume()` — proceeding
 *  would wire a graph that never receives frames (a silent, hung session). The
 *  caller maps it to `device-unavailable` so the UI surfaces a real state. */
export class AudioContextSuspendedError extends Error {
  constructor() {
    super('audio context stayed suspended after resume — no frames would arrive');
    this.name = 'AudioContextSuspendedError';
  }
}

/** Thrown internally to unwind `wireCaptureGraph` when `stop()` raced it. */
class CaptureStopped extends Error {
  constructor() {
    super('capture stopped mid-wire');
    this.name = 'CaptureStopped';
  }
}

/**
 * Below this, a getUserMedia rejection arrived faster than a human could dismiss
 * a dialog — so the browser auto-rejected WITHOUT prompting (denied-suppressed),
 * rather than the user clicking Block. Heuristic, deliberately generous: a
 * genuine user "Block" click is on the order of a second or more.
 */
const SUPPRESSED_REJECT_MS = 250;

export interface AudioCaptureOptions {
  /** Called with each Int16 16 kHz frame while capture is live. */
  readonly onFrame: (frame: Int16Frame) => void;
  /** Called on every mic-permission state transition (R8). */
  readonly onState: (detail: MicStateDetail) => void;
  /**
   * Injected getUserMedia (defaults to navigator.mediaDevices.getUserMedia) —
   * the test seam for the permission flow without a real mic.
   */
  readonly getUserMedia?: (constraints: MediaStreamConstraints) => Promise<MediaStream>;
  /**
   * Injected AudioContext factory (defaults to the platform AudioContext) — the
   * test seam for capture wiring without a real audio graph.
   */
  readonly audioContextFactory?: (sampleRate: number) => AudioContext;
  /** Injected clock for the suppressed-reject heuristic (defaults to performance.now). */
  readonly now?: () => number;
}

/** Per-browser re-enable steps shown on `denied`. */
function deniedGuidance(): string {
  const ua = typeof navigator !== 'undefined' ? navigator.userAgent : '';
  if (/firefox/i.test(ua)) {
    return 'Mic access was blocked. Click the mic/permissions icon in the address bar, ' +
      'clear the block, then retry.';
  }
  if (/safari/i.test(ua) && !/chrome|chromium|crios/i.test(ua)) {
    return 'Mic access was blocked. Open Safari ▸ Settings for This Website ▸ Microphone ▸ Allow, ' +
      'then retry.';
  }
  // Chromium-family default.
  return 'Mic access was blocked. Click the camera/mic icon in the address bar ▸ ' +
    'Site settings ▸ allow Microphone, then retry.';
}

/** Re-enable steps shown on `denied-suppressed` (no dialog appeared). */
function suppressedGuidance(): string {
  return 'Your browser blocked the mic prompt before it could appear (usually after an ' +
    'earlier block). Open this site’s settings ▸ Microphone ▸ Allow (or reset ' +
    'permissions), then retry.';
}

function detailFor(state: MicState): MicStateDetail {
  switch (state) {
    case 'pending':
      return { state, message: 'Waiting for microphone permission…', retryable: false };
    case 'granted':
      return { state, message: 'Listening.', retryable: false };
    case 'denied':
      return { state, message: deniedGuidance(), retryable: true };
    case 'denied-suppressed':
      return { state, message: suppressedGuidance(), retryable: true };
    case 'device-unavailable':
      return {
        state,
        message: 'Your microphone is unavailable — it may be in use by another app, ' +
          'or there’s a hardware issue. Close other apps using the mic, then retry.',
        retryable: true,
      };
  }
}

/**
 * Map a getUserMedia rejection to a mic state. A `NotAllowedError`/
 * `SecurityError` that arrived faster than `SUPPRESSED_REJECT_MS` is
 * `denied-suppressed` (no dialog shown); a slower one is a real `denied`.
 * `NotReadableError`/`AbortError`/`NotFoundError` are `device-unavailable`.
 */
export function classifyMicError(err: unknown, elapsedMs: number): MicState {
  const name = err instanceof DOMException || err instanceof Error ? err.name : '';
  switch (name) {
    case 'NotAllowedError':
    case 'SecurityError':
      return elapsedMs < SUPPRESSED_REJECT_MS ? 'denied-suppressed' : 'denied';
    case 'NotReadableError':
    case 'AbortError':
    case 'NotFoundError':
    case 'OverconstrainedError':
      return 'device-unavailable';
    default:
      // An unknown rejection is treated as a plain denial (retryable) rather
      // than a hang — the user can always retry.
      return 'denied';
  }
}

/** Build the public detail for a mic state (exported for the UI + tests). */
export function micStateDetail(state: MicState): MicStateDetail {
  return detailFor(state);
}

/**
 * A live mic capture session. `start()` runs the permission flow and, on grant,
 * wires the 16 kHz capture graph; `stop()` tears it down. The five permission
 * states are surfaced through `onState` so the UI can show each distinctly.
 */
export class AudioCapture {
  private ctx: AudioContext | null = null;
  private stream: MediaStream | null = null;
  private worklet: AudioWorkletNode | null = null;
  private scriptNode: ScriptProcessorNode | null = null;
  private source: MediaStreamAudioSourceNode | null = null;
  private state: MicState = 'pending';
  /** Set by `stop()` so an in-flight `wireCaptureGraph` (which awaits across the
   *  permission + module-load boundary) bails out instead of leaving orphaned
   *  nodes wired to a context that is about to be torn down. */
  private stopping = false;

  constructor(private readonly opts: AudioCaptureOptions) {}

  /** The last-known mic state. */
  micState(): MicState {
    return this.state;
  }

  private setState(state: MicState): void {
    this.state = state;
    this.opts.onState(detailFor(state));
  }

  /**
   * Request the mic and, on grant, start 16 kHz capture. Idempotent guidance:
   * calling `start()` again after a denial is the retry path. Resolves to the
   * resulting state; never throws for a permission outcome (the state carries it).
   */
  async start(): Promise<MicState> {
    const getMedia =
      this.opts.getUserMedia ??
      ((c: MediaStreamConstraints) => navigator.mediaDevices.getUserMedia(c));
    const now = this.opts.now ?? (() => performance.now());

    this.stopping = false; // a fresh start clears any prior stop flag
    this.setState('pending');
    const t0 = now();
    let stream: MediaStream;
    try {
      stream = await getMedia({
        audio: {
          sampleRate: TARGET_SAMPLE_RATE,
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
        },
        video: false,
      });
    } catch (err) {
      const elapsed = now() - t0;
      const state = classifyMicError(err, elapsed);
      this.setState(state);
      return state;
    }

    this.stream = stream;
    if (this.stopping) {
      // stop() raced the permission grant — release the just-granted mic, no wire.
      this.teardownGraph();
      this.stream?.getTracks().forEach((t) => t.stop());
      this.stream = null;
      return this.state;
    }
    try {
      await this.wireCaptureGraph(stream);
    } catch (err) {
      if (err instanceof CaptureStopped) {
        // A stop() landed mid-wire; teardownGraph already ran in stop(). Stay calm.
        return this.state;
      }
      // The mic was granted but the audio graph failed to wire (incl. a context
      // that never left `suspended`) — treat the device as unavailable rather than
      // leaving the session in a silent, frameless limbo.
      this.teardownGraph();
      this.setState('device-unavailable');
      void err;
      return 'device-unavailable';
    }
    this.setState('granted');
    return 'granted';
  }

  private async wireCaptureGraph(stream: MediaStream): Promise<void> {
    const factory =
      this.opts.audioContextFactory ??
      ((sr: number) => new AudioContext({ sampleRate: sr }));
    const ctx = factory(TARGET_SAMPLE_RATE);
    this.ctx = ctx;

    // iOS Safari quirk: an AudioContext created before/around a user gesture can
    // start `suspended`, and routes capture to the receiver (earpiece) until the
    // graph is running. Resume explicitly so the worklet actually receives
    // frames; the gesture that triggered start() satisfies the autoplay policy.
    if (ctx.state === 'suspended') {
      await ctx.resume().catch(() => undefined);
      if (this.stopping) throw new CaptureStopped();
      // If it STILL won't resume, no frames will ever arrive — fail loudly rather
      // than wiring a graph into a silent context (a hung, frameless session).
      if (ctx.state === 'suspended') throw new AudioContextSuspendedError();
    }

    const source = ctx.createMediaStreamSource(stream);
    this.source = source;

    if (ctx.audioWorklet) {
      await ctx.audioWorklet.addModule('/mic-worklet.js');
      if (this.stopping) throw new CaptureStopped();
      const node = new AudioWorkletNode(ctx, 'mic-capture');
      node.port.onmessage = (e: MessageEvent<Int16Array>) => this.opts.onFrame(e.data);
      source.connect(node);
      this.worklet = node;
      return;
    }

    // ScriptProcessorNode fallback (deprecated; old Safari has no AudioWorklet).
    // It must connect to the destination to be pulled, so route it through a
    // zero-gain node to stay silent.
    const node = ctx.createScriptProcessor(4096, 1, 1);
    node.onaudioprocess = (e: AudioProcessingEvent) => {
      const channel = e.inputBuffer.getChannelData(0);
      this.opts.onFrame(floatChannelToInt16(channel));
    };
    const mute = ctx.createGain();
    mute.gain.value = 0;
    source.connect(node);
    node.connect(mute);
    mute.connect(ctx.destination);
    this.scriptNode = node;
  }

  private teardownGraph(): void {
    if (this.scriptNode) this.scriptNode.onaudioprocess = null;
    this.worklet?.disconnect();
    this.scriptNode?.disconnect();
    this.source?.disconnect();
    this.worklet = null;
    this.scriptNode = null;
    this.source = null;
    void this.ctx?.close().catch(() => undefined);
    this.ctx = null;
  }

  /** Resume capture after the page was backgrounded / the screen locked. iOS
   *  Safari suspends (or non-standardly "interrupts") the AudioContext on lock;
   *  without a resume, no frames arrive once the user returns and recording
   *  silently stops. Resume the context if it isn't running. Best-effort +
   *  idempotent — safe to call on every visibility regain and tap. iOS may require
   *  this to run inside a user gesture for the resume to take, so the host calls it
   *  from BOTH `visibilitychange` and a pointer tap. (A track that iOS *ended*
   *  while backgrounded can't be revived by resume — that needs a fresh session.) */
  async resume(): Promise<void> {
    if (this.stopping || !this.ctx) return;
    const state = this.ctx.state as string; // iOS adds a non-standard 'interrupted'
    if (state === 'suspended' || state === 'interrupted') {
      await this.ctx.resume().catch(() => undefined);
    }
  }

  /** Stop capture and release the mic. Safe to call when not started, and safe to
   *  call WHILE `start()` is still awaiting — the `stopping` flag makes an in-flight
   *  `wireCaptureGraph` bail before it leaves orphaned nodes emitting frames. */
  stop(): void {
    this.stopping = true;
    this.teardownGraph();
    this.stream?.getTracks().forEach((t) => t.stop());
    this.stream = null;
  }
}

/** Convert a Float32 channel [-1,1] to Int16 PCM (ScriptProcessor fallback path). */
function floatChannelToInt16(channel: Float32Array): Int16Array {
  const out = new Int16Array(channel.length);
  for (let i = 0; i < channel.length; i++) {
    const s = channel[i] < -1 ? -1 : channel[i] > 1 ? 1 : channel[i];
    out[i] = s < 0 ? s * 0x8000 : s * 0x7fff;
  }
  return out;
}
