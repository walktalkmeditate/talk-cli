# Whisper base.en Pass-2 Swap — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the pass-2 transcription model (Moonshine base) with Whisper base.en (int8) on the existing serial worker, for accuracy + self-punctuation, with no architectural change.

**Architecture:** A drop-in model swap. `Stt` (today a Moonshine `OfflineRecognizer`) becomes a Whisper `OfflineRecognizer`; the 8s decode envelope becomes Whisper's 30s window; the pass-2 Revise text (now self-cased/punctuated) gets a thinned formatter instead of force-capitalize/terminate, plus a conservative continuation de-capitalizer; the manifest pins Whisper base.en int8; the privacy proofs retarget to the new model. The serial worker, settle machine, renderer, and event seam are unchanged.

**Tech Stack:** Rust, sherpa-onnx 1.13.2 (`OfflineWhisperModelConfig`), cpal, ureq. Models pinned + hash-verified.

**Spec:** `docs/superpowers/specs/2026-06-10-talk-cli-whisper-threaded-pass2-design.md` (v3).

**Branch:** `talk-whisper-pass2` (already created; the spec is committed there).

**Pins (computed + corroborated 2026-06-10):**
- Archive `sherpa-onnx-whisper-base.en.tar.bz2` (208,576,005 bytes): `475bc7052ce299c007f6d5d5407ba8601f819a2867f6eecee510ed17df581542`
- `base.en-encoder.int8.onnx`: `ef6b936f4c9b1d90a3b68634b60c4ed8576b26172b33c2535ec0e933c9edb823` (matches HF mirror `csukuangfj/sherpa-onnx-whisper-base.en`)
- `base.en-decoder.int8.onnx`: `f7162ad6db2dbef16cfaeaa7f945b9d7dd9c1b8d472f6aca82f2273d185e4d41` (matches HF mirror)
- `base.en-tokens.txt`: `306cd27f03c1a714eca7108e03d66b7dc042abe8c258b44c199a7ed9838dd930`

---

## Task 1: Whisper `Stt` wrapper + 30s envelope + hallucination guard

**Files:**
- Modify: `src/listen/stt.rs` (whole file)
- Modify: `src/listen/mod.rs` (wire the guard into the two rescue branches)

- [ ] **Step 1: Rewrite `Stt::new` to a Whisper recognizer.** Replace the Moonshine config:

```rust
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};

pub struct Stt {
    recognizer: OfflineRecognizer,
}

impl Stt {
    /// Paths from the unpacked Whisper base.en (int8) model dir:
    /// `base.en-encoder.int8.onnx`, `base.en-decoder.int8.onnx`, `base.en-tokens.txt`.
    pub fn new(encoder: &str, decoder: &str, tokens: &str) -> Result<Stt, String> {
        let mut cfg = OfflineRecognizerConfig::default();
        cfg.model_config.whisper.encoder = Some(encoder.to_string());
        cfg.model_config.whisper.decoder = Some(decoder.to_string());
        cfg.model_config.whisper.language = Some("en".to_string());
        cfg.model_config.whisper.task = Some("transcribe".to_string());
        cfg.model_config.tokens = Some(tokens.to_string());
        cfg.model_config.provider = Some("cpu".to_string());
        cfg.model_config.num_threads = 2;
        let recognizer = OfflineRecognizer::create(&cfg)
            .ok_or_else(|| "failed to create Whisper recognizer (check model paths)".to_string())?;
        Ok(Stt { recognizer })
    }

    /// Transcribe one 16 kHz mono segment. Whisper takes one ≤30s window; use
    /// `transcribe_chunked` for anything that could exceed it.
    pub fn transcribe(&self, samples: &[f32], sample_rate: i32) -> String {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(sample_rate, samples);
        self.recognizer.decode(&stream);
        stream.get_result().map(|r| r.text.trim().to_string()).unwrap_or_default()
    }
}
```

- [ ] **Step 2: Change the decode envelope from 8s (Moonshine) to 30s (Whisper).** Update the three constants near `transcribe_chunked`:

```rust
/// Whisper decodes a fixed 30 s mel window; segments are ≤20 s (rule3 cap), so a
/// single call covers them. The chunker only fires defensively past 30 s.
const MAX_DECODE_SECS: f32 = 30.0;
/// Preferred cut point, searched ±CUT_SLACK_SECS for the quietest gap.
const CUT_TARGET_SECS: f32 = 28.0;
const CUT_SLACK_SECS: f32 = 1.5;
```

