//! The browser façade over `talk-core`.
//!
//! `talk-core` is the pure reflection engine: deterministic cleanup, the
//! journal/reflect append format, the rust palette, the settle live/committing/
//! settled machine, the render-model compose, and curated-question selection. No
//! I/O, no audio, no ML. This crate re-exposes that surface to JS through
//! `wasm-bindgen`, with every boundary input clamped/validated so nothing can
//! panic across the wasm boundary and kill the session.
//!
//! The privacy-critical off-record Commit/Revise *pairing* state machine (lifted
//! from `src/live.rs` into `talk_core::pairing`) is exposed here as `Pairing`, so the
//! web ASR driver shares the CLI's exact drop-while-paused / disarm-paired-Revise /
//! straddling-Revise invariant instead of re-implementing it in TypeScript.

use talk_core::cleanup;
use talk_core::close;
use talk_core::entry::{self, Entry, Mode as EntryMode};
use talk_core::pairing::{Decision, EventKind, Pairing as CorePairing};
use talk_core::palette::{self, Theme, Tone};
use talk_core::questions::Pack;
use talk_core::render_model::{self, LineKind, Mode as ViewMode, View};
use talk_core::selection::{select, SelectionState};
use talk_core::settle::Settle as CoreSettle;
use wasm_bindgen::prelude::*;

/// Shape a whole entry for a cleanup level (the session-end transform applied to
/// the joined clean text). Mirrors `talk_core::cleanup::shape_entry`: High
/// paragraphizes, lower levels pass through. Unknown level strings fall back to
/// Light (the safe default), so the export path can never panic on a bad string.
#[wasm_bindgen(js_name = shapeEntry)]
pub fn shape_entry(level: &str, text: &str) -> String {
    cleanup::shape_entry(cleanup::parse_level(level), text)
}

/// The deterministic per-phrase "Light" cleanup (capitalize, terminate, strip
/// leading non-lexical fillers). Exposed so a clean⇄raw toggle can re-derive the
/// clean text in the browser without re-implementing the rule in TS.
#[wasm_bindgen(js_name = deterministicLight)]
pub fn deterministic_light(text: &str) -> String {
    cleanup::deterministic_light(text)
}

/// Append an entry to a journal/reflect body, returning the new body — CLI-
/// identical markdown so a web export drops cleanly into a real `~/talk` vault.
/// Wraps `talk_core::entry::append`.
///
/// `raw` is the verbatim transcript (`null`/`undefined` → omit the `<!-- raw: -->`
/// comment, as when keep-raw is off or the entry is ephemeral). `mode` is
/// "journal" (time-keyed sections + `---` divider) or "reflect" (date-keyed
/// sections); anything else falls back to journal.
#[wasm_bindgen(js_name = appendEntry)]
pub fn append_entry(
    body: &str,
    date: &str,
    time: &str,
    raw: Option<String>,
    clean: &str,
    mode: &str,
) -> String {
    let entry_mode = match mode.trim().to_ascii_lowercase().as_str() {
        "reflect" => EntryMode::Reflect,
        _ => EntryMode::Journal,
    };
    let entry = Entry {
        date,
        time,
        raw: raw.as_deref(),
        clean,
    };
    entry::append(body, &entry, entry_mode)
}

/// The three palette tones for a theme as 9 bytes — core RGB, dim RGB, edge RGB —
/// for the web renderer. `core` is settled text (brightest), `dim` is the live
/// edge + question, `edge` is chrome.
///
/// The `Mono` theme defers to the terminal's own foreground (it has no fixed RGB),
/// so it returns an EMPTY vector — the JS treats empty as "use the terminal fg".
/// Unknown theme strings fall back to Rust (the default).
///
/// Length contract — the only two lengths ever returned:
///   * empty `Uint8Array` → Mono ("use the terminal foreground")
///   * exactly 9 bytes    → core/dim/edge RGB triples
/// No other length is possible: a color theme's three tones are all
/// `Tone::Color`, and Mono's are all `Tone::Terminal*`, so the push closure
/// emits either 9 bytes or 0. The TS side (`themeTones`) relies on this.
#[wasm_bindgen(js_name = palette)]
pub fn palette_bytes(theme: &str) -> Vec<u8> {
    let theme = Theme::from_str(theme).unwrap_or_default();
    let p = palette::palette(theme);
    let mut out = Vec::with_capacity(9);
    let mut push = |t: Tone| {
        if let Tone::Color(c) = t {
            out.push(c.r);
            out.push(c.g);
            out.push(c.b);
        }
    };
    push(p.core);
    push(p.dim);
    push(p.edge);
    out
}

