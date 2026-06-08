use crate::clock::{time_of_day, TimeOfDay};
use crate::questions::{Pack, Question};

/// Caller-supplied selection state (persisted on disk by the binary).
#[derive(Default)]
pub struct SelectionState {
    /// id -> times served
    pub served_count: std::collections::HashMap<String, u32>,
    /// id -> last-served ordinal (monotonic counter; higher = more recent)
    pub last_served: std::collections::HashMap<String, u64>,
    /// An in-progress held run: (question id, turns completed so far).
    pub held_run: Option<(String, u32)>,
}

pub fn select<'a>(pack: &'a Pack, state: &SelectionState, hour: u32) -> Option<&'a Question> {
    // 1. A held run in progress wins until complete.
    if let Some((id, done)) = &state.held_run {
        if let Some(q) = pack.questions.iter().find(|q| &q.id == id) {
            if let Some(len) = Pack::held_len(&q.cadence) {
                if *done < len {
                    return Some(q);
                }
            }
        }
    }

    let slot = match time_of_day(hour) {
        TimeOfDay::Morning => Some("morning"),
        TimeOfDay::Evening => Some("evening"),
        _ => None,
    };

    let candidates: Vec<&Question> = match slot {
        Some(s) if pack.questions.iter().any(|q| q.slot.as_deref() == Some(s)) => {
            pack.questions.iter().filter(|q| q.slot.as_deref() == Some(s)).collect()
        }
        _ => pack.questions.iter().collect(),
    };

    // 2/3. Least-recently-served, then lowest count, then declaration order.
    candidates.into_iter().enumerate().min_by(|(ia, a), (ib, b)| {
        let la = state.last_served.get(&a.id).copied().unwrap_or(0);
        let lb = state.last_served.get(&b.id).copied().unwrap_or(0);
        let ca = state.served_count.get(&a.id).copied().unwrap_or(0);
        let cb = state.served_count.get(&b.id).copied().unwrap_or(0);
        la.cmp(&lb).then(ca.cmp(&cb)).then(ia.cmp(ib))
    }).map(|(_, q)| q)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack() -> Pack {
        Pack::from_toml(r#"
            name = "t"
            [[questions]]
            id = "a"
            text = "A?"
            slot = "morning"
            [[questions]]
            id = "b"
            text = "B?"
            slot = "evening"
            [[questions]]
            id = "h"
            text = "Held?"
            cadence = "held:7"
        "#).unwrap()
    }

    #[test]
    fn held_run_keeps_serving_until_complete() {
        let p = pack();
        let st = SelectionState { held_run: Some(("h".into(), 3)), ..Default::default() };
        assert_eq!(select(&p, &st, 10).unwrap().id, "h");
    }

    #[test]
    fn held_run_releases_when_complete() {
        let p = pack();
        let st = SelectionState { held_run: Some(("h".into(), 7)), ..Default::default() };
        assert_ne!(select(&p, &st, 7).unwrap().id, "h");
    }

    #[test]
    fn morning_prefers_morning_slot() {
        let p = pack();
        let st = SelectionState::default();
        assert_eq!(select(&p, &st, 7).unwrap().id, "a");
    }

    #[test]
    fn rotation_avoids_the_most_recent() {
        let p = pack();
        let mut st = SelectionState::default();
        st.last_served.insert("a".into(), 5);
        // At midday no slot filter; "a" is most recent so it should be skipped.
        assert_ne!(select(&p, &st, 12).unwrap().id, "a");
    }
}