(`transcribe_chunked` and `quietest_cut` bodies are unchanged — only these constants move. Update the doc comment above `transcribe_chunked` to say "past Whisper's 30 s window" instead of "the model errors past ~8 s".)

- [ ] **Step 3: Add a hallucination sanity-check (pure, TDD it first).** Append a test to `stt.rs`'s `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn suspect_hallucination_flags_repetition_and_density() {
        // single token repeated (classic Whisper-on-silence output)
        assert!(suspect_hallucination("you you you you", 3.0));
        // implausibly dense for the duration
        assert!(suspect_hallucination("a b c d e f g h i j k l", 1.0));
        // real speech is fine
        assert!(!suspect_hallucination("oh well that worked", 1.3));
        assert!(!suspect_hallucination("", 2.0));
    }
```

- [ ] **Step 4: Run it to confirm it fails** (function undefined):

Run: `cargo test --features listen --quiet -p talk suspect_hallucination 2>&1 | tail -5`
Expected: compile error / FAIL — `suspect_hallucination` not found.

- [ ] **Step 5: Implement `suspect_hallucination`** (add to `stt.rs`, above the tests):

```rust
/// A Whisper transcript is a suspected hallucination over near-silence when it is
/// a single token repeated or implausibly dense for the audio length. Used only on
/// the quiet-speech rescue path (pass-1 found nothing), where Whisper is most prone
/// to inventing text from silence.
pub fn suspect_hallucination(text: &str, audio_secs: f32) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 4 {
        return false;
    }
    if words.len() as f32 > audio_secs * 4.0 + 4.0 {
        return true;
    }
    let distinct: std::collections::HashSet<String> =
        words.iter().map(|w| w.to_lowercase()).collect();
    distinct.len() == 1
}
```

- [ ] **Step 6: Run to confirm it passes**

Run: `cargo test --features listen --quiet -p talk suspect_hallucination 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 7: Wire the guard into the worker's two RESCUE branches** in `src/listen/mod.rs` (the normal path, where pass-1 had text, is left untouched — only the rescue path, where Whisper transcribes near-silence, is guarded). In BOTH the endpoint-rescue branch (`plausibly_speech(&segment, RESCUE_MIN_ENDPOINT_SAMPLES)`) and the finish-flush-rescue branch (`RESCUE_MIN_FLUSH_SAMPLES`), change:

```rust
                            } else if plausibly_speech(&segment, RESCUE_MIN_ENDPOINT_SAMPLES) {
                                let text2 = stt::transcribe_chunked(&stt, &segment, 16_000);
                                let secs = segment.len() as f32 / 16_000.0;
                                if !text2.trim().is_empty()
                                    && !stt::suspect_hallucination(&text2, secs)
                                {
                                    let _ = tx.send(Event::Commit(text2.clone()));
                                    let _ = tx.send(Event::Revise(text2));
                                }
                            }
```

(Apply the same `let secs = … ; if !empty && !suspect_hallucination` guard in the flush-rescue branch, which uses `RESCUE_MIN_FLUSH_SAMPLES`.)

- [ ] **Step 8: Build with listen to confirm it compiles**

Run: `cargo build --features listen --quiet 2>&1 | tail -3`
Expected: compiles clean.

- [ ] **Step 9: Commit**

```bash
git add src/listen/stt.rs src/listen/mod.rs
git commit -m "feat(listen): Whisper base.en Stt wrapper + 30s envelope + rescue-path hallucination guard"
```

---

## Task 2: Thin pass-2 formatter — `format_revise` + `decapitalize_continuation`

**Files:**
- Modify: `crates/talk-core/src/cleanup.rs`

- [ ] **Step 1: Write the failing tests** (append to `cleanup.rs` `mod tests`):

```rust
    #[test]
    fn decapitalize_lowercases_an_allowlist_continuation_after_unterminated_prior() {
        // prior block did not end a sentence → "All these" continues it
        assert_eq!(
            decapitalize_continuation("All these edge cases get sorted out.", Some("with their product")),
            "all these edge cases get sorted out."
        );
    }

    #[test]
    fn decapitalize_keeps_capital_after_a_terminated_prior() {
        assert_eq!(
            decapitalize_continuation("All these edge cases.", Some("That worked.")),
            "All these edge cases."
        );
    }

    #[test]
    fn decapitalize_never_lowercases_a_non_allowlist_word_protecting_proper_nouns() {
        // "Whisper" is not a continuation function word → keep its capital even if
        // the prior block is unterminated.
        assert_eq!(
            decapitalize_continuation("Whisper does the rest", Some("the tool i use is")),
            "Whisper does the rest"
        );
    }

    #[test]
    fn format_revise_trusts_whisper_casing_and_applies_features() {
        // Whisper text is already cased+punctuated; format_revise must NOT re-capitalize
        // or force terminal punctuation, but DOES apply spoken commands + backtrack.
        assert_eq!(format_revise("hello there", None), "hello there");
        assert_eq!(format_revise("first line new line second", None), "first line\nsecond");
    }
```