/// A live settle machine: the live edge, the committing block (still inside its
/// decode-lag / async-swap window), and the immutable settled blocks. JS drives it
/// from the ASR pipeline and reads it back through the renderer.
#[wasm_bindgen]
pub struct Settle {
    inner: CoreSettle,
}

#[wasm_bindgen]
impl Settle {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Settle {
        Settle {
            inner: CoreSettle::new(),
        }
    }

    /// A new/revised partial hypothesis for the live edge.
    #[wasm_bindgen(js_name = onPartial)]
    pub fn on_partial(&mut self, partial: &str) {
        self.inner.on_partial(partial);
    }

    /// Endpoint boundary: the live edge becomes the committing block. Any prior
    /// committing block is finalized into settled first.
    #[wasm_bindgen(js_name = "commit")]
    pub fn commit(&mut self, raw: &str, clean: &str) {
        self.inner.commit(raw, clean);
    }

    /// Promote the committing block to settled (its lag/swap window elapsed).
    #[wasm_bindgen(js_name = "finalize")]
    pub fn finalize(&mut self) {
        self.inner.finalize();
    }

    /// Async swap: replace the committing block's clean text while still inside
    /// its window. Returns false (no-op) once finalized.
    #[wasm_bindgen(js_name = upgradeCommitting)]
    pub fn upgrade_committing(&mut self, clean: &str) -> bool {
        self.inner.upgrade_committing(clean)
    }

    /// Second-pass swap: replace BOTH raw and clean of the committing block (a
    /// better transcription of the same audio). Returns false once finalized.
    #[wasm_bindgen(js_name = reviseCommitting)]
    pub fn revise_committing(&mut self, raw: &str, clean: &str) -> bool {
        self.inner.revise_committing(raw, clean)
    }

    /// The settled blocks' clean text, joined by `\n` (immutable, brightest).
    #[wasm_bindgen(js_name = settledText)]
    pub fn settled_text(&self) -> String {
        join_clean(self.inner.settled())
    }

