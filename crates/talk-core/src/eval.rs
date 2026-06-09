//! The checked-in restraint eval set (pure). Makes "the formatter never changes
//! meaning" a FALSIFIABLE gate, not a vibe: each fixture lists impermissible
//! substrings (meaning-changing edits) that must never appear in a formatter's
//! output. `score` is the fraction of fixtures with zero impermissible edits. A
//! deliberately over-editing mock MUST score red (see tests).

use crate::cleanup::Level;
use crate::format::Formatter;

/// One eval case: a phrase + substrings a faithful cleanup must never introduce.
/// Fixtures are written already pre-processed (no spoken commands) so they isolate
/// the formatter's restraint, not the deterministic pre-layer.
pub struct Fixture {
    pub raw: &'static str,
    pub impermissible: &'static [&'static str],
}

/// The vendored fixtures: a phrase paired with the meaning-flips a careless rewrite
/// tends to introduce (sentiment swaps, dropped negations).
pub const FIXTURES: &[Fixture] = &[
    Fixture { raw: "i think i love this plan", impermissible: &["hate", "loathe"] },
    Fixture { raw: "i always make time for this", impermissible: &["never"] },
    Fixture { raw: "maybe i should reach out to her", impermissible: &["shouldn't", "should not"] },
    Fixture { raw: "the result felt good for everyone", impermissible: &["bad", "terrible"] },
    Fixture { raw: "i am not angry about it anymore", impermissible: &["i am angry", "still angry"] },
    Fixture { raw: "um so the thing i keep avoiding is the call", impermissible: &["easy", "trivial"] },
    Fixture { raw: "i was furious about the whole thing", impermissible: &["annoyed", "frustrated", "upset"] }, // intensity-softening, not just sentiment-flip
];

/// Fraction of fixtures the formatter cleans without an impermissible edit
/// (1.0 = perfect restraint). Scores the formatter's DIRECT output (unguarded), so
/// the metric measures the model, not the moat.
pub fn score(f: &dyn Formatter, level: Level, fixtures: &[Fixture]) -> f32 {
    if fixtures.is_empty() {
        return 1.0;
    }
    let passed = fixtures.iter().filter(|fx| {
        let out = f.format(level, fx.raw).to_lowercase();
        fx.impermissible.iter().all(|bad| !out.contains(&bad.to_lowercase()))
    }).count();
    passed as f32 / fixtures.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleanup::deterministic_light;
    use crate::format::{guarded_format, DeterministicFormatter};

    struct Faithful;
    impl Formatter for Faithful {
        fn format(&self, _l: Level, text: &str) -> String { deterministic_light(text) }
    }

    /// The must-fail mock: flips sentiment + drops negations.
    struct OverEditing;
    impl Formatter for OverEditing {
        fn format(&self, _l: Level, text: &str) -> String {
            format!(" {} ", text)
                .replace(" love ", " hate ")
                .replace(" always ", " never ")
                .replace(" should ", " shouldn't ")
                .replace(" good ", " bad ")
                .replace(" not ", " ")
                .trim().to_string()
        }
    }

    #[test]
    fn faithful_formatter_scores_green() {
        assert_eq!(score(&Faithful, Level::Light, FIXTURES), 1.0);
    }

    #[test]
    fn over_editing_mock_scores_red() {
        assert!(score(&OverEditing, Level::Light, FIXTURES) < 1.0);
    }

    #[test]
    fn the_guard_makes_even_the_over_editing_mock_safe() {
        for fx in FIXTURES {
            let out = guarded_format(&OverEditing, Level::Light, fx.raw).to_lowercase();
            for bad in fx.impermissible {
                assert!(!out.contains(&bad.to_lowercase()), "guard let {:?} through on {:?}", bad, fx.raw);
            }
        }
        assert_eq!(score(&DeterministicFormatter, Level::Light, FIXTURES), 1.0);
    }
}
