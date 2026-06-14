import './style.css';
import init from './wasm/talk_wasm.js';
import { createTerminal } from './terminal';
import { startRenderLoop, type LoopHandle } from './loop';
import { Theme, parseComposed, themeTones, type Rgb } from './theme';
import { resolveModelState, downloadModels, type DownloadProgress } from './asr/download';
import {
  DemoModeSession,
  type LiveSessionView,
} from './session/demo-session';
import { LiveModeSession } from './session/live-session';
import {
  ModeRouter,
  type ModeClock,
} from './session/modes';
import {
  JournalStore,
  detectPrivateMode,
  durabilityWarnings,
  DURABILITY_WARNING_COPY,
  type StorageEvent,
  type StorageLike,
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
  cleanupForMode,
  type SessionMode,
} from './mobile';
import { renderBoot, INSTALL_HINT, type BootLine, type BootTone } from './boot';
import { parseHash } from './deeplink';

const MB = 1024 * 1024;

/** The web app version stamped into the boot MOTD. Tracks web/package.json. */
const VERSION = '0.1.0';

/**
 * Engine selection (the U6 spike). The live app defaults to the REAL
 * browser-native engine (`TransformersRecognizer` behind `LiveModeSession`) so
 * `npm run dev` actually transcribes; `?engine=mock` forces the scripted demo
 * (the path the unit tests drive). The Whisper model id is overridable via
 * `?model=` so the user can try `onnx-community/whisper-tiny.en` (smaller/faster).
 */
const USE_REAL_ENGINE_DEFAULT = true;

/** Resolve the engine + model id from the URL, falling back to the defaults. The
 *  Demo path takes no model id; the real path defaults to whisper-base.en. */
function resolveEngine(search: string): { real: boolean; modelId: string | undefined } {
  const params = new URLSearchParams(search);
  const engine = params.get('engine');
  const real = engine === 'real' ? true : engine === 'mock' ? false : USE_REAL_ENGINE_DEFAULT;
  const modelParam = params.get('model');
  return { real, modelId: modelParam ?? undefined };
}

/** How long the login/MOTD boot banner holds before the front door takes over;
 *  any key dismisses it early. Reduced motion shortens it (R26 calm parity). */
const BOOT_DWELL_MS = 4200;
const BOOT_DWELL_MS_REDUCED = 1800;

/** A dedicated last-visit stamp for the boot banner's "Last visit …" line, kept
 *  in its own localStorage key so it is independent of the journal blob's schema
 *  (boot must work even before any entry is kept, and a journal-schema bump must
 *  not disturb the visit clock). Best-effort: a storage failure simply degrades
 *  to a first-visit welcome. */
const LAST_VISIT_KEY = 'talk.last-visit.v1';

/** Read the previous visit's epoch-ms, or null on a first visit / unreadable
 *  storage (→ the boot banner shows the first-reflection welcome). */
function readLastVisit(backend: StorageLike): number | null {
  try {
    const raw = backend.getItem(LAST_VISIT_KEY);
    if (raw === null) return null;
    const n = Number(raw);
    return Number.isFinite(n) ? n : null;
  } catch {
    return null;
  }
}

/** Stamp this visit (call once at boot, AFTER reading the previous one). */
function markVisit(backend: StorageLike, now: number): void {
  try {
    backend.setItem(LAST_VISIT_KEY, String(now));
  } catch {
    // No durable visit clock here just means the next boot welcomes again —
    // harmless, never a failure.
  }
}

/**
 * The up-front privacy assertion (R5) — the precisely-worded promise, shown ONCE
 * on a visitor's first load. localStorage flag, so a returning reflector is not
 * re-nagged. Worded to match index.html's meta description + the CSP's intent:
 * the one-time model download is the only thing that ever crosses the network.
 */
const PRIVACY_ASSERTION =
  'after a one-time model download, nothing you say or write leaves your browser.';
const PRIVACY_SEEN_KEY = 'talk.privacy-seen.v1';

/** True the first time this browser profile loads the app (R5 one-time gate). */
function isFirstVisit(backend: StorageLike): boolean {
  try {
    return backend.getItem(PRIVACY_SEEN_KEY) === null;
  } catch {
    // Storage unavailable (hard private-mode block): treat as first visit so the
    // promise is still shown — better to over-show the assertion than to hide it.
    return true;
  }
}

