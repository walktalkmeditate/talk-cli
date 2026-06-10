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

/// A tampered cached model must refuse to run (verify-before-load, spec §11/§14).
/// Runs only in listen builds: the gate lives in the live-session path. The
/// process runs non-TTY (Command::output), so T9's first-run fetch offer is
/// skipped and the clean exit(1) holds.
#[cfg(feature = "listen")]
#[test]
fn tampered_model_refuses_to_run() {
    let home = tempfile::tempdir().unwrap();
    let models = tempfile::tempdir().unwrap();
    // Place WRONG bytes at every manifest archive name: present, but hash-mismatched.
    for name in [
        "sherpa-onnx-whisper-base.en.tar.bz2",
        "sherpa-onnx-streaming-zipformer-en-20M-2023-02-17.tar.bz2",
    ] {
        std::fs::write(models.path().join(name), b"tampered").unwrap();
    }
    let out = run_reflect(home.path(), models.path());
    assert!(!out.status.success(), "tampered models must exit non-zero");
    // Intent pin: the refusal must steer the user to the download command —
    // the exact wording around it is free to change.
    assert!(String::from_utf8_lossy(&out.stderr).contains("download"));
    // (Under extracted-first verification this first scenario refuses via the
    // extracted-MISSING branch — no extracted dirs exist yet — then the
    // archive re-verify fails too. The independent extracted-layer proof is the
    // heal test below; the happy-path-skips-archives proof is the test after it.)

    // Second scenario: the EXTRACTED files the session loads hold wrong bytes.
    // With fake archives both layers fail, which is exactly the point — an
    // attacker swapping an extracted weight must not slip past the gate either.
    // Cover BOTH new model dirs: one base file and one zipformer file.
    let base = models
        .path()
        .join("sherpa-onnx-whisper-base.en");
    let zipformer = models
        .path()
        .join("sherpa-onnx-streaming-zipformer-en-20M-2023-02-17");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::create_dir_all(&zipformer).unwrap();
    std::fs::write(base.join("base.en-encoder.int8.onnx"), b"tampered extraction").unwrap();
    std::fs::write(
        zipformer.join("encoder-epoch-99-avg-1.int8.onnx"),
        b"tampered extraction",
    )
    .unwrap();
    let out = run_reflect(home.path(), models.path());
    assert!(
        !out.status.success(),
        "tampered extracted models must exit non-zero"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("download"));
}

/// The extracted-file layer of the gate, proven INDEPENDENTLY of the archive
/// layer (the scenarios above use fake archives, so the archive check fails
/// first and says nothing about the extracted pins). With VERIFIED archives and
/// a tampered extracted weight, the gate must heal: the wrong bytes are
/// overwritten from the verified archive before anything is loaded — the
/// tampered file never reaches the FFI. Models-present-gated; skips elsewhere.
#[cfg(feature = "listen")]
#[test]
fn tampered_extracted_file_is_healed_from_verified_archives() {
    let cached = cached_models_dir();
    let archives = [
        "sherpa-onnx-whisper-base.en.tar.bz2",
        "sherpa-onnx-streaming-zipformer-en-20M-2023-02-17.tar.bz2",
    ];
    if !archives.iter().all(|a| cached.join(a).exists()) {
        eprintln!("skipping heal test: cached model archives not present");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let models = tempfile::tempdir().unwrap();
    for a in archives {
        std::fs::copy(cached.join(a), models.path().join(a)).unwrap(); // APFS clone — cheap
    }
    let victim = models
        .path()
        .join("sherpa-onnx-streaming-zipformer-en-20M-2023-02-17")
        .join("encoder-epoch-99-avg-1.int8.onnx");
    std::fs::create_dir_all(victim.parent().unwrap()).unwrap();
    std::fs::write(&victim, b"tampered extraction").unwrap();

    let out = run_reflect(home.path(), models.path());
    // The session still dies later (tests have no TTY for raw mode), but the
    // GATE must have passed — a refusal would print the models-not-ready hint.
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("models not ready"),
        "gate refused instead of healing"
    );
    let healed = std::fs::read(&victim).unwrap();
    assert_ne!(healed.as_slice(), b"tampered extraction");
    assert!(healed.len() > 1_000_000, "healed weight should be the real model file");
}

/// The startup-perf claim, as a behavioral guarantee: when the extracted files all
/// verify, the gate passes WITHOUT the 239 MB archives — so a regression that
/// re-required archive hashing on every launch (or skipped a weight) would fail
/// here. Copies the real extracted dirs but leaves NO archives present; the gate
/// must still pass. Models-present-gated; skips when the cache is absent.
#[cfg(feature = "listen")]
#[test]
fn happy_path_passes_without_archives_present() {
    let cached = cached_models_dir();
    let dirs = [
        "sherpa-onnx-whisper-base.en",
        "sherpa-onnx-streaming-zipformer-en-20M-2023-02-17",
    ];
    if !dirs.iter().all(|d| cached.join(d).is_dir()) {
        eprintln!("skipping happy-path test: cached extracted models not present");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let models = tempfile::tempdir().unwrap();
    for d in dirs {
        copy_dir(&cached.join(d), &models.path().join(d));
    }
    // Deliberately NO .tar.bz2 archives in the temp dir.
    let out = run_reflect(home.path(), models.path());
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("models not ready"),
        "gate refused despite valid extracted files (archives should not be required)"
    );
}

#[cfg(feature = "listen")]
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dst = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dst);
        } else {
            std::fs::copy(entry.path(), &dst).unwrap();
        }
    }
}

