// The session interaction layer (U7) — CLI-parity controls mapped onto the U6
// pipeline. Mirrors src/live.rs::run_loop's key handling exactly:
//
//   space → finish    (save & close; "release" in unburden)
//   p     → pause/off-record (drives pipeline.pause()/resume())
//   u     → toggle raw⇄clean (flows into Settle.compose's show_raw flag)
//   esc   → cancel    (ephemeral cancels immediately; otherwise a confirm-cancel
//                      gate: esc/y confirms the discard, any other key resumes)
//
// The controller is deliberately DOM-free and injectable: it takes the Pipeline
// (or any object matching SessionActions) plus an onChange callback, so the
// control LOGIC is unit-tested without a browser. main.ts wires the real key
// events (xterm) and the chip-bar taps into the same `command(...)` entry point.

/** The pipeline surface the controller drives (a structural subset of Pipeline,
 *  so a fake satisfies it in tests without the DOM or the real wasm engine). */
export interface SessionActions {
  pause(): void;
  resume(): void;
  finish(): void;
  isPaused(): boolean;
}

/** A normalized session command — the SAME verbs whether they arrive from a key
 *  (desktop) or a chip tap (mobile). Chip `command` strings map onto these. */
export type SessionCommand = 'done' | 'pause' | 'toggle-raw' | 'cancel';

/** The controller's externally-observable state (read by the renderer to drive
 *  Settle.compose's show_raw / paused / confirm_cancel flags). */
export interface ControlsState {
  /** True → render the verbatim raw transcript; false → the cleaned text. */
  readonly showRaw: boolean;
  /** True while off-record (mirrors the pipeline's pause). */
  readonly paused: boolean;
  /** True while the discard-confirm prompt is showing (non-ephemeral cancel). */
  readonly confirmCancel: boolean;
  /** True once the session has finished (space) or been cancelled+confirmed. */
  readonly finished: boolean;
  /** True when finished via cancel (the entry is discarded, not kept). */
  readonly cancelled: boolean;
}

export interface ControlsOptions {
  readonly pipeline: SessionActions;
  /**
   * Ephemeral (unburden) sessions cancel immediately — nothing is at risk, so
   * esc never shows the discard prompt. Mirrors `cfg.ephemeral` in live.rs.
   */
  readonly ephemeral?: boolean;
  /** Called after any state change so the host can repaint. */
  readonly onChange?: () => void;
}

/**
 * The session controller. Holds the show-raw / confirm-cancel UI state the
 * pipeline does not (pause lives in the pipeline; this mirrors it for the
 * renderer), and translates commands + raw key bytes into pipeline actions.
 */
export class SessionControls {
  private readonly pipeline: SessionActions;
  private readonly ephemeral: boolean;
  private readonly onChange: (() => void) | undefined;

  private showRaw = false;
  private confirmCancel = false;
  private finished = false;
  private cancelled = false;

  constructor(opts: ControlsOptions) {
    this.pipeline = opts.pipeline;
    this.ephemeral = opts.ephemeral ?? false;
    this.onChange = opts.onChange;
  }

  /** The current state, for the renderer (show_raw / paused / confirm_cancel). */
  state(): ControlsState {
    return {
      showRaw: this.showRaw,
      paused: this.pipeline.isPaused(),
      confirmCancel: this.confirmCancel,
      finished: this.finished,
      cancelled: this.cancelled,
    };
  }

  /**
   * Handle a raw key chunk from xterm (`onData`). Maps the CLI keys (space / u /
   * p / esc) onto commands, and — while a cancel is pending confirmation —
   * consumes the y/n decision exactly as live.rs does. Returns true if the chunk
   * was a recognized control (so the host can decide whether to swallow it).
   */
  onKey(data: string): boolean {
    if (this.finished) return false;

    // While confirming a cancel, the next key is the y/n decision (live.rs parity:
    // y or esc confirms the discard; any other key resumes the session).
    if (this.confirmCancel) {
      if (data === 'y' || data === ESC) {
        this.doCancel();
      } else {
        this.confirmCancel = false;
        this.emit();
      }
      return true;
    }

    switch (data) {
      case ' ':
        return this.command('done');
      case 'u':
        return this.command('toggle-raw');
      case 'p':
        return this.command('pause');
      case ESC:
      case CTRL_C:
        return this.command('cancel');
      default:
        return false;
    }
  }

  /**
   * Run a normalized command (the entry point chip taps use too). Returns true
   * if it was handled. While a cancel is pending confirmation, only `cancel`
   * (re-confirm) is meaningful; other commands clear the prompt first.
   */
  command(cmd: SessionCommand): boolean {
    if (this.finished) return false;

    switch (cmd) {
      case 'done':
        this.doFinish();
        return true;
      case 'toggle-raw':
        this.showRaw = !this.showRaw;
        this.emit();
        return true;
      case 'pause':
        this.togglePause();
        return true;
      case 'cancel':
        this.doCancel();
        return true;
    }
  }

  private togglePause(): void {
    if (this.pipeline.isPaused()) {
      this.pipeline.resume();
    } else {
      this.pipeline.pause();
    }
    this.emit();
  }

  private doFinish(): void {
    this.pipeline.finish();
    this.finished = true;
    this.confirmCancel = false;
    this.emit();
  }

  /**
   * Cancel = discard. Ephemeral (unburden) cancels immediately (nothing at
   * risk). Otherwise the FIRST cancel arms the confirm prompt; the SECOND
   * (esc/y, handled in onKey) actually discards.
   */
  private doCancel(): void {
    if (this.ephemeral) {
      this.cancelled = true;
      this.finished = true;
      this.confirmCancel = false;
      this.emit();
      return;
    }
    if (!this.confirmCancel) {
      this.confirmCancel = true;
      this.emit();
      return;
    }
    // confirm-cancel was already armed (chip tapped twice / command re-issued):
    // discard now.
    this.cancelled = true;
    this.finished = true;
    this.confirmCancel = false;
    this.emit();
  }

  private emit(): void {
    this.onChange?.();
  }
}

/** ESC byte as xterm delivers it on `onData`. */
const ESC = '\x1b';
/** Ctrl-C byte (parity with live.rs's ctrl-c → cancel). */
const CTRL_C = '\x03';