/** Mark the privacy assertion as shown so it appears only once. Best-effort. */
function markPrivacySeen(backend: StorageLike): void {
  try {
    backend.setItem(PRIVACY_SEEN_KEY, '1');
  } catch {
    // If we cannot persist the flag, the assertion simply shows again next visit
    // — a harmless re-affirmation of the promise, never a failure.
  }
}

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
  // click, and re-focus on click and when the tab regains focus. Held as named
  // handlers so `beforeunload` can detach them (no listeners leak past teardown).
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
  let boundSession: LiveSessionView | null = null;
  let completed = false;

  // Bridge the U7 controls → the router's completion: when a session's controls
  // report `finished`, the router lands (done) or discards (cancel) exactly once.
  // The session fires this after every control state change (key/chip), so
  // completion is observed without polling.
  const onControlsChange = (): void => {
    const session = boundSession;
    if (!session) return;
    const st = session.controlsState();
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
  // A quota failure means the just-kept entry did NOT durably persist (R23/R17):
  // hold its banner longer (it is the cue to export now) than an advisory/transient
  // failure, so the user has a real chance to act before it scrolls away.
  const QUOTA_NOTICE_DWELL_MS = 20000;
  const store = new JournalStore({
    backend: localStorage,
    onStorageEvent: (event: StorageEvent) =>
      showNotice(
        event.message,
        event.kind === 'quota-exceeded' ? QUOTA_NOTICE_DWELL_MS : NOTICE_DWELL_MS,
      ),
    onFirstKeep: () =>
      showNotice('first entry kept — open your journal ( v ) and export ( e ) to keep a copy off this browser'),
    onExternalChange: () => {
      // Another tab kept/cleared an entry — the journal view (if open) is now stale.
      // The rAF loop repaints from the store on the next frame; nothing to do here
      // beyond letting the mirror reload (which the store already did).
    },
  });
  const privateMode = detectPrivateMode(localStorage);
  const exportSink = browserExportSink();
  const exportDisclosure = new ExportDisclosure();

  // Up-front privacy assertion (R5): on a first visit, lead with the promise —
  // held longer than an ordinary notice because it is the load-bearing claim the
  // whole app is built to honor (the strict CSP + the net-silence canary are its
  // enforcement). Shown once; a returning reflector is not re-nagged. It rides
  // alongside the taste-it preview below, so the visitor reads the promise WHILE
  // the live edge animates a sample reflection.
  if (isFirstVisit(localStorage)) {
    const PRIVACY_DWELL_MS = 14000;
    showNotice(PRIVACY_ASSERTION, PRIVACY_DWELL_MS);
    markPrivacySeen(localStorage);
  }

  // The mode router (U8) owns the experience flow: it boots into the reflect
  // front door, runs a session through the demo pipeline + U7 controls, lands the
  // entry through the durable store seam (U9), then routes to the between-session
  // picker. A repaint is implicit via the rAF loop.
  // Engine choice (the U6 spike): the live app defaults to the real
  // transformers.js Whisper engine so `npm run dev` transcribes; `?engine=mock`
  // forces the scripted demo (the tests' path). The model/mic status flows into
  // the bottom status line so the first-run download + permission states show.
  const { real: useRealEngine, modelId } = resolveEngine(window.location.search);
  const router = new ModeRouter({
    store,
    clock: browserClock(),
    reduceMotion,
    session: (mode) =>
      useRealEngine
        ? new LiveModeSession(mode, onControlsChange, {
            modelId,
            onModelStatus: (s) => {
              statusText = s.message;
            },
            onMicState: (d) => {
              // Surface a non-listening mic state (waiting / denied / busy) so the
              // user isn't left guessing; a granted mic falls back to the model line.
              if (d.state !== 'granted') statusText = d.message;
            },
          })
        : new DemoModeSession(mode, onControlsChange),
  });

  // The login-style boot banner (U11): a "Last visit … on talk.pilgrimapp.org"
  // line over a short MOTD (version stamp + install funnel), painted in the rust
  // theme over the live front door for a calm dwell, then dismissed (any key
  // skips it). boot.ts returns tone-tagged lines; the host paints them — keeping
  // the banner copy pure + tested. Read the previous visit BEFORE stamping this
  // one, so the line shows when you were last here, not "just now".
  const lastVisit = readLastVisit(localStorage);
  markVisit(localStorage, Date.now());
  const bootLines: readonly BootLine[] = renderBoot(VERSION, lastVisit, Date.now());
  let showingBoot = true;
  const bootDwell = reduceMotion ? BOOT_DWELL_MS_REDUCED : BOOT_DWELL_MS;
  const dismissBoot = (): void => {
    showingBoot = false;
  };
  const bootTimer = setTimeout(dismissBoot, bootDwell);

  // Deep-link (U11): `#q=<question-id>` opens that specific reflect question on
  // load IF it resolves to a known question in the pack. The hash is already run
  // through deeplink.ts's SAFE_ID alphabet gate + control-byte neutralizer, so
  // an attacker-controlled fragment can never reach the terminal; an unknown
  // (but safe) id is ignored and the router keeps its normal front-door
  // selection. Applied once at boot — the banner still rides on top.
  const link = parseHash(window.location.hash);
  if (link.questionId !== undefined) {
    router.startWithQuestion(link.questionId);
  }

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
  // The commands/help overlay (web addition): `?` or `/` lists every command the
  // current screen accepts plus the globals. Any key closes it. A cheat-sheet, not
  // a type-to-run palette — discoverability without a new input mode.
  let showingHelp = false;
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
    // The router types sessions as the narrower ModeSession, but THIS host's
    // sessionFactory only ever builds LiveSessionView instances (DemoModeSession |
    // LiveModeSession), so the cast is sound. (The old `instanceof DemoModeSession`
    // check silently excluded the real LiveModeSession — keys never bound.)
    const live = router.liveSession() as LiveSessionView | null;
    if (live) {
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

  /** Start the current session's capture — MUST run from a user gesture
   *  (keypress/click), because getUserMedia + AudioContext are gesture-gated.
   *  Idempotent, so calling it on every session-starting gesture is safe. */
  const beginCurrentSession = (): void => {
    (router.liveSession() as LiveSessionView | null)?.begin();
  };

  // The deferred clipboard-disclosure note's timer — held so unload can clear it
  // (a pending setTimeout firing after teardown would touch a dead screen).
  let disclosureTimer: ReturnType<typeof setTimeout> | null = null;

  /** Export the whole journal (the `:export` command / the journal-view chip).
   *  Downloads a CLI-identical markdown file; the confirmation lands as a notice. */
  const exportJournal = (channel: 'download' | 'clipboard' = 'download'): void => {
    const entries = store.allEntries();
    if (entries.length === 0) {
      showNotice('nothing to export yet');
      return;
    }
    const scope: ExportScope = { kind: 'all', entries };
    runExport(scope, channel, exportSink, exportDisclosure)
      .then((result) => {
        showNotice(result.message);
        // Capture the narrowed value before the closure so it's a plain string,
        // not a re-read of the (widening) result field — no cast needed.
        const disclosure = result.clipboardDisclosure;
        if (disclosure) {
          if (disclosureTimer !== null) clearTimeout(disclosureTimer);
          // The one-time OS-shared note follows the confirmation on the next frame.
          disclosureTimer = setTimeout(() => {
            disclosureTimer = null;
            showNotice(disclosure);
          }, 1500);
        }
      })
      .catch((err) => {
        showNotice('export failed — try again');
        console.error('export failed', err);
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
        session?.controlsCommand(command);
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
        beginCurrentSession();
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
    if (showingBoot) {
      // Any key skips the login banner straight into the front door — and this
      // keypress is the user gesture that lets the front-door session start the mic.
      clearTimeout(bootTimer);
      dismissBoot();
      beginCurrentSession();
      return;
    }
    // The commands/help overlay swallows the next key to close — so the key that
    // dismisses help never also fires a command underneath (e.g. space ≠ done).
    if (showingHelp) {
      showingHelp = false;
      return;
    }
    if (data === '?' || data === '/') {
      showingHelp = true;
      return;
    }
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
      if (data === 'i') {
        // The soft install funnel: surface the CLI install line as a notice, not
        // a nag. The same hint also rides the boot MOTD; this is the on-demand
        // echo for a returning visitor who skipped the banner.
        showNotice(`run it in your real terminal:  ${INSTALL_HINT}`);
        return;
      }
      const mode = pickerKey(data);
      if (mode) {
        router.start(mode);
        beginCurrentSession(); // the picker keypress is the gesture
      }
      return;
    }
    // session phase
    // Catch-all gesture: if boot auto-dismissed on its timer (no keypress), this
    // first session key is the gesture that starts the mic. Idempotent.
    beginCurrentSession();
    const session = boundSession;
    if (!session) return;
    if (session.controlsKey(data)) return;
    if (data === 'n') router.command('new-question');
  });

  // A click/tap anywhere is ALSO a "begin" gesture — robust when the terminal
  // lacks keyboard focus (e.g. DevTools is open), where xterm never receives the
  // "press any key" keystroke. A click dismisses the boot banner, starts the
  // session's mic (the gesture getUserMedia/AudioContext require), and focuses the
  // terminal so subsequent keys land.
  document.addEventListener('pointerdown', () => {
    if (showingBoot) {
      clearTimeout(bootTimer);
      dismissBoot();
    }
    beginCurrentSession();
    term.focus();
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
    beginCurrentSession();
  };

  const composeSession = (session: LiveSessionView, mode: SessionMode): string => {
    const json = session.compose({
      mode,
      question: router.questionText() ?? '',
      heldLabel: '', // U8 selection has no held-label surface yet
      elapsed: '0:00',
      cleanup: cleanupForMode(mode), // journal → High (paragraphs), reflect/unburden → Light
    });
    return theme.renderComposed(parseComposed(json));
  };

  /** Paint the boot banner (U11): each tone-tagged line through the rust theme,
   *  indented to sit calmly off the left edge, with a closing hint. boot.ts owns
   *  the COPY decision; this only maps tones → the shared palette. */
  const composeBoot = (): string => {
    const paintTone = (line: BootLine): string => {
      if (line.text === '') return '';
      const tone: BootTone = line.tone;
      const painted =
        tone === 'core' ? theme.core(line.text)
        : tone === 'dim' ? theme.dim(line.text)
        : theme.edge(line.text);
      return `  ${painted}`;
    };
    const body = bootLines.map(paintTone).join('\r\n');
    const hint = theme.edge('  press any key to begin');
    return `${body}\r\n\r\n${hint}`;
  };

  const composePicker = (): string => {
    const head = theme.core('  take a breath. what now?');
    const opts = router
      .pickerOptions()
      .map((m, i) => theme.dim(`    [${i + 1}] ${PICKER_LABELS[m]}`))
      .join('\r\n');
    const hint = theme.edge('  press 1 · 2 · 3   (or r · j · u)   ·   v  journal   ·   i  install   ·   ?  help');
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

    const hint = theme.edge('  e  export   ·   c  continue a thread   ·   b / esc  back   ·   ?  help');
    lines.push(hint);
    return lines.join('\r\n');
  };

  /** The commands/help overlay (`?` or `/`). Context-aware: it mirrors exactly
   *  what the current screen accepts, then the always-available globals and the
   *  privacy promise. view.ts/compose* own the inline hints; this is the full
   *  reference behind one keystroke. */
  const composeHelp = (): string => {
    const rows: string[] = [theme.core('  commands'), ''];
    const section = (title: string, lines: readonly string[]): void => {
      rows.push(theme.edge(`  ${title}`));
      for (const l of lines) rows.push(theme.dim(`    ${l}`));
      rows.push('');
    };
    if (viewingJournal) {
      section('your journal', ['e  export', 'c  continue a thread', 'b / esc  back']);
    } else {
      const phase = router.currentPhase();
      if (phase === 'session') {
        section('this reflection', [
          'space  done — keep it and continue',
          'u  show raw ⇄ cleaned text',
          'p  pause (off the record)',
          'n  ask a different question',
          'esc  cancel — discard this one',
        ]);
      } else if (phase === 'picker') {
        section('choose a mode', [
          '1 / r  reflect',
          '2 / j  journal',
          '3 / u  unburden — nothing is kept',
          'v  read your journal',
          'i  install the CLI',
        ]);
      } else if (phase === 'closing') {
        section('this moment', ['any key  continue']);
      }
    }
    section('always', ['?  or  /   this help']);
    rows.push(theme.dim('  nothing you say or write leaves your browser.'));
    rows.push('');
    rows.push(theme.edge('  press any key to close'));
    return rows.join('\r\n');
  };

  const composeView = (): string => {
    syncSession();
    if (showingBoot) return composeBoot();
    if (showingHelp) return composeHelp();
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

  // U5 model orchestration drives the SHERPA model manifest (cdn.pilgrimapp.org).
  // The transformers.js spike engine fetches + caches its OWN model from the HF
  // CDN inside the worker (status flows through onModelStatus above), so skip the
  // sherpa download when the real transformers engine is active — running both
  // would double-download and fight over the status line.
  if (!useRealEngine) {
    void orchestrateModels((text) => {
      statusText = text;
    });
  }

  // Stop the loop + tear down the router/session + detach every global listener
  // and timer when the page unloads, so nothing fires against a dead screen.
  const onUnload = (): void => {
    loop.stop();
    router.dispose();
    store.dispose();
    clearTimeout(bootTimer);
    if (noticeTimer !== null) clearTimeout(noticeTimer);
    if (disclosureTimer !== null) clearTimeout(disclosureTimer);
    document.removeEventListener('pointerdown', refocus);
    window.removeEventListener('focus', refocus);
    window.removeEventListener('resize', refit);
    window.visualViewport?.removeEventListener('resize', refit);
    window.removeEventListener('beforeunload', onUnload);
    term.dispose();
  };
  window.addEventListener('beforeunload', onUnload);
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
