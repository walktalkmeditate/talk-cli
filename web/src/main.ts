import './style.css';
import init from './wasm/talk_wasm.js';
import { createTerminal } from './terminal';
import { startRenderLoop, type LoopHandle } from './loop';
import { Theme, parseComposed, themeTones, type Rgb } from './theme';
import { resolveModelState, downloadModels, type DownloadProgress } from './asr/download';
import { Pipeline } from './asr/pipeline';
import { MockRecognizer, type ScriptStep } from './asr/recognizer';
import { SessionControls } from './session/controls';
import {
  ModeRouter,
  type ModeClock,
  type ModeSession,
} from './session/modes';
import {
  JournalStore,
  detectPrivateMode,
  durabilityWarnings,
  DURABILITY_WARNING_COPY,
  type StorageEvent,
} from './journal/store';
import { buildJournalView, continueThread, type JournalThreadGroup } from './journal/view';
import {
  runExport,
  browserExportSink,
  ExportDisclosure,
  type ExportScope,
} from './journal/export';
import {
  isTouch,
  createChipBar,
  chipsFor,
  sessionChipScreen,
  isEphemeralMode,
  cleanupForMode,
  type SessionMode,
} from './mobile';

const MB = 1024 * 1024;

function fmtMb(bytes: number): string {
  return `${(bytes / MB).toFixed(0)} MB`;
}

