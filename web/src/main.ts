import './style.css';
import init, { Settle } from './wasm/talk_wasm.js';
import { createTerminal } from './terminal';
import { frameSequence } from './loop';
import { Repl } from './repl';
import { Theme, parseComposed, themeTones, type Rgb } from './theme';

const PROMPT = '❯ ';

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
 * Build a representative static reflect screen: a question, a settled line, a dim
 * live-edge line, and the status/chrome lines. This is the U4 "it paints"
 * milestone — full session wiring (mic, modes, journal) lands in later units.
 */
function staticReflectScreen(theme: Theme): string {
  const settle = new Settle();
  settle.commit('um i think i have been', 'I think I have been holding my breath all week.');
  settle.finalize();
  settle.onPartial('and what i want is to slow down');

  const json = settle.compose(
    'reflect',
    'What am I avoiding?',
    '', // held_label
    true, // listening — the live edge is active
    '2:14',
    'Light',
    false, // show_raw
    false, // paused
    false, // confirm_cancel
  );
  const body = theme.renderComposed(parseComposed(json));
  settle.free();
  return body;
}

async function boot(): Promise<void> {
  await init();

  const screen = document.getElementById('screen');
  if (!screen) throw new Error('missing #screen mount');

  const { term, fit } = createTerminal(screen);
  term.write('\x1b[?25l'); // hide the terminal cursor; the REPL renders its own

  const themeName = 'rust';
  applyPaletteToCss(themeName);
  const theme = Theme.load(themeName);

  // Keyboard devices: grab focus so keystrokes land in the terminal without a
  // click, and re-focus on click and when the tab regains focus.
  const refocus = (): void => term.focus();
  refocus();
  document.addEventListener('pointerdown', refocus);
  window.addEventListener('focus', refocus);

  // A minimal interactive REPL so the terminal responds to input. Commands are
  // stubbed for U4 — full dispatch (modes, controls) arrives in later units.
  const repl = new Repl(() => ['reflect', 'journal', 'unburden', 'help']);
  let statusText = '';

  const paint = (): void => {
    const { cols, rows } = term;
    if (cols === 0 || rows === 0) return;
    const composed = staticReflectScreen(theme);
    const bottom = statusText
      ? theme.edge(`  ${statusText}`)
      : `  ${repl.line(theme.dim(PROMPT), "type 'help'")}`;
    term.write(frameSequence(`${composed}\r\n\r\n${bottom}\x1b[K`));
  };

  term.onData((data) => {
    const result = repl.handle(data);
    if (result.submitted !== undefined) {
      statusText = `'${result.submitted}' — session wiring lands in a later unit`;
    } else if (result.changed) {
      statusText = '';
    }
    paint();
  });

  const refit = (): void => {
    fit.fit();
    paint();
  };
  refit();
  window.addEventListener('resize', refit);
  window.visualViewport?.addEventListener('resize', refit);

  dismissLoading();
  paint();
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
