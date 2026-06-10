/// One emitted transcript event from a source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A revised partial hypothesis for the live edge.
    Partial(String),
    /// A phrase boundary: the final raw text of the committed phrase (pass 1).
    Commit(String),
    /// A better transcription of the LAST committed phrase (pass 2) — replaces
    /// the committing block's raw+clean. Dropped if already finalized.
    // Constructed only by the two-pass worker (`feature = "listen"`); in the
    // default build it is matched (session/live arms) but never built outside
    // tests, so dead-code analysis needs this allow.
    #[cfg_attr(not(feature = "listen"), allow(dead_code))]
    Revise(String),
    /// The user finished the whole turn.
    Done,
}

/// A source of transcript events. The live session implements this over the
/// streaming Zipformer (partials/commits) + Moonshine base (pass-2 revises).
pub trait TranscriptSource {
    fn next(&mut self) -> Option<Event>;
}

/// A scripted source for tests and `--from-text`.
pub struct FakeTranscript {
    events: std::collections::VecDeque<Event>,
}

impl FakeTranscript {
    pub fn new(events: Vec<Event>) -> Self {
        Self { events: events.into() }
    }

    /// Build a one-commit source from a plain string (used by `talk --from-text`).
    pub fn from_text(text: &str) -> Self {
        Self::new(vec![Event::Partial(text.into()), Event::Commit(text.into()), Event::Done])
    }
}

impl TranscriptSource for FakeTranscript {
    fn next(&mut self) -> Option<Event> {
        self.events.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_yields_scripted_events_in_order() {
        let mut s = FakeTranscript::from_text("hello world");
        assert_eq!(s.next(), Some(Event::Partial("hello world".into())));
        assert_eq!(s.next(), Some(Event::Commit("hello world".into())));
        assert_eq!(s.next(), Some(Event::Done));
        assert_eq!(s.next(), None);
    }
}
