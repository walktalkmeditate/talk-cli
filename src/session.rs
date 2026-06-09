use crate::source::{Event, TranscriptSource};
use crate::writer::{write_entry, Target, WriteRequest};
use std::path::{Path, PathBuf};
use talk_core::cleanup::{apply_backtrack, apply_spoken_commands, deterministic_light};
use talk_core::settle::Settle;

pub struct RunConfig<'a> {
    pub base: &'a Path,
    pub date: &'a str,
    pub time: &'a str,
    pub keep_raw: bool,
    pub ephemeral: bool,
}

/// Consume the whole source, running the deterministic cleanup layer
/// (spoken-commands → backtrack → Light) on each committed phrase, settling it,
/// then persisting. `settle` is the single source of truth for the output (raw
/// verbatim + Light clean), so the file matches what Plan 2 will render.
///
/// Note: the Light layer normalizes whitespace, so a spoken `new line` removes
/// the command words but does not yet preserve structural newlines — structured
/// formatting is a Plan 3 (High cleanup / LLM) concern. Plan 1 only guarantees
/// the literal command words don't survive.
pub fn run(
    source: &mut dyn TranscriptSource,
    target: Target,
    cfg: &RunConfig,
) -> std::io::Result<Option<PathBuf>> {
    let mut settle = Settle::new();

    while let Some(ev) = source.next() {
        match ev {
            Event::Partial(p) => settle.on_partial(&p),
            Event::Commit(raw) => {
                let pre = apply_backtrack(&apply_spoken_commands(&raw));
                let clean = deterministic_light(&pre);
                settle.commit(&raw, &clean); // raw stored verbatim for recovery
            }
            Event::Done => break,
        }
    }
    settle.finalize(); // promote the last committing block

    let raw_joined = settle.settled().iter().map(|b| b.raw.as_str()).collect::<Vec<_>>().join(" ");
    let clean_joined = settle.settled().iter().map(|b| b.clean.as_str()).collect::<Vec<_>>().join(" ");

    write_entry(&WriteRequest {
        base: cfg.base,
        target,
        date: cfg.date,
        time: cfg.time,
        raw: Some(&raw_joined),
        clean: &clean_joined,
        keep_raw: cfg.keep_raw,
        ephemeral: cfg.ephemeral,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::FakeTranscript;

    fn cfg(base: &Path, ephemeral: bool) -> RunConfig<'_> {
        RunConfig { base, date: "2026-06-08", time: "08:14", keep_raw: true, ephemeral }
    }

    #[test]
    fn run_settles_and_writes_clean_text() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = FakeTranscript::new(vec![
            Event::Partial("um so the thing".into()),
            Event::Commit("um so the thing is i keep avoiding it".into()),
            Event::Done,
        ]);
        let p = run(&mut src, Target::Journal, &cfg(dir.path(), false)).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("The thing is i keep avoiding it."));
        assert!(text.contains("<!-- raw: um so the thing is i keep avoiding it -->"));
    }

    #[test]
    fn multiple_commits_accumulate_into_one_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = FakeTranscript::new(vec![
            Event::Commit("the first thing".into()),
            Event::Commit("the second thing".into()),
            Event::Done,
        ]);
        let p = run(&mut src, Target::Journal, &cfg(dir.path(), false)).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("The first thing.") && text.contains("The second thing."));
    }

    #[test]
    fn spoken_command_words_do_not_survive() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = FakeTranscript::new(vec![
            Event::Commit("first point new line second point".into()),
            Event::Done,
        ]);
        let p = run(&mut src, Target::Journal, &RunConfig {
            base: dir.path(), date: "2026-06-08", time: "08:14", keep_raw: false, ephemeral: false,
        }).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("new line"));
    }

    #[test]
    fn ephemeral_run_persists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = FakeTranscript::from_text("the secret is");
        let out = run(&mut src, Target::Journal, &cfg(dir.path(), true)).unwrap();
        assert!(out.is_none());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
