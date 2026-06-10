use crate::source::{Event, TranscriptSource};
use crate::writer::{write_entry, Target, WriteRequest};
use std::path::{Path, PathBuf};
use talk_core::cleanup::Level;
use talk_core::format::{guarded_format, Formatter};
use talk_core::settle::Settle;

pub struct RunConfig<'a> {
    pub base: &'a Path,
    pub date: &'a str,
    pub time: &'a str,
    pub keep_raw: bool,
    pub raw_sidecar: bool,
    pub ephemeral: bool,
    pub formatter: &'a dyn Formatter,
    pub level: Level,
}

/// Consume the whole source, cleaning each committed phrase through
/// `guarded_format` (the deterministic pre-layer + the injected `Formatter`, gated
/// by the content-word guard with a deterministic-Light fallback), settling it,
/// then persisting. `settle` is the single source of truth for the output (raw
/// verbatim + clean), so the file matches what the live renderer shows.
///
/// Note: the Light layer normalizes whitespace, so a spoken `new line` removes the
/// command words but does not preserve structural newlines — structured formatting
/// (Medium/High) is a follow-on concern; this path only guarantees the literal
/// command words don't survive.
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
                let clean = guarded_format(cfg.formatter, cfg.level, &raw);
                settle.commit(&raw, &clean);
            }
            Event::Revise(raw2) => {
                // Pass-2 (Whisper) text is self-cased/punctuated — thin format only
                // (spoken commands + backtrack + continuation de-cap), no re-cap.
                let prev = settle.settled().last().map(|b| b.clean.clone());
                let clean2 = talk_core::cleanup::format_revise(&raw2, prev.as_deref());
                settle.revise_committing(&raw2, &clean2);
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
        raw_sidecar: cfg.raw_sidecar,
        ephemeral: cfg.ephemeral,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::FakeTranscript;

    fn cfg(base: &Path, ephemeral: bool) -> RunConfig<'_> {
        RunConfig {
            base, date: "2026-06-08", time: "08:14", keep_raw: true, raw_sidecar: false, ephemeral,
            formatter: &talk_core::format::DeterministicFormatter, level: Level::Light,
        }
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
        // Only the non-lexical "um" is stripped; the discourse opener "So" is a
        // word the user said and survives (leading-content-word fix).
        assert!(text.contains("So the thing is I keep avoiding it."));
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
            base: dir.path(), date: "2026-06-08", time: "08:14", keep_raw: false, raw_sidecar: false, ephemeral: false,
            formatter: &talk_core::format::DeterministicFormatter, level: Level::Light,
        }).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("new line"));
    }

    #[test]
    fn revise_event_upgrades_the_committing_phrase_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = FakeTranscript::new(vec![
            Event::Commit("the streaming hypothesis".into()),
            Event::Revise("the corrected transcription".into()),
            Event::Done,
        ]);
        let p = run(&mut src, Target::Journal, &cfg(dir.path(), false)).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        // Whisper revise is thin-formatted — no force-capitalize or terminal punct.
        assert!(text.contains("the corrected transcription"));
        assert!(!text.contains("streaming hypothesis"));
        assert!(text.contains("<!-- raw: the corrected transcription -->"));
    }

    #[test]
    fn revise_targets_the_block_it_was_paired_with_not_a_later_commit() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = FakeTranscript::new(vec![
            Event::Commit("alpha streaming".into()),
            Event::Revise("alpha corrected".into()),
            Event::Commit("bravo streaming".into()),
            Event::Done,
        ]);
        let p = run(&mut src, Target::Journal, &cfg(dir.path(), false)).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        // Whisper revise is verbatim — no force-capitalize on the Revise path.
        // Commit path (bravo) still uses guarded_format → deterministic_light.
        assert!(text.contains("alpha corrected"));
        assert!(text.contains("Bravo streaming."));
        assert!(!text.contains("Alpha streaming."));
    }

    #[test]
    fn ephemeral_run_persists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = FakeTranscript::from_text("the secret is");
        let out = run(&mut src, Target::Journal, &cfg(dir.path(), true)).unwrap();
        assert!(out.is_none());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn whisper_revise_is_thin_formatted_and_continuation_decapitalized() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = FakeTranscript::new(vec![
            Event::Commit("rough one".into()),
            Event::Revise("With their product.".into()),
            Event::Commit("rough two".into()),
            Event::Revise("All these edge cases.".into()),
            Event::Done,
        ]);
        let p = run(&mut src, Target::Journal, &cfg(dir.path(), false)).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("With their product."));
        // "With their product." ends in '.', so "All these" is a NEW sentence — capital KEPT.
        assert!(text.contains("All these edge cases."));
    }

    #[test]
    fn an_over_editing_formatter_cannot_corrupt_the_file() {
        struct Flip;
        impl talk_core::format::Formatter for Flip {
            fn format(&self, _l: Level, text: &str) -> String {
                format!(" {} ", text).replace(" love ", " hate ").trim().to_string()
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let mut src = FakeTranscript::new(vec![
            Event::Commit("i love this".into()),
            Event::Done,
        ]);
        let cfg = RunConfig {
            base: dir.path(), date: "2026-06-08", time: "08:14", keep_raw: true, raw_sidecar: false, ephemeral: false,
            formatter: &Flip, level: Level::Light,
        };
        let p = run(&mut src, Target::Journal, &cfg).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("hate"));
        assert!(text.to_lowercase().contains("love"));
    }
}
