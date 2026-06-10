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
fn held_pack_prints_ascending_day_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let talk_dir = dir.path().join("talk");
    std::fs::create_dir_all(&talk_dir).unwrap();
    std::fs::write(talk_dir.join("config.toml"), "default_pack = \"held\"\n").unwrap();
    // A held:7 question stays selected across runs until the run completes — so
    // two consecutive runs are day 1 and day 2 of the same run.
    let day1 = talk(dir.path(), &["--from-text", "first day words", "--date", "2026-06-09", "--time", "10:00"]);
    assert!(day1.status.success(), "stderr: {}", String::from_utf8_lossy(&day1.stderr));
    assert!(String::from_utf8_lossy(&day1.stdout).contains("held day 1"), "{}", String::from_utf8_lossy(&day1.stdout));
    let day2 = talk(dir.path(), &["--from-text", "second day words", "--date", "2026-06-10", "--time", "10:00"]);
    assert!(String::from_utf8_lossy(&day2.stdout).contains("held day 2"), "{}", String::from_utf8_lossy(&day2.stdout));
}

/// Switching default_pack mid-held-run must pause the orphaned run cleanly (the
/// run lives in the pack that started it), serve the new pack, and — once back on
/// held — start a FRESH day-1 run rather than resuming the abandoned one.
#[test]
fn pack_switch_pauses_the_held_run_and_restarts_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let talk_dir = dir.path().join("talk");
    std::fs::create_dir_all(&talk_dir).unwrap();

    std::fs::write(talk_dir.join("config.toml"), "default_pack = \"held\"\n").unwrap();
    let day1 = talk(dir.path(), &["--from-text", "held words", "--date", "2026-06-09", "--time", "10:00"]);
    assert!(day1.status.success(), "stderr: {}", String::from_utf8_lossy(&day1.stderr));
    assert!(String::from_utf8_lossy(&day1.stdout).contains("held day 1"));

    std::fs::write(talk_dir.join("config.toml"), "default_pack = \"spine\"\n").unwrap();
    let switched = talk(dir.path(), &["--from-text", "spine words", "--date", "2026-06-10", "--time", "10:00"]);
    assert!(switched.status.success(), "stderr: {}", String::from_utf8_lossy(&switched.stderr));
    assert!(
        String::from_utf8_lossy(&switched.stderr).contains("held run paused"),
        "stderr: {}", String::from_utf8_lossy(&switched.stderr)
    );
    let spine_files = std::fs::read_dir(&talk_dir).unwrap().flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .filter(|e| std::fs::read_to_string(e.path()).is_ok_and(|t| t.contains("pack: spine")))
        .count();
    assert_eq!(spine_files, 1, "the switch must serve a spine question");

    std::fs::write(talk_dir.join("config.toml"), "default_pack = \"held\"\n").unwrap();
    let fresh = talk(dir.path(), &["--from-text", "held again", "--date", "2026-06-11", "--time", "10:00"]);
    assert!(fresh.status.success(), "stderr: {}", String::from_utf8_lossy(&fresh.stderr));
    assert!(
        String::from_utf8_lossy(&fresh.stdout).contains("held day 1"),
        "back on held must start a FRESH day-1 run: {}", String::from_utf8_lossy(&fresh.stdout)
    );
}

