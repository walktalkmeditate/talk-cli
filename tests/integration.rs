use std::path::Path;
use std::process::{Command, Output};

// `directories` resolves the base dir from $HOME on unix, so overriding HOME
// steers the binary's writes under the tempdir.
fn talk(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_talk"))
        .args(args).env("HOME", home).output().unwrap()
}

#[test]
fn byo_reflect_writes_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = talk(dir.path(), &[
        "What am I avoiding?", "--from-text", "um keep avoiding it",
        "--date", "2026-06-08", "--time", "08:14",
    ]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let text = std::fs::read_to_string(dir.path().join("talk/what-am-i-avoiding.md")).unwrap();
    assert!(text.contains("id: what-am-i-avoiding"));
    assert!(text.contains("pack: byo"));
    assert!(text.contains("## 2026-06-08"));
    assert!(text.contains("Keep avoiding it.")); // leading "um" stripped by Light
}

#[test]
fn bare_reflect_selects_from_spine_by_time_of_day() {
    let dir = tempfile::tempdir().unwrap();
    // 07:30 → morning slot → the spine's morning question, keyed by its AUTHORED id.
    let out = talk(dir.path(), &[
        "--from-text", "something came up", "--date", "2026-06-08", "--time", "07:30",
    ]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let text = std::fs::read_to_string(dir.path().join("talk/grateful-this-moment.md")).unwrap();
    assert!(text.contains("id: grateful-this-moment"));
    assert!(text.contains("pack: spine"));
}

#[test]
fn spine_is_the_curated_spine() {
    let spine = talk_core::questions::Pack::from_toml(include_str!("../questions/spine.toml")).unwrap();
    let n = spine.questions.len();
    // Pinned at conversion time: 60 = 65 sourced - 5 dropped by the curation gate.
    assert_eq!(n, 60);
    let morning = spine.questions.iter().filter(|q| q.slot.as_deref() == Some("morning")).count();
    let evening = spine.questions.iter().filter(|q| q.slot.as_deref() == Some("evening")).count();
    assert_eq!(morning, 10); // pinned: all 10 morning.yaml survivors
    assert_eq!(evening, 13); // pinned: 13 evening.yaml survivors (15 - 2 dropped)
    let mut ids: Vec<&str> = spine.questions.iter().map(|q| q.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n, "ids must be unique");
    assert!(ids.iter().all(|id| id.chars().all(|c| c.is_ascii_lowercase() || c == '-')));
}

#[test]
fn unburden_keeps_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = talk(dir.path(), &[
        "unburden", "--from-text", "the secret is", "--date", "2026-06-08", "--time", "08:14",
    ]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Released. Nothing was written."));

    let talk_dir = dir.path().join("talk");
    assert_eq!(std::fs::read_dir(&talk_dir).unwrap().count(), 0, "ephemeral left files behind");
}

#[test]
fn missing_from_text_errors_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let out = talk(dir.path(), &["journal", "--date", "2026-06-08", "--time", "08:14"]);
    assert!(!out.status.success()); // exits non-zero, writes nothing
    assert_eq!(std::fs::read_dir(dir.path().join("talk")).unwrap().count(), 0);
}

#[test]
fn config_default_mode_journal_routes_bare_talk_to_journal() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("talk")).unwrap();
    std::fs::write(dir.path().join("talk/config.toml"), "default_mode = \"journal\"\n").unwrap();
    let out = talk(dir.path(), &["--from-text", "just talking", "--date", "2026-06-08", "--time", "09:00"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(dir.path().join("talk/2026-06-08.md").exists(), "bare talk should have written a journal file");
}

#[test]
fn thread_returns_the_right_file_on_slug_collision() {
    let dir = tempfile::tempdir().unwrap();
    // Both questions share the first 6 words → same base slug; the 2nd is suffixed on write.
    let q1 = "what am i avoiding in life";
    let q2 = "what am i avoiding in life today and tomorrow";
    talk(dir.path(), &[q1, "--from-text", "first answer", "--date", "2026-06-08", "--time", "08:00"]);
    talk(dir.path(), &[q2, "--from-text", "second answer", "--date", "2026-06-08", "--time", "09:00"]);
    let out = talk(dir.path(), &["thread", q2]);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("second answer"), "thread for q2 returned the wrong file: {s}");
    assert!(!s.contains("first answer"));
}

#[test]
fn default_pack_config_serves_from_that_pack() {
    let dir = tempfile::tempdir().unwrap();
    let talk_dir = dir.path().join("talk");
    std::fs::create_dir_all(&talk_dir).unwrap();
    std::fs::write(talk_dir.join("config.toml"), "default_pack = \"examen\"\n").unwrap();
    let out = talk(dir.path(), &["--from-text", "today held more than i noticed", "--date", "2026-06-09", "--time", "20:30"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    // 20:30 → evening slot → examen's first evening question by declaration order.
    let text = std::fs::read_to_string(talk_dir.join("most-alive-today.md")).unwrap();
    assert!(text.contains("pack: examen"));
}

#[test]
fn thread_lists_questions_by_recency() {
    let dir = tempfile::tempdir().unwrap();
    talk(dir.path(), &["Old question?", "--from-text", "first words", "--date", "2026-06-01", "--time", "08:00"]);
    talk(dir.path(), &["New question?", "--from-text", "second words", "--date", "2026-06-09", "--time", "08:00"]);
    let out = talk(dir.path(), &["thread"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let new_pos = stdout.find("new-question").unwrap();
    let old_pos = stdout.find("old-question").unwrap();
    assert!(new_pos < old_pos, "most recent first:\n{stdout}");
    assert!(stdout.contains("1 entr"));

    let empty = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(empty.path().join("talk")).unwrap();
    let out = talk(empty.path(), &["thread"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("No threads yet"));
}

#[test]
fn date_traversal_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let out = talk(dir.path(), &["journal", "--from-text", "x", "--date", "../escape", "--time", "08:14"]);
    assert!(!out.status.success(), "a traversal date should fail, not write outside base");
    assert!(!dir.path().join("escape.md").exists());
}
