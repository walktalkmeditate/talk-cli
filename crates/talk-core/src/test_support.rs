//! Shared test-only formatter mocks, so format.rs and eval.rs can't drift apart.

use crate::cleanup::{deterministic_light, Level};
use crate::format::Formatter;

/// Restraint incarnate — deterministic-Light never substitutes a word.
pub(crate) struct Faithful;
impl Formatter for Faithful {
    fn format(&self, _l: Level, text: &str) -> String { deterministic_light(text) }
}

/// Substitutes meaning words / drops negations — exactly what the guard rejects.
pub(crate) struct OverEditing;
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
