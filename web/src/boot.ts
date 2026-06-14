// The boot moment (U11): a calm, login-style banner over a short MOTD — a real
// shell's "Last login…" recolored to rust. Ported from meditate-web's boot.ts +
// motd.ts, restricted to talk's surface (no orb, no audio copy) and made
// pure-ish: it returns tone-tagged lines, so it is unit-tested without a
// terminal and main.ts paints each line through the rust theme (the same
// single-source palette the live edge uses). Reduced-motion is the caller's
// concern (it shortens the dwell); boot only decides the copy.

/** The host (talk.pilgrimapp.org) the login line names — the web front door. */
export const BOOT_HOST = 'talk.pilgrimapp.org';

/** The OS families the install funnel detects, so the funnel shows the command
 *  that actually works on the visitor's machine instead of one canonical line. */
export type InstallOs = 'mac' | 'linux' | 'windows' | 'unknown';

/** The per-OS install command, kept VERBATIM in sync with the README's Install
 *  section so the funnel never drifts from the real tap / crate. macOS leads with
 *  Homebrew (the tap is `walktalkmeditate/tap`); everywhere else uses the crate
 *  (`talk-cli`, installing the `talk` binary), which needs only a Rust toolchain. */
export function installHint(os: InstallOs): string {
  switch (os) {
    case 'mac':
      return 'brew install walktalkmeditate/tap/talk';
    case 'linux':
    case 'windows':
    case 'unknown':
      return 'cargo install talk-cli';
  }
}

/** The default funnel line (macOS Homebrew) used when no OS is supplied — the
 *  pure renderers fall back to it so `boot.test.ts` stays platform-independent. */
export const INSTALL_HINT = installHint('mac');

/** Which rust tone a boot line paints in (main.ts maps these to theme.core /
 *  theme.dim / theme.edge — the same tones the composed session uses). */
export type BootTone = 'core' | 'dim' | 'edge';

/** One rendered boot line: its text and the tone the host paints it in. */
export interface BootLine {
  readonly text: string;
  readonly tone: BootTone;
}

const MIN_SECONDS = 90;

/**
 * A short, friendly elapsed-time phrase for the "last visit" line — the same
 * buckets a returning reflector reads as calm prose ("just now" / "5 minutes
 * ago" / "1 hour ago" / "3 days ago"). Clamped at zero so a backwards clock skew
 * reads "just now" rather than a negative duration.
 */
export function relativeTime(fromMs: number, nowMs: number): string {
  const seconds = Math.max(0, Math.floor((nowMs - fromMs) / 1000));
  if (seconds < MIN_SECONDS) return 'just now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} minutes ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? '' : 's'} ago`;
  const days = Math.floor(hours / 24);
  return `${days} day${days === 1 ? '' : 's'} ago`;
}

/**
 * The boot banner as tone-tagged lines: a login line over the MOTD (wordmark, a
 * rule, a version/local-session stamp, and the soft install funnel). A returning
 * visitor sees when they last spoke here; a first-timer gets a gentle welcome.
 * Pure — no terminal, no theme, no clock of its own — so `boot.test.ts` asserts
 * the copy directly and main.ts owns the painting.
 */
export function renderBoot(
  version: string,
  lastVisit: number | null,
  nowMs: number,
  install: string = INSTALL_HINT,
): BootLine[] {
  const login: BootLine =
    lastVisit !== null
      ? { text: `Last visit: ${relativeTime(lastVisit, nowMs)} on ${BOOT_HOST}`, tone: 'dim' }
      : { text: `Welcome — first reflection on ${BOOT_HOST}`, tone: 'dim' };

  return [login, { text: '', tone: 'edge' }, ...renderMotd(version, install)];
}

/** The MOTD banner: the talk wordmark, a rule, the version/local-session stamp,
 *  and the soft install funnel (the only nudge to the real CLI). `install` is the
 *  OS-detected command the host resolves; it defaults to the macOS Homebrew line. */
export function renderMotd(version: string, install: string = INSTALL_HINT): BootLine[] {
  return [
    { text: 'talk', tone: 'core' },
    { text: '────', tone: 'edge' },
    { text: `v${version} · local session · no account, nothing leaves your browser`, tone: 'dim' },
    { text: `install the CLI: ${install}`, tone: 'edge' },
  ];
}