- [ ] **Step 2: Run to confirm they fail**

Run: `cargo test --quiet -p talk-core decapitalize 2>&1 | tail -5 && cargo test --quiet -p talk-core format_revise 2>&1 | tail -5`
Expected: compile error — `decapitalize_continuation` / `format_revise` not found.

- [ ] **Step 3: Implement both** (add to `cleanup.rs`, after `apply_backtrack`):

```rust
/// Continuation function-words: common enough as sentence-internal openers that
/// lowercasing them when a sentence spans a pause is safe. Deliberately excludes
/// anything proper-noun-shaped — we never lowercase an arbitrary capitalized token.
const CONTINUATIONS: &[&str] = &[
    "and", "but", "so", "or", "the", "a", "an", "it", "that", "this", "these",
    "those", "all", "then", "because", "which", "who",
];

/// Lowercase the first letter of `text` when it CONTINUES the previous block —
/// the previous block didn't end a sentence (no terminal `.!?`) AND the first word
/// is an allow-listed continuation word. Whisper cases each segment as a fresh
/// sentence; this undoes the spurious mid-sentence capital when a sentence spans a
/// pause. Conservative by construction (only the allow-list; never a proper noun).
pub fn decapitalize_continuation(text: &str, prev_clean: Option<&str>) -> String {
    let continues = prev_clean.is_some_and(|p| {
        !matches!(p.trim_end().chars().last(), Some('.') | Some('!') | Some('?') | None)
    });
    if !continues {
        return text.to_string();
    }
    let first = text.split_whitespace().next().unwrap_or("");
    let bare = first.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    if !CONTINUATIONS.contains(&bare.as_str()) {
        return text.to_string();
    }
    let mut chars = text.chars();
    match chars.next() {
        Some(c) if c.is_uppercase() => c.to_lowercase().collect::<String>() + chars.as_str(),
        _ => text.to_string(),
    }
}

/// Format a pass-2 Whisper revise. Whisper already cased + punctuated, so this does
/// NOT re-capitalize sentence starts or force terminal punctuation (that re-creates
/// the per-segment mid-sentence capital). It applies only the spoken-command and
/// `scratch that` backtrack features and the continuation de-capitalizer.
pub fn format_revise(whisper: &str, prev_clean: Option<&str>) -> String {
    let pre = apply_backtrack(&apply_spoken_commands(whisper));
    decapitalize_continuation(&pre, prev_clean)
}
```

- [ ] **Step 4: Run to confirm they pass**

Run: `cargo test --quiet -p talk-core cleanup 2>&1 | tail -5`
Expected: PASS (all cleanup tests, including the four new ones).

- [ ] **Step 5: Commit**

```bash
git add crates/talk-core/src/cleanup.rs
git commit -m "feat(core): format_revise + conservative continuation de-capitalizer for Whisper output"
```

---

## Task 3: Wire `format_revise` into the Revise path (live + session)

**Files:**
- Modify: `src/live.rs` (the `apply_event` Revise arm)
- Modify: `src/session.rs` (the `run` loop Revise arm + a test)

- [ ] **Step 1: Update `src/session.rs` Revise arm.** Replace lines 43-46:

```rust
            Event::Revise(raw2) => {
                // Pass-2 (Whisper) text is self-cased/punctuated — thin format only
                // (spoken commands + backtrack + continuation de-cap), no re-cap.
                let prev = settle.settled().last().map(|b| b.clean.clone());
                let clean2 = talk_core::cleanup::format_revise(&raw2, prev.as_deref());
                settle.revise_committing(&raw2, &clean2);
            }
```

- [ ] **Step 2: Update `src/live.rs` `apply_event` Revise arm.** Replace the `Event::Revise(raw2) if !g.commit_dropped` body:

```rust
        Event::Revise(raw2) if !g.commit_dropped => {
            let prev = settle.settled().last().map(|b| b.clean.clone());
            let clean = talk_core::cleanup::format_revise(&raw2, prev.as_deref());
            settle.revise_committing(&raw2, &clean);
        }
```

