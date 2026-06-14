import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import '@xterm/xterm/css/xterm.css';

const FONT_STACK =
  '"SF Mono", "JetBrains Mono", "Cascadia Code", Menlo, Consolas, monospace';

/**
 * Build the xterm.js terminal: WebGL for truecolor at frame rate, the fit addon
 * for sizing, the rust theme, and the cursor hidden during the live edge. The
 * base background matches the page's dark warm ground (`#14100e`) so the fit
 * margins are invisible; the rust tones themselves come from `theme.ts` (the
 * single palette source), not from this theme object.
 */
export function createTerminal(container: HTMLElement): {
  term: Terminal;
  fit: FitAddon;
} {
  const term = new Terminal({
    allowProposedApi: true,
    cursorBlink: false,
    cursorStyle: 'bar',
    fontFamily: FONT_STACK,
    fontSize: 15,
    lineHeight: 1.0,
    scrollback: 0,
    theme: {
      background: '#14100e',
      foreground: '#e6d5c8',
      cursor: '#e6d5c8',
    },
  });

  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(container);

  // WebGL is essential for truecolor at frame rate; fall back silently to the
  // DOM renderer if the context can't be created (older/headless GPUs).
  try {
    term.loadAddon(new WebglAddon());
  } catch {
    /* DOM renderer fallback */
  }

  fit.fit();
  return { term, fit };
}
