//! Curated close phrases (spec §7) — the single source of truth shared by the
//! CLI (`src/live.rs` / `src/main.rs`) and the web (`talk-wasm` → `session/modes.ts`).
//!
//! Rotated by a caller-supplied seed (the CLI uses kept-entry count; the web uses
//! the clock) so a returning user doesn't see the same line twice in a row.

/// The curated close phrases, in rotation order.
pub const CLOSE_PHRASES: &[&str] = &[
    "Stillness carries forward.",
    "Said out loud, it weighs less.",
    "You showed up. That was the practice.",
    "Let it settle.",
    "The thread holds.",
    "Nothing to fix. Just to hear.",
    "The words keep working after you stop.",
    "Come back when it tugs.",
];

/// Pick a close phrase by `seed`, rotating over the curated list. The seed is
/// reduced modulo the list length, so any non-negative index maps to a phrase.
pub fn select_close_phrase(seed: usize) -> &'static str {
    CLOSE_PHRASES[seed % CLOSE_PHRASES.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_rotates_over_the_list() {
        assert_eq!(select_close_phrase(0), CLOSE_PHRASES[0]);
        assert_eq!(select_close_phrase(CLOSE_PHRASES.len()), CLOSE_PHRASES[0]);
        assert_eq!(select_close_phrase(CLOSE_PHRASES.len() + 3), CLOSE_PHRASES[3]);
    }
}
