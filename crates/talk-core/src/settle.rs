#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub clean: String,
    /// Raw is retained so `u` (raw⇄clean toggle) and recovery work.
    pub raw: String,
}

/// Three-phase model per spec §7, so Plan 2's renderer and Plan 3's async swap
/// EXTEND this type instead of rewriting it:
/// - `live`       — the jittering partial hypothesis (not committed)
/// - `committing` — the most recent phrase, shown bright with deterministic-Light,
///   still inside the decode-lag / async-swap window (may be upgraded or take a
///   late STT revision)
/// - `settled`    — finalized, immutable blocks (never move again)
#[derive(Default)]
pub struct Settle {
    settled: Vec<Block>,
    committing: Option<Block>,
    live: String,
    committing_revised: bool,
}

impl Settle {
    pub fn new() -> Self { Self::default() }

    /// A new/revised partial hypothesis for the live edge.
    pub fn on_partial(&mut self, partial: &str) {
        self.live = partial.to_string();
    }

    /// Endpoint boundary: the live edge becomes the committing block (`clean` is
    /// the deterministic-Light result). Any prior committing block's window has
    /// closed, so it is finalized into `settled` first.
    pub fn commit(&mut self, raw: &str, clean: &str) {
        self.finalize();
        self.committing = Some(Block { clean: clean.to_string(), raw: raw.to_string() });
        self.committing_revised = false;
        self.live.clear();
    }

    /// Promote the committing block to settled (its lag/swap window elapsed).
    pub fn finalize(&mut self) {
        if let Some(b) = self.committing.take() {
            self.settled.push(b);
        }
    }

    /// Plan 3 async LLM swap: replace the committing block's clean text while
    /// still inside its window. No-op (returns false) once finalized — the
    /// settled rule wins.
    pub fn upgrade_committing(&mut self, clean: &str) -> bool {
        match self.committing.as_mut() {
            Some(b) => {
                b.clean = clean.to_string();
                self.committing_revised = true;
                true
            }
            None => false,
        }
    }

    /// Plan 5 second-pass swap: replace BOTH raw and clean of the committing
    /// block (a better transcription of the same audio, not a reformat). No-op
    /// once finalized — the settled rule wins, exactly like `upgrade_committing`.
    pub fn revise_committing(&mut self, raw: &str, clean: &str) -> bool {
        match self.committing.as_mut() {
            Some(b) => {
                b.raw = raw.to_string();
                b.clean = clean.to_string();
                self.committing_revised = true;
                true
            }
            None => false,
        }
    }

    /// A late STT revision targeting already-settled text is DROPPED (returns
    /// false so the caller can log it to the raw layer).
    pub fn try_late_revision_settled(&mut self, _index: usize, _new_text: &str) -> bool {
        false
    }

    pub fn settled(&self) -> &[Block] { &self.settled }
    pub fn committing(&self) -> Option<&Block> { self.committing.as_ref() }
    pub fn live(&self) -> &str { &self.live }

    /// True once the committing block has been revised/upgraded (its text is
    /// final-quality). Drives the bright-is-final rendering: dim until revised.
    pub fn committing_revised(&self) -> bool { self.committing_revised }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_updates_only_the_live_edge() {
        let mut s = Settle::new();
        s.on_partial("um so the thing");
        assert_eq!(s.live(), "um so the thing");
        assert!(s.settled().is_empty() && s.committing().is_none());
    }

    #[test]
    fn commit_moves_live_into_committing_not_settled() {
        let mut s = Settle::new();
        s.on_partial("um so the thing is");
        s.commit("um so the thing is", "The thing is.");
        assert_eq!(s.committing().unwrap().clean, "The thing is.");
        assert_eq!(s.live(), "");
        assert!(s.settled().is_empty());
    }

    #[test]
    fn second_commit_finalizes_the_first() {
        let mut s = Settle::new();
        s.commit("a", "A.");
        s.commit("b", "B.");
        assert_eq!(s.settled().len(), 1);
        assert_eq!(s.settled()[0].clean, "A.");
        assert_eq!(s.committing().unwrap().clean, "B.");
    }

    #[test]
    fn upgrade_changes_only_the_committing_block() {
        let mut s = Settle::new();
        s.commit("a", "A.");
        s.commit("b raw", "B.");
        let settled0 = s.settled()[0].clone();
        assert!(s.upgrade_committing("B, refined."));
        assert_eq!(s.settled()[0], settled0); // settled is immutable
        assert_eq!(s.committing().unwrap().clean, "B, refined.");
    }

    #[test]
    fn upgrade_is_noop_after_finalize() {
        let mut s = Settle::new();
        s.commit("a", "A.");
        s.finalize();
        assert!(!s.upgrade_committing("A!"));
        assert_eq!(s.settled()[0].clean, "A.");
    }

    #[test]
    fn revise_replaces_both_raw_and_clean_of_the_committing_block() {
        let mut s = Settle::new();
        s.commit("a", "A.");
        s.commit("live hypothesis", "Live hypothesis.");
        assert!(s.revise_committing("better raw", "Better raw."));
        assert_eq!(s.settled()[0].clean, "A."); // settled untouched
        let c = s.committing().unwrap();
        assert_eq!(c.raw, "better raw");
        assert_eq!(c.clean, "Better raw.");
    }

    #[test]
    fn revise_is_noop_after_finalize() {
        let mut s = Settle::new();
        s.commit("a", "A.");
        s.finalize();
        assert!(!s.revise_committing("x", "X."));
        assert_eq!(s.settled()[0].raw, "a");
    }

    #[test]
    fn committing_revised_flag_tracks_commit_then_revise_then_recommit() {
        let mut s = Settle::new();
        s.commit("a", "A.");
        assert!(!s.committing_revised());
        assert!(s.revise_committing("a2", "A2."));
        assert!(s.committing_revised());
        s.commit("b", "B.");
        assert!(!s.committing_revised());
    }

    #[test]
    fn upgrade_committing_sets_the_revised_flag() {
        let mut s = Settle::new();
        s.commit("a", "A.");
        assert!(!s.committing_revised());
        assert!(s.upgrade_committing("A, refined."));
        assert!(s.committing_revised());
    }

    #[test]
    fn late_revision_never_mutates_settled() {
        let mut s = Settle::new();
        s.commit("first", "First.");
        s.finalize();
        let before = s.settled().to_vec();
        assert!(!s.try_late_revision_settled(0, "FIRST!!"));
        assert_eq!(s.settled(), &before[..]);
    }
}