    /// The settled blocks' raw verbatim text, joined by `\n` (for the raw toggle).
    #[wasm_bindgen(js_name = settledRaw)]
    pub fn settled_raw(&self) -> String {
        self.inner
            .settled()
            .iter()
            .map(|b| b.raw.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The committing block's clean text, or empty if there is none.
    #[wasm_bindgen(js_name = committingText)]
    pub fn committing_text(&self) -> String {
        self.inner
            .committing()
            .map(|b| b.clean.clone())
            .unwrap_or_default()
    }

    /// The committing block's raw verbatim text, or empty if there is none.
    #[wasm_bindgen(js_name = committingRaw)]
    pub fn committing_raw(&self) -> String {
        self.inner
            .committing()
            .map(|b| b.raw.clone())
            .unwrap_or_default()
    }

    /// Whether there is a committing block at all.
    #[wasm_bindgen(js_name = hasCommitting)]
    pub fn has_committing(&self) -> bool {
        self.inner.committing().is_some()
    }

    /// True once the committing block has been revised/upgraded (final-quality):
    /// drives bright-is-final rendering — dim until revised.
    #[wasm_bindgen(js_name = committingRevised)]
    pub fn committing_revised(&self) -> bool {
        self.inner.committing_revised()
    }

    /// The live edge text (the jittering partial hypothesis).
    #[wasm_bindgen(js_name = edgeText)]
    pub fn edge_text(&self) -> String {
        self.inner.live().to_string()
    }

    /// Compose the full screen as a JSON array of `{ "text": string, "kind": string }`
    /// (top to bottom), where `kind` is one of "chrome" | "settled" | "edge" |
    /// "question". Wraps `talk_core::render_model::compose` over this settle's state
    /// plus the scalar view fields JS supplies.
    ///
    /// `mode` is "reflect" | "journal" | "ephemeral" (anything else → reflect).
    /// `question`/`held_label` are passed as empty strings to mean "none".
    #[wasm_bindgen(js_name = compose)]
    #[allow(clippy::too_many_arguments)]
    pub fn compose(
        &self,
        mode: &str,
        question: &str,
        held_label: &str,
        listening: bool,
        elapsed: &str,
        cleanup: &str,
        show_raw: bool,
        paused: bool,
        confirm_cancel: bool,
    ) -> String {
        let view_mode = match mode.trim().to_ascii_lowercase().as_str() {
            "journal" => ViewMode::Journal,
            "ephemeral" => ViewMode::Ephemeral,
            _ => ViewMode::Reflect,
        };
        let view = View {
            mode: view_mode,
            question: none_if_empty(question),
            held_label: none_if_empty(held_label),
            settle: &self.inner,
            listening,
            elapsed,
            cleanup,
            show_raw,
            paused,
            confirm_cancel,
        };
        let lines = render_model::compose(&view);
        lines_to_json(&lines)
    }
}

impl Default for Settle {
    fn default() -> Self {
        Self::new()
    }
}

/// The off-record Commit/Revise/pause pairing machine — the privacy-critical guard
/// that keeps off-record (paused) speech out of a kept entry, shared verbatim with
/// the CLI (`talk_core::pairing`). The JS ASR driver feeds it event KINDS and pause/
/// resume edges; it returns a DECISION string the driver acts on against its `Settle`.
///
/// The machine deliberately never sees the transcript text — the driver runs its own
/// cleanup over the payload only when `decide` returns "applyCommit"/"applyRevise"/
/// "applyPartial". This is exactly how `src/live.rs::apply_event` calls it.
#[wasm_bindgen]
pub struct Pairing {
    inner: CorePairing,
}

#[wasm_bindgen]
impl Pairing {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Pairing {
        Pairing {
            inner: CorePairing::new(),
        }
    }

    /// Enter off-record. The driver still clears the live edge and tells its audio
    /// path to go off-record — those are not modeled here.
    #[wasm_bindgen(js_name = "pause")]
    pub fn pause(&mut self) {
        self.inner.pause();
    }

    /// Leave off-record. Does NOT re-arm the pairing guard (an off-record Revise can
    /// still be in flight) — only the next accepted Commit re-arms it.
    #[wasm_bindgen(js_name = "resume")]
    pub fn resume(&mut self) {
        self.inner.resume();
    }

    /// Lift the pause for the finish-drain while carrying the pairing guard forward,
    /// so an in-flight Revise of a pause-dropped Commit is still dropped at finish.
    #[wasm_bindgen(js_name = beginFinishDrain)]
    pub fn begin_finish_drain(&mut self) {
        self.inner.begin_finish_drain();
    }

    /// Whether the machine is currently off-record (paused).
    #[wasm_bindgen(js_name = isPaused)]
    pub fn is_paused(&self) -> bool {
        self.inner.is_paused()
    }

    /// Whether the next pass-2 Revise will be dropped (its paired Commit was an
    /// off-record drop). Exposed for driver-side diagnostics/mirroring.
    #[wasm_bindgen(js_name = commitDropped)]
    pub fn commit_dropped(&self) -> bool {
        self.inner.commit_dropped()
    }

    /// Decide what to do with an event of the given KIND, mutating the pairing guard.
    ///
    /// `kind` is "partial" | "commit" | "revise" | "done". An UNKNOWN kind returns
    /// "drop" — the privacy-safe default: an unrecognized event must NEVER be
    /// applied (which could land off-record text), and dropping it leaves the
    /// pairing guard's invariant untouched. The boundary never throws. Returns one
    /// of "done" | "drop" | "applyPartial" | "applyCommit" | "applyRevise".
    #[wasm_bindgen(js_name = "decide")]
    pub fn decide(&mut self, kind: &str) -> String {
        let event = match kind.trim().to_ascii_lowercase().as_str() {
            "partial" => EventKind::Partial,
            "commit" => EventKind::Commit,
            "revise" => EventKind::Revise,
            "done" => EventKind::Done,
            // An UNKNOWN kind is dropped — never applied (privacy-safe default).
            _ => return "drop".to_string(),
        };
        match self.inner.decide(event) {
            Decision::Done => "done",
            Decision::Drop => "drop",
            Decision::ApplyPartial => "applyPartial",
            Decision::ApplyCommit => "applyCommit",
            Decision::ApplyRevise => "applyRevise",
        }
        .to_string()
    }
}

impl Default for Pairing {
    fn default() -> Self {
        Self::new()
    }
}

/// The closing screen after `[space]` in reflect/journal, as a JSON string array.
/// Wraps `talk_core::render_model::compose_close`.
#[wasm_bindgen(js_name = composeClose)]
pub fn compose_close(path: &str, provenance: &str, phrase: &str) -> String {
    json_string_array(&render_model::compose_close(path, provenance, phrase))
}

/// The ephemeral release screen, as a JSON string array. Wraps
/// `talk_core::render_model::compose_released`.
#[wasm_bindgen(js_name = composeReleased)]
pub fn compose_released() -> String {
    json_string_array(&render_model::compose_released())
}

/// Pick a curated close phrase by `seed`, rotating over the shared
/// `talk_core::close::CLOSE_PHRASES` list — the single source of truth the CLI
/// uses too, so the web can never drift from it. The seed is reduced modulo the
/// list length; a negative seed is clamped to 0 at the boundary.
#[wasm_bindgen(js_name = selectClosePhrase)]
pub fn select_close_phrase(seed: f64) -> String {
    let seed = if seed.is_finite() && seed >= 0.0 {
        (seed as u64 % close::CLOSE_PHRASES.len() as u64) as usize
    } else {
        0
    };
    close::select_close_phrase(seed).to_string()
}

/// Select the next curated question from a TOML pack, returning a JSON object
/// `{ "id", "text", "slug", "addressee", "cadence", "slot" }` for the chosen
/// question, or `null` when the pack is empty or fails to parse — the boundary
/// never throws.
///
/// `served_ids` / `served_counts` are parallel arrays (the per-id served counts);
/// `recent_ids` / `recent_ordinals` are parallel arrays (the per-id last-served
/// ordinals, higher = more recent). `held_id`/`held_done` describe an in-progress
/// held run (`held_id` empty = none). `hour` is clamped to 0..=23.
///
/// Precondition: each parallel pair must be the same length —
/// `served_ids.len() == served_counts.len()` and
/// `recent_ids.len() == recent_ordinals.len()`. A mismatch returns `None`
/// (→ JS `null`) rather than zipping, which would silently truncate to the
/// shorter array and corrupt the selection state.
#[wasm_bindgen(js_name = selectQuestion)]
#[allow(clippy::too_many_arguments)]
pub fn select_question(
    pack_toml: &str,
    served_ids: Vec<String>,
    served_counts: Vec<u32>,
    recent_ids: Vec<String>,
    recent_ordinals: Vec<f64>,
    held_id: &str,
    held_done: u32,
    hour: u32,
) -> Option<String> {
    if served_ids.len() != served_counts.len() || recent_ids.len() != recent_ordinals.len() {
        return None;
    }
    let pack = Pack::from_toml(pack_toml).ok()?;
    let mut state = SelectionState::default();
    for (id, count) in served_ids.into_iter().zip(served_counts) {
        state.served_count.insert(id, count);
    }
    for (id, ord) in recent_ids.into_iter().zip(recent_ordinals) {
        // f64 carries the JS number safely; clamp to a finite, non-negative u64.
        let ord = if ord.is_finite() {
            ord.clamp(0.0, u64::MAX as f64) as u64
        } else {
            0
        };
        state.last_served.insert(id, ord);
    }
    if !held_id.is_empty() {
        state.held_run = Some((held_id.to_string(), held_done));
    }
    let hour = hour.min(23);
    let q = select(&pack, &state, hour)?;
    Some(question_to_json(
        &q.id,
        &q.text,
        q.slug.as_deref(),
        &q.addressee,
        &q.cadence,
        q.slot.as_deref(),
    ))
}

fn none_if_empty(s: &str) -> Option<&str> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn join_clean(blocks: &[talk_core::settle::Block]) -> String {
    blocks
        .iter()
        .map(|b| b.clean.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

fn line_kind_str(kind: LineKind) -> &'static str {
    match kind {
        LineKind::Chrome => "chrome",
        LineKind::Settled => "settled",
        LineKind::Edge => "edge",
        LineKind::Question => "question",
    }
}

/// Hand-built JSON so the façade needs no serde_json dep (keeps the wasm crate's
/// dependency surface to talk-core + wasm-bindgen only).
fn lines_to_json(lines: &[(String, LineKind)]) -> String {
    let mut out = String::from("[");
    for (i, (text, kind)) in lines.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"text\":");
        push_json_string(&mut out, text);
        out.push_str(",\"kind\":\"");
        out.push_str(line_kind_str(*kind));
        out.push_str("\"}");
    }
    out.push(']');
    out
}

fn json_string_array(items: &[String]) -> String {
    let mut out = String::from("[");
    for (i, s) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_json_string(&mut out, s);
    }
    out.push(']');
    out
}

