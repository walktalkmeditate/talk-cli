import './style.css';
import init from './wasm/talk_wasm.js';
import { createTerminal } from './terminal';
import { startRenderLoop, type LoopHandle } from './loop';
import { Theme, parseComposed, themeTones, type Rgb } from './theme';
import { resolveModelState, downloadModels, type DownloadProgress } from './asr/download';
import { Pipeline } from './asr/pipeline';
import { MockRecognizer, type ScriptStep } from './asr/recognizer';
import { SessionControls, type SessionCommand } from './session/controls';
import {
  isTouch,
  createChipBar,
  chipsFor,
  sessionChipScreen,
  isEphemeralMode,
  cleanupForMode,
  type SessionMode,
} from './mobile';

/** Session commands the controller handles directly; other chip verbs
 *  (new-question/new-entry/export/back) are routed by the U8/U9 mode router. */
const SESSION_COMMANDS: ReadonlySet<string> = new Set<SessionCommand>([
  'done',
  'pause',
  'toggle-raw',
  'cancel',
]);

const MB = 1024 * 1024;

function fmtMb(bytes: number): string {
  return `${(bytes / MB).toFixed(0)} MB`;
}

/**
 * First-run model-acquisition status as a single terminal line. U5 owns the LOGIC
 * + states; the rich first-run screens are refined with U6/U10. Returns the line
 * for a given download phase/progress.
 */
function modelStatusLine(p: DownloadProgress): string {
  switch (p.phase) {
    case 'complete':
      return 'models ready';
    case 'pre-accept':
      return 'one-time model download needed (~327 MB) — starting…';
    case 'blocked':
      return 'offline — connect once to download the models';
    case 'paused':
      return `download paused (${fmtMb(p.receivedBytes)} fetched) — will resume`;
    case 'error':
      return 'model download failed — reload to retry';
    case 'downloading': {
      const pct = p.fraction === null ? null : Math.round(p.fraction * 100);
      const tail = pct === null ? fmtMb(p.receivedBytes) : `${pct}% (${fmtMb(p.receivedBytes)})`;
      return `downloading models — ${tail}`;
    }
  }
}

/**
 * Push the loaded palette into CSS custom properties so the page chrome (touch
 * chips, focus rings) derives from the same single-source-of-truth palette the
 * terminal paints from, instead of a drifting hard-coded rust accent. The CSS
 * uses `rgba(var(--accent-rgb), <a>)`; here we set just the `r,g,b` triple.
 *
 * Falls back to the `:root` default (set in style.css for the pre-JS paint) when
 * the theme is Mono (null tones — no fixed RGB).
 */
function applyPaletteToCss(theme: string): void {
  const tones = themeTones(theme);
  const setRgb = (name: string, rgb: Rgb | null): void => {
    if (rgb === null) return;
    document.documentElement.style.setProperty(name, `${rgb[0]}, ${rgb[1]}, ${rgb[2]}`);
  };
  setRgb('--accent-rgb', tones.core);
  setRgb('--dim-rgb', tones.dim);
  setRgb('--edge-rgb', tones.edge);
}

/** Fade out and remove the zero-JS loading placeholder once the screen is live. */
function dismissLoading(): void {
  const el = document.getElementById('loading');
  if (!el) return;
  el.classList.add('gone');
  setTimeout(() => el.remove(), 600);
}

/**
 * A scripted reflection for the live demo. The MockRecognizer replays it so the
 * page animates the real settle → render path end-to-end NOW — dim partials
 * jittering, an endpoint settling them, the pass-2 finalize upgrading to bright
 * final text. This is a DEMO driver, not real transcription; the one-line swap to
 * `WireSherpaRecognizer` (recognizer.ts) lights up the real on-device engine once
 * the sherpa-onnx WASM assets exist.
 */