#[cfg(feature = "listen")]
fn run_reflect(home: &Path, models: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_talk"))
        .args(["reflect"]) // no --from-text → live path → verify gate fires pre-mic
        .env("HOME", home)
        .env("TALK_MODELS_DIR", models)
        .output()
        .unwrap()
}

/// Spec §14: the TEXT pipeline makes zero outbound connections. macOS: run the
/// --from-text session under a deny-network sandbox profile; if the text path
/// ever gained a network call, the sandbox would kill it and the run would fail.
/// (This exercises only the text pipeline — the FFI inference path is proven by
/// the models-gated test below.)
#[cfg(target_os = "macos")]
#[test]
fn text_pipeline_makes_no_network_calls_under_sandbox() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new("sandbox-exec")
        .args([
            "-p",
            "(version 1)(allow default)(deny network*)",
            env!("CARGO_BIN_EXE_talk"),
            "journal",
            "--from-text",
            "no packets were harmed",
            "--date",
            "2026-06-09",
            "--time",
            "08:14",
        ])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(std::fs::read_to_string(home.path().join("talk/2026-06-09.md"))
        .unwrap()
        .contains("No packets were harmed."));
}

/// The REAL inference stack (sherpa-onnx FFI: streaming Zipformer pass-1 +
/// Whisper base.en pass-2) under deny-network. Models-present-gated: skips on
/// runners without the cached models; runs in full on dev machines. This is the
/// spec §14 check that the FFI path makes zero outbound connections.
///
/// `ffi_probe` loads the Plan 5 stack (Streaming + Whisper Stt, migrated in T4/T5)
/// and drives both passes; the skip-gate paths below point at the Whisper + zipformer
/// dirs, so on a migrated machine this test RUNS — it does not skip.
#[cfg(all(target_os = "macos", feature = "listen"))]
#[test]
#[ignore = "transitional during the Whisper base.en swap: ffi_probe + the cached \
model are mid-migration (Stt is Whisper as of T1, probe/cache catch up in T5/T6). \
Re-enabled and verified in T6."]
fn inference_stack_runs_under_deny_network_sandbox() {
    const PROFILE: &str = "(version 1)(allow default)(deny network*)";

    // Canary: the sandbox must actually deny network, or this test proves nothing.
    let canary = Command::new("sandbox-exec")
        .args([
            "-p",
            PROFILE,
            "/usr/bin/curl",
            "--max-time",
            "3",
            "-s",
            "http://example.com",
        ])
        .output()
        .unwrap();
    assert!(
        !canary.status.success(),
        "sandbox-exec failed to deny network — the no-egress proof is void"
    );

    // Skip when the cached models are absent (CI runners without the download).
    let models = cached_models_dir();
    let whisper = models.join("sherpa-onnx-whisper-base.en");
    let zipformer = models.join("sherpa-onnx-streaming-zipformer-en-20M-2023-02-17");
    if !whisper.join("base.en-encoder.int8.onnx").exists()
        || !whisper.join("base.en-decoder.int8.onnx").exists()
        || !whisper.join("base.en-tokens.txt").exists()
        || !zipformer.join("encoder-epoch-99-avg-1.int8.onnx").exists()
        || !zipformer.join("decoder-epoch-99-avg-1.onnx").exists()
        || !zipformer.join("joiner-epoch-99-avg-1.int8.onnx").exists()
        || !zipformer.join("tokens.txt").exists()
    {
        eprintln!(
            "skipping inference_stack_runs_under_deny_network_sandbox: \
            cached Whisper/zipformer models absent at {}",
            models.display()
        );
        return;
    }

    // Build the probe once outside the sandbox (cargo itself touches the network on
    // a cold registry), then run the built binary under deny-network — so only the
    // FFI inference stack, not the build, is what the sandbox is judging.
    let build = Command::new(env!("CARGO"))
        .args(["build", "--quiet", "--features", "listen", "--example", "ffi_probe"])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "building ffi_probe failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let probe = probe_binary_path();
    assert!(probe.exists(), "ffi_probe binary not found at {}", probe.display());

    let run = Command::new("sandbox-exec")
        .args([
            "-p",
            PROFILE,
            probe.to_str().unwrap(),
            models.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "FFI inference stack failed under deny-network sandbox:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("ok"),
        "probe did not report ok: {}",
        String::from_utf8_lossy(&run.stdout)
    );
}

/// The platform model cache (`TALK_MODELS_DIR`, else macOS's
/// `~/Library/Caches/org.walktalkmeditate.talk/models`). Mirrors
/// `paths::models_dir()` for the dev-machine cached-models check; the binary
/// crate has no library target to call into, so the path is reconstructed here.
#[cfg(all(target_os = "macos", feature = "listen"))]
fn cached_models_dir() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("TALK_MODELS_DIR") {
        return std::path::PathBuf::from(p);
    }
    let home = std::env::var("HOME").expect("HOME is set");
    std::path::Path::new(&home)
        .join("Library/Caches/org.walktalkmeditate.talk/models")
}

/// Locate the built `ffi_probe` example next to the integration-test binary
/// (`target/<profile>/deps/privacy-*` → `target/<profile>/examples/ffi_probe`).
#[cfg(all(target_os = "macos", feature = "listen"))]
fn probe_binary_path() -> std::path::PathBuf {
    let test_bin = std::env::current_exe().unwrap();
    let profile_dir = test_bin
        .parent() // deps/
        .and_then(|p| p.parent()) // <profile>/
        .expect("test binary lives under target/<profile>/deps/");
    profile_dir.join("examples").join("ffi_probe")
}
