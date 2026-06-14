// Shareable reflect questions via the URL hash fragment (#q=<question-id>). The
// hash is never sent to the server, so a deep-link never 404s on GitHub Pages
// (Pages serves a single static index). Human-readable and copy-pasteable.
//
// Ported from meditate-web's deeplink.ts and restricted to talk's ONE shape — a
// question id — but it keeps meditate's sanitization moat verbatim, because the
// security surface is the same and HARSHER here: the hash drives an on-screen
// reflect question, and the page sits right next to mic transcript (an
// attacker-adjacent surface). A hash value is attacker-controlled (a one-click
// link), so a SAFE_ID alphabet gate + a control/ESC-byte neutralizer stand
// between the hash and anything the terminal renders. Real question ids are
// `[a-z0-9-]` (the CLI's `slug.rs` kebab + `questions.rs` ids), so the gate
// rejects everything else BEFORE it can reach xterm as escape bytes.

/** A talk deep-link: an optional reflect question id to open on load. */
export interface DeepLink {
  /** A sanitized, alphabet-valid question id, or undefined when absent/invalid. */
  questionId?: string;
}

/**
 * The safe-id alphabet + length cap. A real question id is lowercase kebab
 * (`[a-z0-9-]`), 1–64 chars. Anything outside this — uppercase, punctuation,
 * percent-encoded ESC/OSC bytes, an over-long blob — fails the test and is
 * dropped entirely, so it can never reach the terminal or drive question lookup.
 */
const SAFE_ID = /^[a-z0-9-]{1,64}$/;

/** Whether a candidate id is in the safe alphabet (exported for the caller's
 *  pre-lookup guard and for the test). */
export function isSafeId(value: string): boolean {
  return SAFE_ID.test(value);
}

/** The hard length cap any echoed fragment is truncated to (defense in depth). */
const MAX_ECHO_LEN = 64;

/** Strip control/ESC bytes from a value WITHOUT changing its length otherwise.
 *  This is the core of the moat: no ESC (`\x1b`), C0/C1 control, or DEL byte
 *  survives, so a junk fragment can never drive the terminal (cursor moves, OSC
 *  title/clipboard sequences, color resets). Length is left intact so the
 *  alphabet gate can REJECT an over-long id rather than silently truncate it.
 *
 *  Exported so the OTHER untrusted-text→terminal seam (the wasm→xterm compose
 *  path in theme.ts) shares the exact same moat — one neutralizer, two surfaces. */
export function stripControl(value: string): string {
  // eslint-disable-next-line no-control-regex
  return value.replace(/[\x00-\x1f\x7f-\x9f]/g, '');
}

/**
 * Strip control/ESC bytes AND cap length — the echo-safe form for any caller
 * that must surface an untrusted fragment back to the screen. Acceptance
 * (`parseHash`) does NOT use the cap (it lets `SAFE_ID` reject an over-long id
 * outright, so a 65-char value is dropped, not truncated into a different —
 * possibly real — id). Exported for the echo path + the test.
 */
export function neutralize(value: string): string {
  return stripControl(value).slice(0, MAX_ECHO_LEN);
}

/**
 * Parse the location hash into a deep-link. Only `#q=<question-id>` is honored;
 * the value has its control bytes stripped, then must pass the `SAFE_ID`
 * alphabet+length gate. Anything that fails — wrong alphabet, over-long, or
 * neutralized to empty — is silently ignored (the caller falls back to normal
 * reflect selection). Never throws on malformed input.
 */
export function parseHash(hash: string): DeepLink {
  const out: DeepLink = {};
  const params = new URLSearchParams(hash.replace(/^#/, ''));
  const raw = params.get('q');
  if (raw !== null) {
    const candidate = stripControl(raw);
    if (isSafeId(candidate)) out.questionId = candidate;
  }
  return out;
}

/** Build a hash fragment for a question id (`#q=<id>`), or `''` for no id. The
 *  id is alphabet-checked so a caller can't smuggle an unsafe value into a URL
 *  it then shares. */
export function buildHash(questionId: string): string {
  if (!isSafeId(questionId)) return '';
  const params = new URLSearchParams();
  params.set('q', questionId);
  return `#${params.toString()}`;
}

/** Compose a full shareable URL for a question id, from the current origin +
 *  path by default. An unsafe id yields the bare base (no fragment). */
export function shareUrl(
  questionId: string,
  base = `${location.origin}${location.pathname}`,
): string {
  return `${base}${buildHash(questionId)}`;
}