(The `Event::Revise(_) => {}` drop arm and all other arms are unchanged.)

- [ ] **Step 3: Add a session test proving the thin-format + de-cap on a two-phrase revise.** Append to `session.rs` `mod tests`:

```rust
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
        // first revise kept verbatim casing/punctuation (no force-anything)
        assert!(text.contains("With their product."));
        // NOTE: "With their product." ends in '.', so "All these" is a NEW sentence
        // and its capital is KEPT.
        assert!(text.contains("All these edge cases."));
    }
```

- [ ] **Step 4: Run the test + the existing session/live tests**

Run: `cargo test --quiet -p talk session:: 2>&1 | tail -5 && cargo test --quiet -p talk live:: 2>&1 | tail -5`
Expected: PASS. (If `run_settles_and_writes_clean_text` or `revise_*` tests assert old force-formatting on a Revise, update their expectations to the verbatim Whisper text — the Commit path is unchanged so its assertions hold.)

- [ ] **Step 5: Commit**

```bash
git add src/live.rs src/session.rs
git commit -m "feat: thin-format the pass-2 Whisper revise (trust its casing, de-cap continuations)"
```

---

## Task 4: Manifest swap + main.rs model paths/copy

**Files:**
- Modify: `src/download/models.rs` (MODELS + EXTRACTED + the doc comment)
- Modify: `src/main.rs` (model dir/paths, fetch-offer copy)

- [ ] **Step 1: Replace the Moonshine `Artifact` in `MODELS`** with Whisper base.en (keep the zipformer entry unchanged):

```rust
    // Whisper base.en (int8 encoder/decoder + tokens) for pass-2 transcription.
    // Extracts to `sherpa-onnx-whisper-base.en/`; the loader reads
    // `base.en-encoder.int8.onnx`, `base.en-decoder.int8.onnx`, `base.en-tokens.txt`.
    Artifact {
        name: "sherpa-onnx-whisper-base.en.tar.bz2",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-whisper-base.en.tar.bz2",
        // GitHub asset digest is null (predates the field); corroborated by
        // download-and-rehash, with the int8 weights matching the HF mirror below.
        sha256: "475bc7052ce299c007f6d5d5407ba8601f819a2867f6eecee510ed17df581542",
    },
```

- [ ] **Step 2: Replace the three Moonshine entries in `EXTRACTED`** with the three Whisper int8 pins (keep the four zipformer entries unchanged):

```rust
    (
        "sherpa-onnx-whisper-base.en/base.en-encoder.int8.onnx",
        "ef6b936f4c9b1d90a3b68634b60c4ed8576b26172b33c2535ec0e933c9edb823",
    ),
    (
        "sherpa-onnx-whisper-base.en/base.en-decoder.int8.onnx",
        "f7162ad6db2dbef16cfaeaa7f945b9d7dd9c1b8d472f6aca82f2273d185e4d41",
    ),
    (
        "sherpa-onnx-whisper-base.en/base.en-tokens.txt",
        "306cd27f03c1a714eca7108e03d66b7dc042abe8c258b44c199a7ed9838dd930",
    ),
```

- [ ] **Step 3: Update the `MODELS` doc comment** — replace the Moonshine paragraph and corroboration note to describe Whisper base.en: download-and-rehash of the archive, and the int8 encoder/decoder matching the HF mirror `csukuangfj/sherpa-onnx-whisper-base.en` (LFS `ef6b936f…` / `f7162ad6…`). State combined download ≈ 327 MB (zipformer 128 + whisper-base 199).

- [ ] **Step 4: Update `src/main.rs` model paths.** Replace the moonshine dir + the three moonshine `.join(...)` lines in the `let (Some(enc), ...)` tuple:

```rust
    let whisper = models.join("sherpa-onnx-whisper-base.en");
    // ...
        whisper.join("base.en-encoder.int8.onnx").to_str().map(str::to_owned),
        whisper.join("base.en-decoder.int8.onnx").to_str().map(str::to_owned),
        whisper.join("base.en-tokens.txt").to_str().map(str::to_owned),
```

(The `let stt = listen::stt::Stt::new(&enc, &dec, &tok)` call is unchanged — same 3-arg signature.)

- [ ] **Step 5: Update the fetch-offer copy** in `offer_first_run_fetch` to the new size (e.g. "new models: ~330 MB, one time").

- [ ] **Step 6: Build (no test — verified on-machine in Task 6)**

Run: `cargo build --features listen --quiet 2>&1 | tail -3`
Expected: compiles clean.