/** One-line preview of an entry body for the journal list (first line, clipped). */
function truncate(text: string, max = 64): string {
  const firstLine = text.split('\n')[0] ?? '';
  return firstLine.length > max ? `${firstLine.slice(0, max - 1)}…` : firstLine;
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
 * terminal paints from, instead of a drifting hard-coded rust accent.
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
 * `WireSherpaRecognizer` (recognizer.ts, U6's WIRE seam) lights up the real
 * on-device engine once the sherpa-onnx WASM assets exist.
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

/**
 * A demo-backed session the ModeRouter drives: a Pipeline over a MockRecognizer
 * walked on a timer, wrapped with the U7 SessionControls. It exposes the
 * ModeSession surface (final text + cancel flag + buffer-wiping `end`) the router
 * lands an entry from. The real session (U6 WIRE seam) replaces the timer with
 * mic frames and the mock with WireSherpaRecognizer behind this same shape.
 */
class DemoModeSession implements ModeSession {
  readonly pipeline: Pipeline;
  readonly controls: SessionControls;
  private readonly recognizer: MockRecognizer;
  private readonly timer: ReturnType<typeof setInterval>;
  private cancelled = false;

  constructor(mode: SessionMode, onControlsChange: () => void) {
    this.recognizer = new MockRecognizer(DEMO_SCRIPT, { advanceOnAudio: false });
    this.pipeline = new Pipeline({ recognizer: this.recognizer });
    this.controls = new SessionControls({
      pipeline: this.pipeline,
      ephemeral: isEphemeralMode(mode),
      onChange: onControlsChange,
    });
    this.timer = setInterval(() => {
      if (!this.recognizer.step()) clearInterval(this.timer);
    }, DEMO_STEP_MS);
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
    clearInterval(this.timer);
    this.pipeline.free();
  }
}

const PICKER_LABELS: Record<SessionMode, string> = {
  reflect: 'reflect',
  journal: 'journal',
  ephemeral: 'unburden',
};

/** The browser clock the router selects + timestamps from (local-civil). */
function browserClock(): ModeClock {
  const pad = (n: number): string => String(n).padStart(2, '0');
  return {
    hour: () => new Date().getHours(),
    stamp: () => {
      const d = new Date();
      return {
        date: `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`,
        time: `${pad(d.getHours())}:${pad(d.getMinutes())}`,
      };
    },
    now: () => performance.now(),
  };
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

  const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  // The active session bound to the keyboard/chip surface, and a once-guard so a
  // finished session lands/discards exactly once. Declared BEFORE the router
  // because the router boots a reflect session in its constructor (which calls
  // the session factory synchronously, wiring `onControlsChange`).
  let boundSession: DemoModeSession | null = null;
  let completed = false;

  // Bridge the U7 controls → the router's completion: when a session's controls
  // report `finished`, the router lands (done) or discards (cancel) exactly once.
  // The DemoModeSession fires this after every control state change (key/chip),
  // so completion is observed without polling.
  const onControlsChange = (): void => {
    const session = boundSession;
    if (!session) return;
    const st = session.controls.state();
    if (!st.finished || completed) return;
    completed = true;
    if (st.cancelled) {
      session.markCancelled();
      router.command('cancel');
    } else {
      router.command('done');
    }
  };

  // A transient banner the host surfaces below the screen: storage failures
  // (R23), the first-keep export prompt (R21), and durability/exposure warnings
  // (R21/R22). It auto-clears after a dwell so it never sticks. A quota failure
  // is the one banner held longer — it is the user's cue to export.
  let noticeText = '';
  let noticeTimer: ReturnType<typeof setTimeout> | null = null;
  const NOTICE_DWELL_MS = 8000;
  const showNotice = (text: string, dwellMs = NOTICE_DWELL_MS): void => {
    noticeText = text;
    if (noticeTimer !== null) clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => {
      noticeText = '';
      noticeTimer = null;
    }, dwellMs);
  };

  // The durable browser-local journal (U9) — the production EntryStore behind the
  // exact `modes.ts` seam, so the router is unchanged. `localStorage` is the
  // backend; a probe up front decides whether anything will persist (private
  // mode). Storage failures and the first-keep prompt surface as notices.
  const store = new JournalStore({
    backend: localStorage,
    onStorageEvent: (event: StorageEvent) => showNotice(event.message),
    onFirstKeep: () =>
      showNotice('first entry kept — open your journal ( v ) and export ( e ) to keep a copy off this browser'),
  });
  const privateMode = detectPrivateMode(localStorage);
  const exportSink = browserExportSink();
  const exportDisclosure = new ExportDisclosure();

  // The mode router (U8) owns the experience flow: it boots into the reflect
  // front door, runs a session through the demo pipeline + U7 controls, lands the
  // entry through the durable store seam (U9), then routes to the between-session
  // picker. A repaint is implicit via the rAF loop.
  const router = new ModeRouter({
    store,
    clock: browserClock(),
    reduceMotion,
    session: (mode) => new DemoModeSession(mode, onControlsChange),
  });

  // One-time durability/exposure warnings (R21/R22): surface them once the page
  // settles, opting into persistent storage first so the eviction-risk decision
  // reflects reality.
  void surfaceDurabilityWarnings();
  async function surfaceDurabilityWarnings(): Promise<void> {
    const persisted = privateMode ? false : await store.requestPersist();
    const warnings = durabilityWarnings({ privateMode, persisted, hasKept: store.hasKept() });
    if (warnings.length === 0) return;
    // Show the most urgent one (the list is ordered by urgency); the rest are
    // re-derivable from the journal view if the user opens it.
    showNotice(DURABILITY_WARNING_COPY[warnings[0]]);
  }

  // The host-level journal-view overlay (R18/R19): opened from the picker (`v`)
  // or the new-entry/back chips; rendered from view.ts via the theme. `false`
  // means a session/picker is showing, not the journal browser.
  let viewingJournal = false;
  const openJournalView = (): void => {
    viewingJournal = true;
    mountChips();
  };
  const closeJournalView = (): void => {
    viewingJournal = false;
    mountChips();
  };

  let chipBar: HTMLElement | null = null;
  const mountChips = (): void => {
    if (!isTouch()) return;
    if (chipBar) {
      chipBar.remove();
      chipBar = null;
    }
    if (viewingJournal) {
      chipBar = createChipBar(chipsFor('journal-view'), (command) => handleCommand(command));
      document.body.appendChild(chipBar);
      return;
    }
    if (router.currentPhase() !== 'session') return;
    const mode = router.mode();
    if (mode === null) return;
    chipBar = createChipBar(chipsFor(sessionChipScreen(mode)), (command) => handleCommand(command));
    document.body.appendChild(chipBar);
  };

  // Whenever the router swaps the live session, (re)bind it + remount the chips.
  // Called from the render loop (composeView) so it tracks every phase change.
  const syncSession = (): void => {
    const live = router.liveSession();
    if (live instanceof DemoModeSession) {
      if (live !== boundSession) {
        boundSession = live;
        completed = false;
        mountChips();
      }
    } else if (boundSession !== null) {
      boundSession = null;
      mountChips();
    }
  };

  /** Export the whole journal (the `:export` command / the journal-view chip).
   *  Downloads a CLI-identical markdown file; the confirmation lands as a notice. */
  const exportJournal = (channel: 'download' | 'clipboard' = 'download'): void => {
    const entries = store.allEntries();
    if (entries.length === 0) {
      showNotice('nothing to export yet');
      return;
    }
    const scope: ExportScope = { kind: 'all', entries };
    void runExport(scope, channel, exportSink, exportDisclosure).then((result) => {
      showNotice(result.message);
      if (result.clipboardDisclosure) {
        // The one-time OS-shared note follows the confirmation on the next frame.
        setTimeout(() => showNotice(result.clipboardDisclosure as string), 1500);
      }
    });
  };

  /** Route a normalized command (key or chip) to the controls, the router, or
   *  the host-level journal view (new-entry / export / back). */
  const handleCommand = (command: string): void => {
    const session = boundSession;
    switch (command) {
      case 'done':
      case 'pause':
      case 'toggle-raw':
      case 'cancel':
        session?.controls.command(command);
        return;
      case 'new-question':
        router.command('new-question');
        return;
      case 'export':
        exportJournal('download');
        return;
      case 'new-entry':
        closeJournalView();
        router.start('journal');
        return;
      case 'back':
        closeJournalView();
        return;
      default:
        return;
    }
  };

  // Desktop keys → commands. In the journal view: e=export, c=continue the first
  // thread, esc/b=back. In a session: space/u/p/esc drive the controls, `n`
  // re-rolls the reflect question. At the picker: r/j/u (or 1/2/3) pick a mode,
  // `v` opens the journal view. During the closure moment: any key dismisses it.
  term.onData((data) => {
    if (viewingJournal) {
      handleJournalViewKey(data);
      return;
    }
    const phase = router.currentPhase();
    if (phase === 'closing') {
      router.dismissClosure();
      return;
    }
    if (phase === 'picker') {
      if (data === 'v') {
        openJournalView();
        return;
      }
      const mode = pickerKey(data);
      if (mode) router.start(mode);
      return;
    }
    // session phase
    const session = boundSession;
    if (!session) return;
    if (session.controls.onKey(data)) return;
    if (data === 'n') router.command('new-question');
  });

  /** Keys while the journal view is open. `c` continues the most-recent thread —
   *  re-prompting that exact reflect question (R19's "continue a thread"). */
  const handleJournalViewKey = (data: string): void => {
    switch (data) {
      case 'e':
        exportJournal('download');
        return;
      case 'b':
      case '\x1b': // esc
        closeJournalView();
        return;
      case 'c': {
        const top = topThread();
        if (top) continueThreadView(top);
        return;
      }
      default:
        return;
    }
  };

  /** The most-recently-touched reflect thread, or null when there are none. */
  const topThread = (): JournalThreadGroup | null => {
    const vm = buildJournalView(store.journalDays(), store.threads());
    return vm.byThread.length > 0 ? vm.byThread[0] : null;
  };

  /** Continue a thread: resolve its full question (from the thread's first kept
   *  entry) and hand it to the router so the exact question is re-prompted. */
  const continueThreadView = (group: JournalThreadGroup): void => {
    const id = continueThread(group);
    const first = store.thread(id)[0];
    if (!first || first.question === null) return;
    closeJournalView();
    router.continueQuestion(first.question);
  };

  const composeSession = (session: DemoModeSession, mode: SessionMode): string => {
    const idle = session.pipeline.idleStatus();
    const ctl = session.controls.state();
    const json = session.pipeline.settle.compose(
      mode,
      router.questionText() ?? '',
      '', // held_label (U8 selection has no held-label surface yet)
      idle.listening,
      '0:00',
      cleanupForMode(mode), // journal → High (paragraphs), reflect/unburden → Light
      ctl.showRaw, // `u` flips raw verbatim ⇄ cleaned text
      ctl.paused,
      ctl.confirmCancel,
    );
    return theme.renderComposed(parseComposed(json));
  };

  const composePicker = (): string => {
    const head = theme.core('  take a breath. what now?');
    const opts = router
      .pickerOptions()
      .map((m, i) => theme.dim(`    [${i + 1}] ${PICKER_LABELS[m]}`))
      .join('\r\n');
    const hint = theme.edge('  press 1 · 2 · 3   (or r · j · u)   ·   v  read your journal');
    return `${head}\r\n\r\n${opts}\r\n\r\n${hint}`;
  };

  const composeClosing = (): string => {
    return router
      .closureLines()
      .map((l) => `  ${theme.core(l)}`)
      .join('\r\n');
  };

  /** The journal view (R18/R19): the by-date + by-thread IA rendered from
   *  view.ts. view.ts owns the grouping/ordering/empty-state DECISION; this only
   *  paints the resulting view-model in the theme. */
  const composeJournalView = (): string => {
    const vm = buildJournalView(store.journalDays(), store.threads());
    const lines: string[] = [theme.core('  your journal'), ''];

    if (vm.isEmpty) {
      lines.push(theme.dim(`    ${vm.emptyMessage}`));
    } else {
      if (vm.byDate.length > 0) {
        lines.push(theme.edge('  by date'));
        for (const group of vm.byDate) {
          lines.push(theme.dim(`    ${group.date}`));
          for (const e of group.entries) {
            lines.push(`      ${theme.core(`${e.time}  ${truncate(e.clean)}`)}`);
          }
        }
        lines.push('');
      }
      if (vm.byThread.length > 0) {
        lines.push(theme.edge('  by thread'));
        for (const t of vm.byThread) {
          lines.push(theme.dim(`    ${t.questionText}`));
          for (const e of t.entries) {
            lines.push(`      ${theme.core(`${e.date} ${e.time}  ${truncate(e.clean)}`)}`);
          }
        }
        lines.push('');
      }
    }

    const hint = theme.edge('  e  export   ·   c  continue a thread   ·   b / esc  back');
    lines.push(hint);
    return lines.join('\r\n');
  };

  const composeView = (): string => {
    syncSession();
    if (viewingJournal) return composeJournalView();
    const phase = router.currentPhase();
    if (phase === 'picker') return composePicker();
    if (phase === 'closing') return composeClosing();
    const session = boundSession;
    const mode = router.mode();
    if (!session || mode === null) return '';
    return composeSession(session, mode);
  };

  const renderView = (): string | null => {
    const { cols, rows } = term;
    if (cols === 0 || rows === 0) return null;
    const body = composeView();
    const notice = noticeText ? theme.dim(`  ${noticeText}`) : '';
    const bottom = statusText ? theme.edge(`  ${statusText}`) : '';
    return `${body}\r\n\r\n${notice}\x1b[K\r\n${bottom}\x1b[K`;
  };

  const loop: LoopHandle = startRenderLoop({
    term,
    reduceMotion,
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

  // Stop the loop + tear down the router/session when the page unloads.
  window.addEventListener('beforeunload', () => {
    loop.stop();
    router.dispose();
    if (noticeTimer !== null) clearTimeout(noticeTimer);
  });
}

/** The picker key → mode mapping (1/2/3 or r/j/u). */
function pickerKey(data: string): SessionMode | null {
  switch (data) {
    case '1':
    case 'r':
      return 'reflect';
    case '2':
    case 'j':
      return 'journal';
    case '3':
    case 'u':
      return 'ephemeral';
    default:
      return null;
  }
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
