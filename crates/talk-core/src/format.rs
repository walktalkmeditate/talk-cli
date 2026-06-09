//! The formatter seam (pure). `talk-core` owns the restraint POLICY: the
//! `Formatter` contract, the always-safe deterministic fallback, and the
//! diff-guarded call site. The real Candle 0.5B inference lives in the binary's
//! `src/format/` and implements this same trait (Plan 3 T7).

use crate::cleanup::{apply_backtrack, apply_spoken_commands, deterministic_light, guard_accepts, Level};

/// Turn one phrase into cleaned text at a given level. Implementors do ONLY their
/// transform — the deterministic pre-layer and the diff-guard are applied by
/// `guarded_format`, never here. (So a formatter receives already-pre-processed
/// text and must not re-apply spoken commands / backtrack.)
pub trait Formatter {
    fn format(&self, level: Level, text: &str) -> String;
}

/// The always-safe formatter: deterministic-Light, no model. Guard-safe by
/// construction (caps / punctuation / leading-filler only).
pub struct DeterministicFormatter;

impl Formatter for DeterministicFormatter {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Restraint incarnate — deterministic-Light never substitutes a word.
    struct Faithful;
    impl Formatter for Faithful {
        fn format(&self, _l: Level, text: &str) -> String { deterministic_light(text) }
    }

    /// Substitutes meaning words / drops negations — exactly what the guard rejects.
    struct OverEditing;
    impl Formatter for OverEditing {
        fn format(&self, _l: Level, text: &str) -> String {
            format!(" {} ", text).replace(" love ", " hate ").replace(" not ", " ").trim().to_string()
        }
    }

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
        assert_eq!(guarded_format(&Faithful, Level::Light, "um so i keep avoiding it"), "Keep avoiding it.");
    }
}