const DEMO_SCRIPT: readonly ScriptStep[] = [
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

const DEMO_STEP_MS = 700;

/** A running demo session: the Pipeline driven by a timer over the MockRecognizer. */
interface DemoSession {
  pipeline: Pipeline;
  timer: ReturnType<typeof setInterval>;
  recognizer: MockRecognizer;
}

/**
 * Start the live demo session: build a Pipeline over a MockRecognizer and walk
 * the script on a timer so the live edge animates. A real session (U7+) replaces
 * the timer with mic frames and the mock with `WireSherpaRecognizer`.
 */
function startDemoSession(onChange: () => void): DemoSession {
  const recognizer = new MockRecognizer(DEMO_SCRIPT, { advanceOnAudio: false });
  const pipeline = new Pipeline({ recognizer, onChange });
  const timer = setInterval(() => {
    if (!recognizer.step()) {
      clearInterval(timer);
    }
  }, DEMO_STEP_MS);
  return { pipeline, timer, recognizer };
}

async function boot(): Promise<void> {
  await init();

  const screen = document.getElementById('screen');
  if (!screen) throw new Error('missing #screen mount');

  const { term, fit } = createTerminal(screen);
  term.write('\x1b[?25l'); // hide the terminal cursor; the live edge renders its own

  const themeName = 'rust';
  applyPaletteToCss(themeName);
  const theme = Theme.load(themeName);

  // Keyboard devices: grab focus so keystrokes land in the terminal without a
  // click, and re-focus on click and when the tab regains focus.
  const refocus = (): void => term.focus();
  refocus();
  document.addEventListener('pointerdown', refocus);
  window.addEventListener('focus', refocus);

  let statusText = '';

  // The live demo session drives the real settle/pairing pipeline + render path.
  const session = startDemoSession(() => {
    /* the rAF loop reads the composed view each frame; no explicit repaint */
  });

  // The session interaction layer (U7): keys (space/u/p/esc) + chip taps map onto
  // the pipeline, and the show-raw / paused / confirm-cancel UI state flows into
  // Settle.compose. The demo runs in reflect; the U8 router selects the mode per
  // session (kept widely typed so that swap is a one-liner).
  const mode: SessionMode = 'reflect';
  const controls = new SessionControls({
    pipeline: session.pipeline,
    ephemeral: isEphemeralMode(mode),
  });

  // Desktop: route xterm key bytes through the controller so space/u/p/esc fire
  // regardless of which element has focus (term.onData is the focused-terminal
  // stream; refocus() above keeps the terminal focused without a click).
  term.onData((data) => {
    controls.onKey(data);
  });

  // Mobile: the chip bar is the only control surface. Mount the per-mode set and
  // route taps through the SAME controller entry point as the keys.
  if (isTouch()) {
    const chipBar = createChipBar(chipsFor(sessionChipScreen(mode)), (command) => {
      if (SESSION_COMMANDS.has(command)) {
        controls.command(command as SessionCommand);
      }
      // new-question / new-entry / export / back are wired by the U8/U9 router.
    });
    document.body.appendChild(chipBar);
  }

  const composeView = (): string => {
    const idle = session.pipeline.idleStatus();
    const ctl = controls.state();
    const json = session.pipeline.settle.compose(
      mode,
      'What am I avoiding?',
      '', // held_label
      idle.listening,
      '0:00',
      cleanupForMode(mode), // journal → High (paragraphs), reflect → Light
      ctl.showRaw, // `u` flips raw verbatim ⇄ cleaned text
      ctl.paused,
      ctl.confirmCancel,
    );
    return theme.renderComposed(parseComposed(json));
  };

  const renderView = (): string | null => {
    const { cols, rows } = term;
    if (cols === 0 || rows === 0) return null;
    const body = composeView();
    const bottom = statusText ? theme.edge(`  ${statusText}`) : '';
    return `${body}\r\n\r\n${bottom}\x1b[K`;
  };

  const loop: LoopHandle = startRenderLoop({
    term,
    reduceMotion: window.matchMedia('(prefers-reduced-motion: reduce)').matches,
    view: renderView,
  });

  const refit = (): void => {
    fit.fit();
  };
  refit();
  window.addEventListener('resize', refit);
  window.visualViewport?.addEventListener('resize', refit);

  dismissLoading();

  void orchestrateModels((text) => {
    statusText = text;
  });

  // Stop the loop + the session when the page unloads (clean teardown).
  window.addEventListener('beforeunload', () => {
    clearInterval(session.timer);
    loop.stop();
    session.pipeline.free();
  });
}

/**
 * First-run model orchestration (U5). On load: if every model file is cached and
 * verifies, proceed silently; if absent and online, download with a live status
 * line; if absent and offline, surface the blocked state. The session pipeline
 * (U6) consumes the cached, verified files once this resolves complete.
 */
async function orchestrateModels(onStatus: (text: string) => void): Promise<void> {
  try {
    const state = await resolveModelState();
    if (state.phase === 'complete') {
      onStatus(modelStatusLine({ phase: 'complete', files: [], receivedBytes: 0, totalBytes: 0, fraction: 1 }));
      return;
    }
    if (state.phase === 'blocked') {
      onStatus(modelStatusLine({ phase: 'blocked', files: [], receivedBytes: 0, totalBytes: null, fraction: null }));
      return;
    }
    await downloadModels({ onProgress: (p) => onStatus(modelStatusLine(p)) });
  } catch (err) {
    onStatus(`models unavailable — ${err instanceof Error ? err.message : String(err)}`);
  }
}

boot().catch((err) => {
  const loading = document.getElementById('loading');
  if (loading) {
    loading.classList.remove('gone');
    const span = loading.querySelector('span');
    if (span) span.textContent = 'could not start — please reload';
  }
  console.error(err);
});
