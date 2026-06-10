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
/// streaming Zipformer (partials/commits) + Whisper base.en (pass-2 revises).
pub trait TranscriptSource {
    fn next(&mut self) -> Option<Event>;
}

/// Pause signal shared between the UI loop and an audio source. `paused` is the
/// live state; `epoch` increments on every pause ENTRY so a source that was
/// blocked through an entire pause window (e.g. inside a pass-2 transcription)
/// still learns a pause happened and can destroy everything then in flight.
// Constructed by the live mic source (`feature = "listen"`); in the default
// build run_loop only carries it, so dead-code analysis needs this allow.
#[cfg_attr(not(feature = "listen"), allow(dead_code))]
#[derive(Default)]
pub struct PauseSignal {
    paused: std::sync::atomic::AtomicBool,
    epoch: std::sync::atomic::AtomicU64,
}

#[cfg_attr(not(feature = "listen"), allow(dead_code))]
impl PauseSignal {
    pub fn pause(&self) {
        self.paused.store(true, std::sync::atomic::Ordering::SeqCst);
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    pub fn resume(&self) {
        self.paused.store(false, std::sync::atomic::Ordering::SeqCst);
    }
    pub fn is_paused(&self) -> bool {
        self.paused.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn epoch(&self) -> u64 {
        self.epoch.load(std::sync::atomic::Ordering::SeqCst)
    }
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
