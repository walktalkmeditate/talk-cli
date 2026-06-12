//! The formatter seam (pure). `talk-core` owns the restraint POLICY: the
//! `Formatter` contract, the always-safe deterministic fallback, and the
//! diff-guarded call site. The real Candle 0.5B inference lives in the binary's
//! `src/format/` and implements this same trait (Plan 3 T7).

use crate::cleanup::{apply_backtrack, apply_spoken_commands, deterministic_light, guard_accepts, Level};

/// Turn one phrase into cleaned text at a given level. Implementors do ONLY their
/// transform — the deterministic pre-layer and the diff-guard are applied by
/// `guarded_format`, never here. (So a formatter receives already-pre-processed
/// text and must not re-apply spoken commands / backtrack.)
///
/// May receive a single phrase (guarded_format) or a whole document (guarded_document); implementors must be whole-text-safe.
pub trait Formatter {
    fn format(&self, level: Level, text: &str) -> String;
}

/// The always-safe formatter: deterministic-Light, no model. Guard-safe by
/// construction (caps / punctuation / leading-filler only).
pub struct DeterministicFormatter;

impl Formatter for DeterministicFormatter {
    /// `level` is intentionally ignored: with no model, every level collapses to
    /// deterministic-Light (Medium/High word-removal would be rejected by the guard
    /// anyway). This is the always-present fallback.
    fn format(&self, _level: Level, text: &str) -> String {
        deterministic_light(text)
    }
}

/// The moat. Pre-layer → format → accept iff the content-word guard passes, else
/// deterministic-Light. `None` short-circuits to the pre-processed text (no
/// formatting). The result is ALWAYS guard-safe relative to the pre-processed
/// phrase — fail-safe is always your words.
pub fn guarded_format(f: &dyn Formatter, level: Level, raw: &str) -> String {
    let pre = apply_backtrack(&apply_spoken_commands(raw));
    if level == Level::None {
        return pre;
    }
    let candidate = f.format(level, &pre);
    if guard_accepts(&pre, &candidate) {
        candidate
    } else {
        deterministic_light(&pre)
    }
}

/// The whole-document moat (Medium/High). `full_text` is the caller's already-computed
/// Light join; it is returned unchanged on every non-accept path (Light/None level,
/// empty input, or guard rejection) — so the worst case is byte-identical to today's
/// Light output. Only invoked with a model-backed formatter; `DeterministicFormatter`
/// never flows through here (the caller skips the pass when no model is present).
pub fn guarded_document(level: Level, full_text: &str, f: &dyn Formatter) -> String {
    let fallback = full_text.to_string();
    if matches!(level, Level::None | Level::Light) || full_text.trim().is_empty() {
        return fallback;
    }
    let candidate = crate::cleanup::strip_model_preamble(&f.format(level, full_text));
    if crate::cleanup::guard_accepts_deletions(full_text, &candidate) {
        candidate
    } else {
        fallback
    }
}

#[cfg(test)]
mod doc_tests {
    use super::*;

    struct Paragrapher;
    impl Formatter for Paragrapher {
        fn format(&self, _l: Level, text: &str) -> String { text.replace(". ", ".\n\n") }
    }
    struct Substitutor;
    impl Formatter for Substitutor {
        fn format(&self, _l: Level, text: &str) -> String { text.replace("love", "hate") }
    }
    struct NegationDropper;
    impl Formatter for NegationDropper {
        fn format(&self, _l: Level, text: &str) -> String { text.replace(" not", "") }
    }
    struct Prefacer;
    impl Formatter for Prefacer {
        fn format(&self, _l: Level, text: &str) -> String { format!("Sure, here:\n{text}") }
    }

    #[test]
    fn paragraph_reflow_accepted_at_high() {
        let out = guarded_document(Level::High, "One thing. Two thing.", &Paragrapher);
        assert!(out.contains("\n\n") && out.contains("One thing") && out.contains("Two thing"));
    }
    #[test]
    fn substitution_falls_back_to_light_join() {
        assert_eq!(guarded_document(Level::Medium, "i love her", &Substitutor), "i love her");
    }
    #[test]
    fn negation_drop_falls_back() {
        assert_eq!(guarded_document(Level::Medium, "i am not sure", &NegationDropper), "i am not sure");
    }
    #[test]
    fn empty_input_short_circuits() {
        assert_eq!(guarded_document(Level::High, "   ", &Substitutor), "   ");
    }
    #[test]
    fn light_and_none_never_invoke_the_formatter() {
        assert_eq!(guarded_document(Level::Light, "i love her", &Substitutor), "i love her");
        assert_eq!(guarded_document(Level::None, "i love her", &Substitutor), "i love her");
    }
    #[test]
    fn preface_is_stripped_then_accepted() {
        assert_eq!(guarded_document(Level::Medium, "the real thing", &Prefacer), "the real thing");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Faithful, OverEditing};

    #[test]
    fn none_level_returns_pre_layer_unchanged() {
        assert_eq!(guarded_format(&Faithful, Level::None, "i am not done"), "i am not done");
    }

    #[test]
    fn pre_layer_runs_before_formatting() {
        let out = guarded_format(&Faithful, Level::Light, "the answer is yes scratch that the answer is no");
        assert!(!out.contains("yes"));
        assert!(out.contains("answer is no"));
    }

    #[test]
    fn guard_rejects_a_meaning_substitution_and_falls_back() {
        let out = guarded_format(&OverEditing, Level::Light, "i love her");
        assert!(!out.contains("hate"));
        assert!(out.to_lowercase().contains("love"));
    }

    #[test]
    fn guard_rejects_a_dropped_negation_and_falls_back() {
        let out = guarded_format(&OverEditing, Level::Light, "i am not angry");
        assert!(out.to_lowercase().contains("not"));
    }

    #[test]
    fn faithful_output_passes_the_guard_unchanged() {
        assert_eq!(guarded_format(&Faithful, Level::Light, "um so i keep avoiding it"), "So I keep avoiding it.");
    }

    #[test]
    fn guard_fires_at_medium_too() {
        // Medium/High would remove words, so any LLM rewrite there is rejected and
        // falls back to deterministic-Light. The guard is level-agnostic.
        let out = guarded_format(&OverEditing, Level::Medium, "i love her");
        assert!(!out.contains("hate"));
        assert!(out.to_lowercase().contains("love"));
    }
}
