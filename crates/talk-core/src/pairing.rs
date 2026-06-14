//! The off-record Commit/Revise/pause pairing state machine (pure).
//!
//! This is the privacy-critical guard lifted out of `src/live.rs::apply_event` so
//! the CLI and the browser pipeline share ONE implementation of "off-record speech
//! never lands" instead of re-deriving it in two languages.
//!
//! The machine owns only the *decision*: given the kind of transcript event and the
//! current pause/pairing state, it returns what the caller must do (drop it, apply a
//! partial/commit/revise, or finish). It deliberately does NOT touch the transcript
//! text or the `Settle` blocks — the caller still runs its own lexicon + cleanup over
//! the payload, because those transforms differ per consumer (the CLI carries a
//! personal lexicon; the web driver carries its own). Keeping the text out keeps this
//! module pure and consumer-agnostic.
//!
//! ## The invariant (verified against `src/live.rs::apply_event`)
//!
//! - On pause, the live edge is cleared by the caller (not modeled here — it is a
//!   render concern). A `Commit` arriving while paused is DROPPED and arms a flag so
//!   its paired pass-2 `Revise` is ALSO dropped. The worker emits Commit then Revise
//!   serially, so an off-record Commit's Revise would otherwise overwrite the last
//!   ACCEPTED phrase with off-record text.
//! - The flag (`commit_dropped`) is re-armed (cleared) ONLY by the next ACCEPTED
//!   Commit — never by resume — because the off-record Revise can arrive after the
//!   user unpauses.
//! - A STRADDLING Revise (its Commit was accepted on-record, the pass-2 Revise lands
//!   during a pause) still upgrades: Revise is gated on `commit_dropped`, NOT on
//!   `paused`.
//! - `Done` finishes regardless of pause/pairing state.
//! - The finish-drain shares the same machine: it carries `commit_dropped` in while
//!   lifting `paused`, so an in-flight Revise of a pause-dropped Commit is still
//!   dropped even though the drain is on-record.
//!
//! ## Arm ORDER is load-bearing
//!
//! The decision below evaluates the gates in the exact order of `apply_event`'s match
//! arms: `Done` first; then `Revise` (gated on `commit_dropped`, above the paused
//! catch-all, so a straddling Revise still applies); then a paused `Commit` (arms the
//! flag); then the paused catch-all (drops a paused `Partial`); then an on-record
//! `Commit` (clears the flag); then an on-record `Partial`.

/// The kind of transcript event the machine decides on. The caller holds the actual
/// payload (the raw/partial text) and applies it on an `Apply*` decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// A revised partial hypothesis for the live edge.
    Partial,
    /// A phrase boundary: the final raw text of the committed phrase (pass 1).
    Commit,
    /// A better transcription of the LAST committed phrase (pass 2).
    Revise,
    /// The user finished the whole turn.
    Done,
}

/// What the caller must do with the event whose kind it passed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// The turn is finished (stop the loop / drain).
    Done,
    /// Discard this event — off-record (paused) or a disarmed pass-2 Revise. The
    /// caller records nothing and lets the channel shrink.
    Drop,
    /// Apply the payload as a live-edge partial.
    ApplyPartial,
    /// Apply the payload as a committed phrase (the caller commits it to `Settle`).
    ApplyCommit,
    /// Apply the payload as a pass-2 revise of the committing block.
    ApplyRevise,
}

/// The pause + pass-2 pairing guards. Construct `default()` for a fresh session;
/// the finish-drain reuses the same value (carrying `commit_dropped`, lifting
/// `paused`) so its guards can't diverge from the live loop's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pairing {
    paused: bool,
    /// A Revise whose paired Commit was dropped during pause must be dropped too, or
    /// pass-2 text of OFF-RECORD speech would overwrite the last accepted phrase.
    /// Re-armed only by the next ACCEPTED Commit — never by resume.
    commit_dropped: bool,
}

impl Pairing {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// True iff the next pass-2 Revise will be dropped (its paired Commit was an
    /// off-record drop). Exposed so a driver can mirror the guard for diagnostics.
    pub fn commit_dropped(&self) -> bool {
        self.commit_dropped
    }

    /// Enter off-record. Idempotent. The caller is responsible for clearing the live
    /// edge (a render concern) and for telling its audio worker to go off-record.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Leave off-record. Idempotent. Resume deliberately does NOT re-arm the pairing
    /// guard — an off-record Revise can still be in flight.
    pub fn resume(&mut self) {
        self.paused = false;
    }

    /// Lift `paused` for the finish-drain while carrying `commit_dropped` forward, so
    /// anything still in the channel at finish is treated as on-record EXCEPT an
    /// in-flight Revise of a pause-dropped Commit.
    pub fn begin_finish_drain(&mut self) {
        self.paused = false;
    }

