use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub served_count: HashMap<String, u32>,
    pub last_served: HashMap<String, u64>,
    pub tick: u64,
    pub held_run: Option<(String, u32)>,
    // Plan 4: streak display; collected but unused in Plan 1.
    pub streak: u32,
    pub last_session_date: Option<String>,
    /// First-run privacy disclosure shown? Set once, the first time a non-ephemeral
    /// session actually proceeds (serde default false for pre-existing state files).
    pub disclosed: bool,
}

impl State {
    pub fn load(text: &str) -> State {
        serde_json::from_str(text).unwrap_or_default()
    }
    /// Serialize for persistence. Callers MUST write the result with
    /// `paths::write_private` (0600) — this file records which private questions
    /// you've engaged with and how often.
    pub fn save(&self) -> String {
        serde_json::to_string_pretty(self).unwrap()
    }

    /// Record that `id` was served now (advances the monotonic tick).
    pub fn record_served(&mut self, id: &str) {
        self.tick += 1;
        self.last_served.insert(id.to_string(), self.tick);
        *self.served_count.entry(id.to_string()).or_insert(0) += 1;
    }

    /// Build the pure selection state talk-core's `select()` consumes.
    pub fn selection_state(&self) -> talk_core::selection::SelectionState {
        talk_core::selection::SelectionState {
            served_count: self.served_count.clone(),
            last_served: self.last_served.clone(),
            held_run: self.held_run.clone(),
        }
    }

    /// After serving `q`, advance held-run bookkeeping for a `held:N` cadence so
    /// the same question keeps being chosen until the run completes, then releases.
    pub fn advance_held(&mut self, q: &talk_core::questions::Question) {
        if let Some(len) = talk_core::questions::Pack::held_len(&q.cadence) {
            let done = match &self.held_run {
                Some((id, n)) if id == &q.id => n + 1,
                _ => 1,
            };
            self.held_run = if done >= len { None } else { Some((q.id.clone(), done)) };
        }
    }
}

/// The 1-based day of a held run for `id`, given the PRE-advance state. Returns
/// `None` for non-held cadences (so only `held:N` questions carry a day label).
pub fn held_day_for(held_run: &Option<(String, u32)>, id: &str, cadence: &str) -> Option<u32> {
    talk_core::questions::Pack::held_len(cadence)?;
    match held_run {
        Some((run_id, done)) if run_id == id => Some(done + 1),
        _ => Some(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_records() {
        let mut s = State::default();
        s.record_served("a");
        s.record_served("a");
        let reloaded = State::load(&s.save());
        assert_eq!(reloaded.served_count.get("a"), Some(&2));
        assert_eq!(reloaded.tick, 2);
    }

    fn q(id: &str, cadence: &str) -> talk_core::questions::Question {
        talk_core::questions::Question {
            id: id.into(), text: String::new(), slug: None,
            addressee: "self".into(), cadence: cadence.into(), slot: None,
        }
    }

    #[test]
    fn advance_held_daily_is_noop() {
        let mut s = State::default();
        s.advance_held(&q("a", "daily"));
        assert!(s.held_run.is_none());
    }

    #[test]
    fn advance_held_runs_then_releases() {
        let mut s = State::default();
        s.advance_held(&q("h", "held:3"));
        assert_eq!(s.held_run, Some(("h".to_string(), 1)));
        s.advance_held(&q("h", "held:3"));
        assert_eq!(s.held_run, Some(("h".to_string(), 2)));
        s.advance_held(&q("h", "held:3"));
        assert!(s.held_run.is_none()); // 3rd serving completes the run
    }

    #[test]
    fn held_day_is_one_based_and_only_for_held_cadence() {
        assert_eq!(held_day_for(&None, "h", "held:7"), Some(1));
        assert_eq!(held_day_for(&Some(("h".into(), 2)), "h", "held:7"), Some(3));
        assert_eq!(held_day_for(&Some(("other".into(), 2)), "h", "held:7"), Some(1));
        assert_eq!(held_day_for(&None, "d", "daily"), None);
    }
}
