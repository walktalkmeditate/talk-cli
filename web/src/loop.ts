import type { Terminal } from '@xterm/xterm';

// ── Pure helpers (unit-tested; no DOM, no wasm) ──────────────────────────────

/** Begin/end Synchronized Output: the terminal buffers the whole frame, then
 *  swaps it in one paint, so the live edge never tears mid-write. */
const BSU = '\x1b[?2026h';
const ESU = '\x1b[?2026l';
/** Home the cursor and overwrite in place — never clear-screen (`\x1b[2J`),
 *  which blanks the grid for a frame and reads as a flash. */
const HOME = '\x1b[H';
/** Erase from the cursor to the end of the current line. */
const EL = '\x1b[K';
/** Erase from the cursor to the end of the screen. */
const ED = '\x1b[J';

/**
 * Wrap one frame's content for a tear-free, flash-free write: synchronized
 * output around a cursor-home overwrite. Each row is erased to end-of-line (so a
 * shorter line can't leave a longer previous line's tail behind), and the screen
 * is erased from the last row down (so a shorter frame can't leave a taller
 * previous frame's rows behind) — the overlap/ghosting fix. Deliberately never
 * emits `\x1b[2J` (a full clear reads as a flash); EL+ED clear precisely what the
 * overwrite leaves.
 */
export function frameSequence(content: string): string {
  const cleared = content.split('\r\n').join(EL + '\r\n') + EL + ED;
  return BSU + HOME + cleared + ESU;
}

/** Frame throttle: render only once `minInterval` ms have passed since `last`.
 *  `last < 0` means "never drawn", so the first frame always draws. */
export function shouldDraw(last: number, now: number, minInterval: number): boolean {
  return last < 0 || now - last >= minInterval;
}

const FPS_NORMAL = 30;
const FPS_REDUCED = 12;

export interface RenderLoopOptions {
  term: Terminal;
  reduceMotion: boolean;
  /** The composed screen for this frame — the live edge, already styled and
   *  joined with `\r\n` row separators. Returning `null` holds the last frame
   *  (e.g. a full-screen page is up and must not be overdrawn). */
  view: () => string | null;
}

export interface LoopHandle {
  stop(): void;
}

/**
 * Drive the live edge: one `requestAnimationFrame` loop, throttled, that asks
 * `view()` for the composed screen and writes exactly one synchronized frame.
 * The composed View redraws as partials/commits arrive — never a full clear, so
 * settled text above the edge stays still.
 */
export function startRenderLoop(opts: RenderLoopOptions): LoopHandle {
  const minInterval = 1000 / (opts.reduceMotion ? FPS_REDUCED : FPS_NORMAL);
  let raf = 0;
  let lastDraw = -1;

  const frame = (t: number): void => {
    raf = requestAnimationFrame(frame);
    // Nothing to draw for a hidden tab; the live edge resumes on foreground.
    if (document.hidden) return;
    if (!shouldDraw(lastDraw, t, minInterval)) return;

    const { cols, rows } = opts.term;
    if (cols === 0 || rows === 0) return;

    const content = opts.view();
    if (content === null) return; // a page holds the frame

    lastDraw = t;
    opts.term.write(frameSequence(content));
  };

  raf = requestAnimationFrame(frame);
  return {
    stop() {
      cancelAnimationFrame(raf);
    },
  };
}
