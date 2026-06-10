use std::path::Path;
use std::process::{Command, Output};

fn talk(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_talk"))
        .args(args)
        .env("HOME", home)
        .output()
        .unwrap()
}

/// Spec §7: after a full ephemeral session, zero bytes of transcript touch the
/// base dir — no entry, no raw sidecar, no state, no streak.
#[test]
fn ephemeral_leaves_zero_bytes_in_the_base_dir() {
    let dir = tempfile::tempdir().unwrap();
    let out = talk(
        dir.path(),
        &[
            "unburden",
            "--from-text",
            "the secret that must not persist",
            "--date",
            "2026-06-09",
            "--time",
            "08:14",
        ],
    );
    assert!(out.status.success());
    let entries: Vec<_> = std::fs::read_dir(dir.path().join("talk"))
        .unwrap()
        .flatten()
        .collect();
    assert!(entries.is_empty(), "ephemeral persisted: {entries:?}");
    // Scope: base-dir bytes only. The models cache and OS swap are explicitly out
    // of scope (covered by the disclosure, not this test).
    for name in [".state.json", ".streak.toml"] {
        assert!(
            !dir.path().join("talk").join(name).exists(),
            "{name} written by ephemeral"
        );
    }
    assert!(
        !dir.path().join("talk").join(".raw").exists(),
        ".raw/ written by ephemeral"
    );
}