- [ ] **Step 7: Commit**

```bash
git add src/download/models.rs src/main.rs
git commit -m "feat(models): pin Whisper base.en int8 (corroborated); drop Moonshine; wire Stt paths"
```

---

## Task 5: Retarget the privacy proofs + ffi_probe to Whisper

**Files:**
- Modify: `examples/ffi_probe.rs` (load Whisper for pass-2)
- Modify: `tests/privacy.rs` (Moonshine names → Whisper in the tamper/heal/sandbox tests)

- [ ] **Step 1: Update `examples/ffi_probe.rs`** — change the moonshine dir + file names to the Whisper dir (`sherpa-onnx-whisper-base.en` / `base.en-encoder.int8.onnx` / `base.en-decoder.int8.onnx` / `base.en-tokens.txt`). The `stt::Stt::new(&enc, &dec, &tok)` call and the `transcribe_chunked` call are unchanged (same signatures).

- [ ] **Step 2: Update `tests/privacy.rs` hardcoded Moonshine names.** In `tampered_model_refuses_to_run`, `tampered_extracted_file_is_healed_from_verified_archives`, `happy_path_passes_without_archives_present`, and the sandbox `inference_stack_runs_under_deny_network_sandbox` helper: replace the two archive names array and the moonshine dir/file references:
  - archive: `sherpa-onnx-whisper-base.en.tar.bz2` (replacing the moonshine archive name; keep zipformer).
  - extracted victim/dir: `sherpa-onnx-whisper-base.en/base.en-encoder.int8.onnx` (replacing the moonshine extracted file).

- [ ] **Step 3: Build the listen tests (run in Task 6 with the model present)**

Run: `cargo test --features listen --no-run --quiet 2>&1 | tail -3`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add examples/ffi_probe.rs tests/privacy.rs
git commit -m "test: retarget no-egress + tamper proofs and ffi_probe to Whisper base.en"
```

---

## Task 6: On-machine verification (download, gauntlet, acoustic V1) + wrap

**Files:** none (verification + the accuracy go/no-go)

- [ ] **Step 1: Fetch the real model into the cache via the binary** (proves the manifest end-to-end):

```bash
cargo run --features listen --quiet -- download models 2>&1 | tail -5
cargo run --features listen --quiet -- download verify 2>&1 | tail -3
```
Expected: fetches `sherpa-onnx-whisper-base.en.tar.bz2`, extracts, all pins verify (exit 0). (If the model is already in `/tmp` from probing, the fetch still validates the pinned hashes.)

- [ ] **Step 2: Full test gauntlet**

```bash
cargo test --workspace --quiet 2>&1 | grep -E "test result|FAILED"
cargo test --features listen --quiet 2>&1 | grep -E "test result|FAILED"   # includes the retargeted privacy + sandbox proofs (real model)
cargo test --features download --quiet 2>&1 | grep -c "test result: ok"
```
Expected: all green; the deny-network sandbox proof runs the Whisper FFI for real.

- [ ] **Step 3: clippy ×3 feature sets**

```bash
for f in "" "--features listen" "--features download"; do cargo clippy --workspace --all-targets $f --quiet -- -D warnings 2>&1 | tail -1; echo "[$f] $?"; done
```
Expected: clean (exit 0) ×3.

- [ ] **Step 4: Acoustic loopback — the V1 accuracy go/no-go (release build, audio unmuted).** Build release; check `osascript -e 'get volume settings'` and unmute; synthesize/play a phrase through the speakers into the mic under a `script` pty with `TALK_BASE_DIR=/tmp/...`; assert the written file contains mixed-case, punctuated Whisper text. **Judge accuracy:** does base.en transcribe at least as well as Moonshine did on the same audio? If it regresses the reported errors and small.en would fix them, STOP and escalate to the v2 (threaded small.en) design. Restore the mute state afterward; delete temp files individually (no `rm -rf`).

- [ ] **Step 5: Update the spec §17/build-state and the OpenWolf bug/memory logs**, then leave the branch ready for `/ce-code-review base:main`.

```bash
git add -A && git commit -m "test: Whisper base.en swap verified on-machine (gauntlet + sandbox proof + acoustic V1)"
git log --oneline -8
```

---

## After all tasks

Run `/ce-code-review base:main` on the `talk-whisper-pass2` branch (the established gate), auto-resolve findings, then merge `--no-ff` to `main`. The spec's v2 (threaded small.en) stays in git history as the escalation path if V1 ever fails.