#[test]
fn streak_credits_consecutive_days() {
    let dir = tempfile::tempdir().unwrap();
    talk(dir.path(), &["journal", "--from-text", "day one", "--date", "2026-06-08", "--time", "08:00"]);
    talk(dir.path(), &["journal", "--from-text", "day two", "--date", "2026-06-09", "--time", "08:00"]);
    let out = talk(dir.path(), &["streak"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("current run 2 days"), "{stdout}");
}

#[test]
fn ephemeral_never_credits_streak() {
    let dir = tempfile::tempdir().unwrap();
    talk(dir.path(), &["unburden", "--from-text", "let it go", "--date", "2026-06-09", "--time", "08:00"]);
    let out = talk(dir.path(), &["streak"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("No reflections yet"));
}

#[test]
fn raw_sidecar_config_routes_raw_out_of_the_main_file() {
    let dir = tempfile::tempdir().unwrap();
    let talk_dir = dir.path().join("talk");
    std::fs::create_dir_all(&talk_dir).unwrap();
    std::fs::write(talk_dir.join("config.toml"), "raw_sidecar = true\n").unwrap();
    let out = talk(dir.path(), &["journal", "--from-text", "um the verbatim words", "--date", "2026-06-09", "--time", "08:14"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let main = std::fs::read_to_string(talk_dir.join("2026-06-09.md")).unwrap();
    assert!(!main.contains("<!-- raw"), "main file kept the inline raw: {main}");
    let side = std::fs::read_to_string(talk_dir.join(".raw").join("2026-06-09.md")).unwrap();
    assert!(side.contains("um the verbatim words"), "sidecar missing raw: {side}");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dmode = std::fs::metadata(talk_dir.join(".raw")).unwrap().permissions().mode() & 0o777;
        assert_eq!(dmode, 0o700);
        let fmode = std::fs::metadata(talk_dir.join(".raw").join("2026-06-09.md")).unwrap().permissions().mode() & 0o777;
        assert_eq!(fmode, 0o600);
    }
}

#[test]
fn first_journal_run_discloses_then_stays_quiet() {
    let dir = tempfile::tempdir().unwrap();
    let first = talk(dir.path(), &["journal", "--from-text", "first words", "--date", "2026-06-09", "--time", "08:00"]);
    assert!(first.status.success(), "stderr: {}", String::from_utf8_lossy(&first.stderr));
    assert!(String::from_utf8_lossy(&first.stdout).contains("local"), "first run must disclose: {}", String::from_utf8_lossy(&first.stdout));

    let second = talk(dir.path(), &["journal", "--from-text", "second words", "--date", "2026-06-10", "--time", "08:00"]);
    assert!(second.status.success());
    assert!(!String::from_utf8_lossy(&second.stdout).contains("local"), "second run must NOT disclose again: {}", String::from_utf8_lossy(&second.stdout));
}

#[test]
fn unburden_never_discloses() {
    let dir = tempfile::tempdir().unwrap();
    let out = talk(dir.path(), &["unburden", "--from-text", "let it go", "--date", "2026-06-09", "--time", "08:00"]);
    assert!(out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("words land only"), "ephemeral must never disclose");
}

/// `talk download verify` re-hashes cached artifacts against their pins and exits
/// non-zero on any mismatch, naming the bad artifact. Gated to `--features download`:
/// the no-download stub exits 2 (not 1) for any explicit target.
#[cfg(feature = "download")]
#[test]
fn download_verify_flags_a_tampered_artifact() {
    let home = tempfile::tempdir().unwrap();
    let models = tempfile::tempdir().unwrap();
    // Wrong bytes under a real manifest name → present but hash-mismatched.
    let bad_name = "sherpa-onnx-moonshine-base-en-quantized-2026-02-27.tar.bz2";
    std::fs::write(models.path().join(bad_name), b"tampered").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_talk"))
        .args(["download", "verify"])
        .env("HOME", home.path())
        .env("TALK_MODELS_DIR", models.path())
        .output()
        .unwrap();
    assert!(!out.status.success(), "a tampered artifact must exit non-zero");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(bad_name), "verify must name the bad artifact: {stdout}");
}

/// A BYO question whose derived slug looks like a journal date (YYYY-MM-DD) must
/// NOT claim the journal date-file namespace — it gets the hash-suffixed slug, so
/// a later journal entry for that date can't corrupt the reflect frontmatter.
#[test]
fn byo_date_shaped_question_does_not_claim_the_journal_filename() {
    let dir = tempfile::tempdir().unwrap();
    let talk_dir = dir.path().join("talk");
    let out = talk(dir.path(), &["2026 06 09", "--from-text", "some words", "--date", "2026-06-09", "--time", "08:00"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    // The reflect file must NOT be the bare date file (that's the journal namespace).
    let bare = std::fs::read_to_string(talk_dir.join("2026-06-09.md")).ok();
    let is_reflect = bare.as_deref().is_some_and(|t| t.contains("pack: byo"));
    assert!(!is_reflect, "BYO claimed the journal date filename 2026-06-09.md");
    // It landed under a hash-suffixed slug instead (one byo reflect file exists).
    let byo_files = std::fs::read_dir(&talk_dir).unwrap().flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .filter(|e| std::fs::read_to_string(e.path()).is_ok_and(|t| t.contains("pack: byo")))
        .count();
    assert_eq!(byo_files, 1, "exactly one byo reflect file expected");
}

#[test]
fn date_traversal_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let out = talk(dir.path(), &["journal", "--from-text", "x", "--date", "../escape", "--time", "08:14"]);
    assert!(!out.status.success(), "a traversal date should fail, not write outside base");
    assert!(!dir.path().join("escape.md").exists());
}

/// A write that can't land must surface a clear non-zero failure on the
/// `--from-text` seam (which has no interactive recovery loop — the live path's
/// retry/clipboard/discard prompt is TTY-only). A read-only HOME blocks creating
/// `~/talk` (the binary re-chmods an existing base dir 0o700 on startup, so
/// chmod-ing the base dir itself is undone before the write — locking the parent
/// is the reliable way to force the failure).
#[cfg(unix)]
#[test]
fn write_failure_errors_non_zero_on_from_text_path() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    // Do NOT create ~/talk — a read-only HOME makes ensure_base_dir fail to
    // create it, which is the write-path failure the --from-text seam must report.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();

    let out = talk(dir.path(), &["journal", "--from-text", "x", "--date", "2026-06-09", "--time", "08:14"]);

    // Restore perms FIRST so tempdir cleanup can recurse, regardless of asserts.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();

    assert!(!out.status.success(), "a write into a read-only home must exit non-zero");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("denied") || stderr.to_lowercase().contains("permission"),
        "stderr should name the write failure: {stderr}"
    );
}

/// Spec §17: a `held:7` question is served across seven days into ONE thread file
/// as seven dated sections, the day-7 run carries `held day 7` provenance, and the
/// run releases afterward so the eighth day starts a different held question (a
/// second file). The `held` pack contains only held:7 questions, so day 1 pins one
/// and the run owns selection until it completes.
#[test]
fn held_seven_serves_one_question_across_days() {
    let dir = tempfile::tempdir().unwrap();
    let talk_dir = dir.path().join("talk");
    std::fs::create_dir_all(&talk_dir).unwrap();
    std::fs::write(talk_dir.join("config.toml"), "default_pack = \"held\"\n").unwrap();

    let mut out7 = String::new();
    for day in 1..=7 {
        let date = format!("2026-06-{day:02}");
        let out = talk(dir.path(), &["--from-text", "held words", "--date", &date, "--time", "12:00"]);
        assert!(out.status.success(), "day {day} stderr: {}", String::from_utf8_lossy(&out.stderr));
        if day == 7 {
            out7 = String::from_utf8_lossy(&out.stdout).into_owned();
        }
    }
    // The day-7 run's path line carries the held-run provenance (T5).
    assert!(out7.contains("held day 7"), "day-7 provenance line:\n{out7}");

    // All seven landed in ONE held question's file, as seven dated sections.
    let files: Vec<_> = std::fs::read_dir(&talk_dir).unwrap().flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    assert_eq!(files.len(), 1, "one thread file, got {files:?}");
    let text = std::fs::read_to_string(&files[0]).unwrap();
    assert_eq!(text.matches("## 2026-06-").count(), 7, "seven dated sections:\n{text}");

    // The eighth run releases the completed run to a different held question.
    let out8 = talk(dir.path(), &["--from-text", "released", "--date", "2026-06-08", "--time", "12:00"]);
    assert!(out8.status.success(), "stderr: {}", String::from_utf8_lossy(&out8.stderr));
    let count = std::fs::read_dir(&talk_dir).unwrap().flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md")).count();
    assert_eq!(count, 2, "the eighth day opens a second thread");
}