fn question_to_json(
    id: &str,
    text: &str,
    slug: Option<&str>,
    addressee: &str,
    cadence: &str,
    slot: Option<&str>,
) -> String {
    let mut out = String::from("{\"id\":");
    push_json_string(&mut out, id);
    out.push_str(",\"text\":");
    push_json_string(&mut out, text);
    out.push_str(",\"slug\":");
    push_json_opt(&mut out, slug);
    out.push_str(",\"addressee\":");
    push_json_string(&mut out, addressee);
    out.push_str(",\"cadence\":");
    push_json_string(&mut out, cadence);
    out.push_str(",\"slot\":");
    push_json_opt(&mut out, slot);
    out.push('}');
    out
}

fn push_json_opt(out: &mut String, value: Option<&str>) {
    match value {
        Some(s) => push_json_string(out, s),
        None => out.push_str("null"),
    }
}

/// Append a JSON-escaped string literal (RFC 8259) to `out`.
fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_entry_high_matches_core_paragraphize() {
        let multi =
            "I woke up early. I made coffee. I read a book. Anyway, then I went for a walk. It was nice. The sun was out.";
        assert_eq!(
            shape_entry("high", multi),
            talk_core::cleanup::paragraphize(multi),
            "the façade must paragraphize identically to talk-core at High"
        );
        // Below High, shape_entry passes the text through unchanged.
        assert_eq!(shape_entry("medium", multi), multi);
        assert_eq!(shape_entry("light", multi), multi);
    }

    #[test]
    fn shape_entry_tolerates_empty_and_whitespace_without_panicking() {
        assert_eq!(shape_entry("high", ""), "");
        assert_eq!(shape_entry("high", "   \n\n  "), "");
        assert_eq!(shape_entry("not-a-level", ""), "");
    }

    #[test]
    fn deterministic_light_matches_core() {
        assert_eq!(deterministic_light("um the thing is"), "The thing is.");
        assert_eq!(deterministic_light(""), "");
    }

    #[test]
    fn journal_append_emits_the_divider_on_the_second_same_day_entry() {
        let body = append_entry("", "2026-06-08", "08:14", None, "Morning.", "journal");
        assert!(body.contains("## 08:14"));
        assert!(
            !body.contains("---"),
            "the first entry of the day has no leading divider"
        );

        let body2 = append_entry(&body, "2026-06-08", "21:30", None, "Night.", "journal");
        assert!(
            body2.contains("Morning.\n\n---\n\n## 21:30"),
            "the second same-day journal entry must be preceded by the `---` divider: {body2:?}"
        );
    }

    #[test]
    fn append_renders_the_raw_comment_when_present() {
        let body = append_entry(
            "",
            "2026-06-06",
            "08:14",
            Some("um the thing is".to_string()),
            "The thing is.",
            "reflect",
        );
        assert!(body.contains("## 2026-06-06"));
        assert!(body.contains("<!-- raw: um the thing is -->"));
        assert!(body.contains("The thing is."));
    }

    #[test]
    fn palette_rust_returns_the_expected_nine_bytes() {
        // The rust palette triple from talk_core::palette::palette(Theme::Rust).
        assert_eq!(
            palette_bytes("rust"),
            vec![210, 146, 118, 170, 124, 104, 150, 122, 112]
        );
    }

    #[test]
    fn palette_unknown_theme_falls_back_to_rust() {
        assert_eq!(palette_bytes("not-a-theme"), palette_bytes("rust"));
    }

    #[test]
    fn palette_mono_returns_empty_for_terminal_foreground() {
        assert!(
            palette_bytes("mono").is_empty(),
            "Mono defers to the terminal fg — the JS sentinel is an empty byte vector"
        );
    }

    #[test]
    fn settle_round_trips_through_the_facade() {
        let mut s = Settle::new();
        s.on_partial("um the thing");
        assert_eq!(s.edge_text(), "um the thing");
        s.commit("um the thing is", "The thing is.");
        assert_eq!(s.edge_text(), "");
        assert!(s.has_committing());
        assert_eq!(s.committing_text(), "The thing is.");
        assert!(!s.committing_revised());
        assert!(s.revise_committing("better raw", "Better raw."));
        assert!(s.committing_revised());
        assert_eq!(s.committing_text(), "Better raw.");
        assert_eq!(s.committing_raw(), "better raw");
        s.finalize();
        assert_eq!(s.settled_text(), "Better raw.");
        assert_eq!(s.settled_raw(), "better raw");
        assert!(!s.has_committing());
    }

    #[test]
    fn compose_emits_json_with_kinds() {
        let mut s = Settle::new();
        s.commit("um the raw words", "The clean words.");
        s.finalize();
        let json = s.compose(
            "reflect",
            "What am I avoiding?",
            "held 3 days",
            false,
            "2:14",
            "Light",
            false,
            false,
            false,
        );
        assert!(json.starts_with('['));
        assert!(json.contains("\"kind\":\"question\""));
        assert!(json.contains("What am I avoiding?"));
        assert!(json.contains("The clean words."));
        assert!(json.contains("[space] done"));
    }

    #[test]
    fn compose_escapes_control_and_quote_characters() {
        let mut s = Settle::new();
        // A committing block whose clean text contains a quote — must be escaped.
        s.commit("raw", "He said \"hello\".");
        let json = s.compose(
            "journal", "", "", false, "0:01", "Medium", false, false, false,
        );
        assert!(
            json.contains("He said \\\"hello\\\"."),
            "quotes inside line text must be JSON-escaped: {json}"
        );
    }

    /// Drive every `push_json_string` escape arm through `compose()`: the committing
    /// block's clean text carries each special char, and the emitted JSON must
    /// contain the RFC 8259 escape and still parse as valid JSON.
    #[test]
    fn push_json_string_escapes_every_special_char() {
        let cases: &[(&str, &str)] = &[
            ("a\"b", "a\\\"b"),       // quote → \"
            ("a\\b", "a\\\\b"),       // backslash → \\
            ("a\nb", "a\\nb"),        // newline → \n
            ("a\rb", "a\\rb"),        // carriage return → \r
            ("a\tb", "a\\tb"),        // tab → \t
            ("a\u{01}b", "a\\u0001b"), // control char → 
        ];
        for (input, expected_escape) in cases {
            let mut s = Settle::new();
            s.commit("raw", input);
            let json = s.compose(
                "journal", "", "", false, "0:01", "Light", false, false, false,
            );
            assert!(
                json.contains(expected_escape),
                "compose output must contain the escape {expected_escape:?} for input {input:?}: {json}"
            );
        }
    }

    /// `push_json_string` is reachable directly from tests in the same crate; verify
    /// each arm in isolation so a regression is localized to the escaper.
    #[test]
    fn push_json_string_escapes_in_isolation() {
        let mut out = String::new();
        push_json_string(&mut out, "\"\\\n\r\t\u{01}");
        assert_eq!(out, "\"\\\"\\\\\\n\\r\\t\\u0001\"");
    }

    #[test]
    fn upgrade_committing_swaps_clean_keeps_raw_and_marks_revised() {
        let mut s = Settle::new();
        s.commit("the original raw", "The original.");
        assert!(!s.committing_revised());
        assert!(s.upgrade_committing("The upgraded."));
        assert_eq!(s.committing_text(), "The upgraded.");
        assert_eq!(
            s.committing_raw(),
            "the original raw",
            "upgrade swaps clean only — raw is preserved"
        );
        assert!(s.committing_revised());
    }

    #[test]
    fn post_finalize_revise_and_upgrade_return_false_and_leave_settled_unchanged() {
        let mut s = Settle::new();
        s.commit("the raw", "The clean.");
        s.finalize();
        let settled_before = s.settled_text();
        assert_eq!(settled_before, "The clean.");
        // After finalize there is no committing block — both swaps are no-ops.
        assert!(!s.upgrade_committing("ignored upgrade"));
        assert!(!s.revise_committing("ignored raw", "ignored clean"));
        assert_eq!(
            s.settled_text(),
            settled_before,
            "a post-finalize swap must not mutate settled text"
        );
        assert!(!s.has_committing());
    }

    #[test]
    fn palette_high_contrast_returns_the_expected_nine_bytes() {
        // The high-contrast palette triple from palette::palette(Theme::HighContrast).
        assert_eq!(
            palette_bytes("high-contrast"),
            vec![236, 205, 186, 198, 152, 124, 176, 142, 126]
        );
    }

    /// Round-trip: both hand-built JSON emitters (`compose` and `select_question`)
    /// must produce output that parses as valid JSON. Uses a permissive recursive
    /// parser since the wasm crate carries no serde_json dependency.
    #[test]
    fn compose_and_select_question_emit_valid_json() {
        let mut s = Settle::new();
        s.commit("um the raw", "He said \"hi\".\nNew line.\tTabbed.");
        s.finalize();
        let compose_json = s.compose(
            "reflect",
            "What's \"true\" now?",
            "held 2 days",
            true,
            "1:23",
            "High",
            false,
            false,
            false,
        );
        assert!(
            is_valid_json(&compose_json),
            "compose output must be valid JSON: {compose_json}"
        );

        let toml = r#"
            name = "t"
            [[questions]]
            id = "a"
            text = "A \"quoted\" question?"
        "#;
        let select_json =
            select_question(toml, vec![], vec![], vec![], vec![], "", 0, 9).expect("a question");
        assert!(
            is_valid_json(&select_json),
            "select_question output must be valid JSON: {select_json}"
        );
    }

    #[test]
    fn select_question_returns_none_on_mismatched_parallel_arrays() {
        let toml = r#"
            name = "t"
            [[questions]]
            id = "a"
            text = "A?"
        "#;
        // served_ids longer than served_counts → None, never a truncated zip.
        assert!(select_question(
            toml,
            vec!["a".to_string(), "b".to_string()],
            vec![1],
            vec![],
            vec![],
            "",
            0,
            9
        )
        .is_none());
        // recent_ids shorter than recent_ordinals → None.
        assert!(select_question(
            toml,
            vec![],
            vec![],
            vec!["a".to_string()],
            vec![1.0, 2.0],
            "",
            0,
            9
        )
        .is_none());
    }

    #[test]
    fn select_question_returns_the_chosen_question_json() {
        let toml = r#"
            name = "t"
            [[questions]]
            id = "a"
            text = "A?"
            slot = "morning"
            [[questions]]
            id = "b"
            text = "B?"
            slot = "evening"
        "#;
        // Morning hour prefers the morning-slotted question.
        let json = select_question(
            toml,
            vec![],
            vec![],
            vec![],
            vec![],
            "",
            0,
            7,
        )
        .expect("a question is selected");
        assert!(json.contains("\"id\":\"a\""));
        assert!(json.contains("\"text\":\"A?\""));
    }

    #[test]
    fn select_question_honors_a_held_run() {
        let toml = r#"
            name = "t"
            [[questions]]
            id = "a"
            text = "A?"
            [[questions]]
            id = "h"
            text = "Held?"
            cadence = "held:7"
        "#;
        let json = select_question(toml, vec![], vec![], vec![], vec![], "h", 3, 12)
            .expect("held run wins");
        assert!(json.contains("\"id\":\"h\""));
    }

    #[test]
    fn select_question_returns_none_on_bad_toml() {
        assert!(select_question("not = [valid", vec![], vec![], vec![], vec![], "", 0, 9).is_none());
    }

    #[test]
    fn select_question_tolerates_out_of_range_hour_and_non_finite_ordinals() {
        let toml = r#"
            name = "t"
            [[questions]]
            id = "a"
            text = "A?"
        "#;
        // hour 99 clamps to 23; an infinite ordinal clamps to 0 — neither panics.
        let json = select_question(
            toml,
            vec![],
            vec![],
            vec!["a".to_string()],
            vec![f64::INFINITY],
            "",
            0,
            99,
        )
        .expect("a question is still selected");
        assert!(json.contains("\"id\":\"a\""));
    }

    #[test]
    fn pairing_drops_a_paused_commit_and_its_paired_revise_even_after_resume() {
        let mut p = Pairing::new();
        assert_eq!(p.decide("commit"), "applyCommit"); // kept, on-record
        p.pause();
        assert_eq!(p.decide("commit"), "drop"); // OFF-RECORD
        assert!(p.commit_dropped(), "a paused Commit must disarm its paired Revise");
        p.resume(); // resume does NOT re-arm
        assert_eq!(
            p.decide("revise"),
            "drop",
            "the off-record Revise must drop, never overwrite the kept phrase"
        );
    }

    #[test]
    fn pairing_lets_a_straddling_revise_upgrade_while_paused() {
        let mut p = Pairing::new();
        assert_eq!(p.decide("commit"), "applyCommit");
        p.pause();
        assert_eq!(
            p.decide("revise"),
            "applyRevise",
            "a Revise of an on-record Commit upgrades even while paused"
        );
    }

    #[test]
    fn pairing_rearms_on_the_next_accepted_commit_not_on_resume() {
        let mut p = Pairing::new();
        p.pause();
        assert_eq!(p.decide("commit"), "drop"); // disarms
        p.resume();
        assert!(p.commit_dropped(), "resume alone must NOT re-arm");
        assert_eq!(p.decide("commit"), "applyCommit"); // re-arms
        assert!(!p.commit_dropped());
        assert_eq!(p.decide("revise"), "applyRevise");
    }

    #[test]
    fn pairing_finish_drain_still_drops_the_paused_pairs_revise() {
        let mut p = Pairing::new();
        p.pause();
        assert_eq!(p.decide("commit"), "drop"); // OFF-RECORD
        p.begin_finish_drain(); // lifts paused, carries commit_dropped
        assert!(!p.is_paused());
        assert_eq!(
            p.decide("revise"),
            "drop",
            "the drain must not let the paused pair's Revise land"
        );
        assert_eq!(p.decide("done"), "done");
    }

    #[test]
    fn pairing_drops_a_paused_partial_and_finishes_on_done_while_paused() {
        let mut p = Pairing::new();
        p.pause();
        assert_eq!(p.decide("partial"), "drop");
        assert_eq!(p.decide("done"), "done");
    }

    #[test]
    fn pairing_unknown_kind_drops_without_panicking() {
        // The privacy-safe default: an UNKNOWN kind must DROP, never apply — an
        // applied unknown could land off-record text into a kept entry.
        let mut p = Pairing::new();
        assert_eq!(p.decide("nonsense"), "drop");
        assert_eq!(p.decide(""), "drop");
    }

    #[test]
    fn compose_close_and_released_emit_json_arrays() {
        let close = compose_close("~/talk/x.md", "entry 3", "Let it settle.");
        assert!(close.contains("~/talk/x.md") && close.contains("Let it settle."));
        assert_eq!(compose_released(), "[\"Released. Nothing was written.\"]");
    }

    #[test]
    fn select_close_phrase_rotates_over_the_shared_list() {
        let n = close::CLOSE_PHRASES.len();
        // seed 0 → first phrase; wrapping past the end returns to the start.
        assert_eq!(select_close_phrase(0.0), close::CLOSE_PHRASES[0]);
        assert_eq!(select_close_phrase(n as f64), close::CLOSE_PHRASES[0]);
        assert_eq!(select_close_phrase((n + 3) as f64), close::CLOSE_PHRASES[3]);
        // A non-finite / negative seed clamps to the first phrase, never panics.
        assert_eq!(select_close_phrase(f64::NAN), close::CLOSE_PHRASES[0]);
        assert_eq!(select_close_phrase(-1.0), close::CLOSE_PHRASES[0]);
    }

    /// A tiny recursive-descent JSON validator — enough to prove the hand-built
    /// emitters produce well-formed JSON without pulling serde_json into the wasm
    /// crate's dependency surface.
    fn is_valid_json(s: &str) -> bool {
        let mut chars = s.chars().peekable();
        let ok = parse_value(&mut chars);
        skip_ws(&mut chars);
        ok && chars.next().is_none()
    }

    fn skip_ws(chars: &mut std::iter::Peekable<std::str::Chars>) {
        while matches!(chars.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            chars.next();
        }
    }

    fn parse_value(chars: &mut std::iter::Peekable<std::str::Chars>) -> bool {
        skip_ws(chars);
        match chars.peek() {
            Some('{') => parse_object(chars),
            Some('[') => parse_array(chars),
            Some('"') => parse_string(chars),
            Some('t') => consume_literal(chars, "true"),
            Some('f') => consume_literal(chars, "false"),
            Some('n') => consume_literal(chars, "null"),
            Some(c) if *c == '-' || c.is_ascii_digit() => parse_number(chars),
            _ => false,
        }
    }

    fn consume_literal(chars: &mut std::iter::Peekable<std::str::Chars>, lit: &str) -> bool {
        for expected in lit.chars() {
            if chars.next() != Some(expected) {
                return false;
            }
        }
        true
    }

    fn parse_string(chars: &mut std::iter::Peekable<std::str::Chars>) -> bool {
        if chars.next() != Some('"') {
            return false;
        }
        while let Some(c) = chars.next() {
            match c {
                '"' => return true,
                '\\' => match chars.next() {
                    Some('"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't') => {}
                    Some('u') => {
                        for _ in 0..4 {
                            if !matches!(chars.next(), Some(h) if h.is_ascii_hexdigit()) {
                                return false;
                            }
                        }
                    }
                    _ => return false,
                },
                c if (c as u32) < 0x20 => return false, // unescaped control char
                _ => {}
            }
        }
        false
    }

    fn parse_number(chars: &mut std::iter::Peekable<std::str::Chars>) -> bool {
        let mut saw_digit = false;
        if chars.peek() == Some(&'-') {
            chars.next();
        }
        while matches!(chars.peek(), Some(c) if c.is_ascii_digit() || *c == '.' || *c == 'e' || *c == 'E' || *c == '+' || *c == '-')
        {
            if chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                saw_digit = true;
            }
            chars.next();
        }
        saw_digit
    }

    fn parse_array(chars: &mut std::iter::Peekable<std::str::Chars>) -> bool {
        if chars.next() != Some('[') {
            return false;
        }
        skip_ws(chars);
        if chars.peek() == Some(&']') {
            chars.next();
            return true;
        }
        loop {
            if !parse_value(chars) {
                return false;
            }
            skip_ws(chars);
            match chars.next() {
                Some(',') => continue,
                Some(']') => return true,
                _ => return false,
            }
        }
    }

    fn parse_object(chars: &mut std::iter::Peekable<std::str::Chars>) -> bool {
        if chars.next() != Some('{') {
            return false;
        }
        skip_ws(chars);
        if chars.peek() == Some(&'}') {
            chars.next();
            return true;
        }
        loop {
            skip_ws(chars);
            if !parse_string(chars) {
                return false;
            }
            skip_ws(chars);
            if chars.next() != Some(':') {
                return false;
            }
            if !parse_value(chars) {
                return false;
            }
            skip_ws(chars);
            match chars.next() {
                Some(',') => continue,
                Some('}') => return true,
                _ => return false,
            }
        }
    }
}
