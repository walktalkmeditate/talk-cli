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

    let text = std::fs::read_to_string(dir.path().join("talk/morning-intention.md")).unwrap();
    assert!(text.contains("id: morning-intention"));
    assert!(text.contains("pack: spine"));
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