    /// The pure decision. Mutates `commit_dropped` exactly as `apply_event` does and
    /// returns what the caller must do with the payload.
    ///
    /// Arm order matches `src/live.rs::apply_event` (load-bearing — see module docs).
    pub fn decide(&mut self, kind: EventKind) -> Decision {
        match kind {
            EventKind::Done => Decision::Done,
            EventKind::Revise if !self.commit_dropped => Decision::ApplyRevise,
            EventKind::Revise => Decision::Drop, // paired Commit was dropped (off-record)
            EventKind::Commit if self.paused => {
                self.commit_dropped = true; // disarm its paired pass-2 Revise
                Decision::Drop
            }
            _ if self.paused => Decision::Drop, // drain-and-discard a paused Partial
            EventKind::Commit => {
                self.commit_dropped = false; // an accepted Commit re-arms the pairing
                Decision::ApplyCommit
            }
            EventKind::Partial => Decision::ApplyPartial,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_on_record_commit_then_revise_applies_both() {
        let mut p = Pairing::new();
        assert_eq!(p.decide(EventKind::Commit), Decision::ApplyCommit);
        assert_eq!(p.decide(EventKind::Revise), Decision::ApplyRevise);
    }

    #[test]
    fn a_partial_on_record_applies() {
        let mut p = Pairing::new();
        assert_eq!(p.decide(EventKind::Partial), Decision::ApplyPartial);
    }

    #[test]
    fn done_finishes_even_while_paused() {
        let mut p = Pairing::new();
        p.pause();
        assert_eq!(p.decide(EventKind::Done), Decision::Done);
    }

    #[test]
    fn a_paused_partial_is_dropped() {
        let mut p = Pairing::new();
        p.pause();
        assert_eq!(p.decide(EventKind::Partial), Decision::Drop);
    }

    #[test]
    fn a_paused_commit_is_dropped_and_disarms_its_revise() {
        let mut p = Pairing::new();
        p.pause();
        assert_eq!(p.decide(EventKind::Commit), Decision::Drop);
        assert!(p.commit_dropped(), "a paused Commit must disarm its paired Revise");
    }

    #[test]
    fn the_revise_of_a_pause_dropped_commit_is_dropped_even_after_resume() {
        let mut p = Pairing::new();
        assert_eq!(p.decide(EventKind::Commit), Decision::ApplyCommit); // kept, on-record
        p.pause();
        assert_eq!(p.decide(EventKind::Commit), Decision::Drop); // OFF-RECORD
        assert!(p.commit_dropped());
        p.resume(); // resume does NOT re-arm — the off-record Revise is still in flight
        assert_eq!(
            p.decide(EventKind::Revise),
            Decision::Drop,
            "the off-record Revise must be dropped, never overwrite the kept phrase"
        );
    }

    #[test]
    fn a_straddling_revise_applies_while_paused() {
        // Commit accepted on-record; pass-2 lands during a pause → still upgrades.
        let mut p = Pairing::new();
        assert_eq!(p.decide(EventKind::Commit), Decision::ApplyCommit);
        p.pause();
        assert_eq!(
            p.decide(EventKind::Revise),
            Decision::ApplyRevise,
            "a Revise of an on-record Commit upgrades even while paused"
        );
    }

    #[test]
    fn an_accepted_commit_rearms_the_pairing_guard() {
        let mut p = Pairing {
            paused: false,
            commit_dropped: true, // a prior paused Commit left the guard disarmed
        };
        assert_eq!(p.decide(EventKind::Commit), Decision::ApplyCommit);
        assert!(!p.commit_dropped(), "an accepted Commit re-arms the pairing");
        assert_eq!(p.decide(EventKind::Revise), Decision::ApplyRevise);
    }

    #[test]
    fn finish_drain_inherits_the_disarmed_guard() {
        // [space] lands inside the pass-2 window of a pause-dropped Commit: the drain
        // lifts `paused` but carries `commit_dropped`, so the off-record Revise must
        // still be dropped (the P0/P1 leak the review named).
        let mut p = Pairing::new();
        p.pause();
        assert_eq!(p.decide(EventKind::Commit), Decision::Drop); // OFF-RECORD
        assert!(p.commit_dropped());
        p.begin_finish_drain(); // lifts paused, carries commit_dropped
        assert!(!p.is_paused());
        assert_eq!(
            p.decide(EventKind::Revise),
            Decision::Drop,
            "the drain must not let the paused pair's Revise land"
        );
        assert_eq!(p.decide(EventKind::Done), Decision::Done);
    }

    #[test]
    fn finish_drain_lets_an_on_record_phrase_land() {
        // Anything in the channel at finish that was NOT a paused pair is on-record.
        let mut p = Pairing::new();
        p.begin_finish_drain();
        assert_eq!(p.decide(EventKind::Commit), Decision::ApplyCommit);
        assert_eq!(p.decide(EventKind::Revise), Decision::ApplyRevise);
        assert_eq!(p.decide(EventKind::Done), Decision::Done);
    }

    #[test]
    fn pause_and_resume_are_idempotent() {
        let mut p = Pairing::new();
        p.pause();
        p.pause();
        assert!(p.is_paused());
        p.resume();
        p.resume();
        assert!(!p.is_paused());
    }
}
