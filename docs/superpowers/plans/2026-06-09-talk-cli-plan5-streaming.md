# talk-cli Streaming Two-Pass Transcription Implementation Plan (Plan 5, v1.1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Text appears *while you talk* (live partial hypotheses that revise themselves), every finished phrase is then re-transcribed with full context by a better model and corrected in place — the Wispr-style architecture, fully local — plus the Moonshine **base** accuracy upgrade.

**Architecture (two-pass):** A streaming **Zipformer-20M** transducer (sherpa-onnx `OnlineRecognizer`) consumes mic chunks and emits revising partials → the live edge *jitters* (the spec §7 rendering that has existed since Plan 1, finally fed). Its built-in endpointing (trailing-silence rules + a 20s utterance cap) replaces Silero VAD. On endpoint, the streaming text **commits instantly**; the worker then re-transcribes the buffered segment audio with **Moonshine base** (better WER, full-segment context) and the committing block is **revised in place** via a new `settle::revise_committing` — within the existing never-re-flow invariant: settled blocks still never move; only the live edge jitters and only the committing block upgrades. The worker is serial, so pass-2 always targets the block it just committed — no new threads, no races.

**Measured facts this plan is built on (probed 2026-06-09 on this machine, release build):**
- Streaming Zipformer-20M int8: **RTF 0.01** (43.3s audio in 0.6s — live transcription costs ~1% of one core), 118 partial revisions, endpoint rules fire correctly (incl. the rule3 20s cap mid-monologue). Output is ALL-CAPS and rough ("SEA MEETINGS") — acceptable for a dim live edge; pass-2 replaces it.
- Moonshine **base** quantized (2026-02-27 export): **same ~8s decode envelope as tiny** (8s ok, ≥10s returns empty — it's the export family, not the size), handled by the existing `transcribe_chunked`; quality clearly better than tiny on identical audio; ~**260ms per 8s chunk** (1.27s for a 43s monologue).
- Both release archives verified live: moonshine-base-en-quantized-2026-02-27 (111.3MB, sha256 `43232c1d13013d37317163baec3135bd771a186a4356f28c889bab453bb0e891`), streaming-zipformer-en-20M-2023-02-17 (107.6MB, sha256 `9c559283e8498d3fe95913c79ca1cb454bb26281ac2b102b41306c7d752765d9`). Layouts confirmed: base = `encoder_model.ort`/`decoder_model_merged.ort`/`tokens.txt` (drop-in for `Stt`); zipformer = `{encoder,decoder,joiner}-epoch-99-avg-1[.int8].onnx` + `tokens.txt`.
- The pinned sherpa-onnx 1.13.2 crate ships the full streaming API (`OnlineRecognizer/OnlineStream`, `enable_endpoint`, `rule{1,2,3}_*`, `is_endpoint`, `reset`) — verified in the vendored source.

**Origin:** user field-test feedback (2026-06-09): accuracy + "transcribe as the user keeps talking, go back and fix with more context, like Wispr". Spec §7 reserved exactly this ("a streaming-Zipformer live-jitter mode remains a later option"); the user's request makes it the default. **Design decision (recorded):** streaming becomes THE live experience — the Silero-VAD settle-on-pause path is *replaced*, not kept as a parallel mode (one path, no mode matrix). The calm "speak blind, see it settled" framing yields to the user's explicit preference; the settled-region invariant is unchanged.

---

## Design rules (read first)

1. **Pass-2 is transcription correction, NOT formatting** — it must *not* be gated by `cleanup::guard_accepts` (a better transcription legitimately changes words; the guard is the formatter moat). Both passes' clean text still goes through the deterministic pre-layer + `deterministic_light` (via `guarded_format` with the deterministic formatter, which always passes its own guard).
2. **Serial worker ordering is the correctness argument:** the worker emits `Commit(streaming_text)`, then runs pass-2 (~0.3–1.5s; mic chunks buffer in the bounded channel meanwhile), then emits `Revise(better_text)`, then resumes feeding. The next `Commit` cannot arrive before the prior `Revise`, so `revise_committing` always targets the right block. A `Revise` arriving after the loop finalized (user hit space) is a no-op — same contract as `upgrade_committing`.
3. **The live edge displays lowercased streaming text** (the 20M model emits ALL-CAPS BPE; shouting is wrong for a dim edge). Raw stored on commit = the lowercased streaming text until pass-2 revises both raw and clean with the base transcript.
4. **Empty endpoints are skipped** (leading-silence endpoints produce empty text — observed in the probe; the existing skip-empty-Commit behavior covers it, keep it).
5. **Models manifest is REPLACED:** out: moonshine-tiny + silero_vad (the VAD's job is now the endpoint rules). In: moonshine-base + zipformer-20M (~219MB total download, stated honestly in the fetch offer). `vad.rs`/`Segmenter` are deleted; `capture.rs`, `stt.rs` (+`transcribe_chunked`, unchanged envelope 8.0s) stay.
6. **Endpoint tuning (from the probe):** `rule1_min_trailing_silence = 2.4`, `rule2_min_trailing_silence = 0.8` (mirrors the old settle-on-pause feel), `rule3_min_utterance_length = 20.0` (bounds pass-2 segments; chunked transcription covers the rest). `decoder` uses the **fp32** file (standard sherpa recipe: int8 encoder/joiner + fp32 decoder); encoder/joiner int8.
7. **"Listening" indicator** derives from partial activity (the speaking handle latches when the partial text changed recently) — no VAD needed.

## File structure

```
talk-cli/
  crates/talk-core/src/
    settle.rs            # + revise_committing(raw, clean)        [CI]
    render_model.rs      # live edge renders settle.live() text   [CI]
  src/
    source.rs            # + Event::Revise(String)                [CI]
    session.rs           # handle Revise in the --from-text path  [CI]
    listen/
      vad.rs             # DELETED
      streaming.rs       # NEW: OnlineRecognizer facade + endpoint rules
      stt.rs             # unchanged (base is a drop-in; transcribe_chunked stays)
      mod.rs             # worker rewrite: partials → endpoint commit → pass-2 revise
    download/models.rs   # manifest: base + zipformer (archive + extracted pins)
    main.rs              # model paths, models_ready, fetch-offer size, ffi_probe path
    live.rs              # Partial → on_partial; Revise → revise_committing
  examples/ffi_probe.rs  # updated to the new stack (sandbox no-egress proof)
  tests/privacy.rs       # tamper test against the new manifest
```

---

## Task 1: `settle::revise_committing` + live-edge rendering [CI]

**Files:** `crates/talk-core/src/settle.rs`, `crates/talk-core/src/render_model.rs`

- [ ] **Step 1: settle — failing test first.** Add to `settle.rs` tests:

```rust
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
```

Implementation (next to `upgrade_committing`):

```rust
    /// Plan 5 second-pass swap: replace BOTH raw and clean of the committing
    /// block (a better transcription of the same audio, not a reformat). No-op
    /// once finalized — the settled rule wins, exactly like `upgrade_committing`.
    pub fn revise_committing(&mut self, raw: &str, clean: &str) -> bool {
        match self.committing.as_mut() {
            Some(b) => {
                b.raw = raw.to_string();
                b.clean = clean.to_string();
                true
            }
            None => false,
        }
    }
```

- [ ] **Step 2: render — failing test first.** The live edge currently shows only a static `…` listening dot; it must render the partial text. In `render_model.rs`, change `edge_line`:

```rust
fn edge_line(v: &View) -> String {
    // The live edge: the streaming partial hypothesis while speaking (dim,
    // jittering — spec §7's signature), else a calm dot while listening.
    let live = v.settle.live();
    if !live.is_empty() {
        format!("  {live}")
    } else if v.listening {
        "  …".to_string()
    } else {
        String::new()
    }
}
```

Tests:

```rust
    #[test]
    fn live_partial_renders_at_the_edge() {
        let mut s = Settle::new();
        s.on_partial("the thing i keep");
        let v = base(Mode::Reflect, &s);
        let joined = text(&v);
        assert!(joined.contains("the thing i keep"));
    }

    #[test]
    fn empty_partial_falls_back_to_the_listening_dot() {
        let s = Settle::new();
        let mut v = base(Mode::Reflect, &s);
        v.listening = true;
        assert!(compose(&v).iter().any(|(l, k)| l.contains('…') && *k == LineKind::Edge));
    }
```

(Adapt to the existing test helpers `base`/`text`; the `show_raw` toggle does not affect the live edge.)

- [ ] **Step 3:** `cargo test -p talk-core` green → commit `feat(core): revise_committing + live-edge partial rendering`.

---

## Task 2: `Event::Revise` through the source seam [CI]

**Files:** `src/source.rs`, `src/session.rs`, `src/live.rs` (match arms only)

- [ ] **Step 1:** Add the variant:

```rust
pub enum Event {
    /// A revised partial hypothesis for the live edge.
    Partial(String),
    /// A phrase boundary: the final raw text of the committed phrase (pass 1).
    Commit(String),
    /// A better transcription of the LAST committed phrase (pass 2) — replaces
    /// the committing block's raw+clean. Dropped if already finalized.
    Revise(String),
    /// The user finished the whole turn.
    Done,
}
```

- [ ] **Step 2: session.rs (the `--from-text`/test seam):** in `run`'s match, handle it exactly like the live path will:

```rust
            Event::Revise(raw2) => {
                let clean2 = guarded_format(cfg.formatter, cfg.level, &raw2);
                settle.revise_committing(&raw2, &clean2);
            }
```

Failing test first (this makes the whole two-pass settle behavior testable WITHOUT hardware):

```rust
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
        assert!(text.contains("The corrected transcription."));
        assert!(!text.contains("streaming hypothesis"));
        assert!(text.contains("<!-- raw: the corrected transcription -->"));
    }
```

- [ ] **Step 3: live.rs match arms** (`run_loop` drain + `drain_until_done`): add the same `Revise` arm (pre-layer + `deterministic_light`, then `settle.revise_committing`) — mirroring the existing inline `Commit` arm's style; while paused, `Revise` is still applied (it corrects already-accepted text, unlike a new Commit). In `drain_until_done`, `Revise` also resets the liveness deadline.

- [ ] **Step 4:** full `cargo test` green → commit `feat: Event::Revise — second-pass correction through the source seam`.

---

## Task 3: Models manifest swap (base + zipformer) [CI for gates, network for pins]

**Files:** `src/download/models.rs`, `src/main.rs` (paths + `models_ready` + fetch-offer size), `tests/privacy.rs`

- [ ] **Step 1: manifest.** Replace `MODELS` and the extracted-file pins:

```rust
pub const MODELS: &[Artifact] = &[
    Artifact {
        name: "sherpa-onnx-moonshine-base-en-quantized-2026-02-27.tar.bz2",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-moonshine-base-en-quantized-2026-02-27.tar.bz2",
        sha256: "43232c1d13013d37317163baec3135bd771a186a4356f28c889bab453bb0e891",
    },
    Artifact {
        name: "sherpa-onnx-streaming-zipformer-en-20M-2023-02-17.tar.bz2",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17.tar.bz2",
        sha256: "9c559283e8498d3fe95913c79ca1cb454bb26281ac2b102b41306c7d752765d9",
    },
];

/// Extracted files the session actually loads — verified at load time (the
/// archive hash alone doesn't cover post-extraction tampering). Pins computed
/// from the verified archives' extractions at implementation time.
pub const EXTRACTED: &[(&str, &str)] = &[
    ("sherpa-onnx-moonshine-base-en-quantized-2026-02-27/encoder_model.ort", "PIN_AT_IMPL"),
    ("sherpa-onnx-moonshine-base-en-quantized-2026-02-27/decoder_model_merged.ort", "PIN_AT_IMPL"),
    ("sherpa-onnx-moonshine-base-en-quantized-2026-02-27/tokens.txt", "PIN_AT_IMPL"),
    ("sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/encoder-epoch-99-avg-1.int8.onnx", "PIN_AT_IMPL"),
    ("sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/decoder-epoch-99-avg-1.onnx", "PIN_AT_IMPL"),
    ("sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/joiner-epoch-99-avg-1.int8.onnx", "PIN_AT_IMPL"),
    ("sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/tokens.txt", "PIN_AT_IMPL"),
];
```

`PIN_AT_IMPL`: the archives are already verified-extracted at `/tmp/sherpa-onnx-moonshine-base-en-quantized-2026-02-27` and `/tmp/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17` on this machine — `shasum -a 256` each listed file and pin (same TOFU-plus-archive-provenance model as Plan 4's `MOONSHINE_EXTRACTED`, which this constant replaces/renames).

- [ ] **Step 2: gates + paths.** `models_ready`/re-extract heal path/`talk download verify` iterate the new `EXTRACTED`; `main.rs` model-path construction switches to the base + zipformer dirs; the fetch-offer prompt says **~220 MB** (measured archive total). The old tiny/silero names disappear everywhere (grep).
- [ ] **Step 3: privacy tests.** `tampered_model_refuses_to_run` writes fake files at the NEW manifest names (archives + one extracted `.ort`); assertions unchanged.
- [ ] **Step 4:** `cargo test` + `--features listen` + `--features download` green (gates run against fake files; no network in tests) → commit `feat(models): moonshine base + streaming zipformer manifest`.

---

## Task 4: The streaming facade + two-pass worker [machine-verified here]

**Files:** create `src/listen/streaming.rs`; rewrite the worker in `src/listen/mod.rs`; delete `src/listen/vad.rs`

- [ ] **Step 1: facade.** `src/listen/streaming.rs`:

```rust
use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};

/// Streaming first-pass recognizer (Zipformer-20M transducer) with built-in
/// endpointing. Emits revising partials while speech is ongoing; `endpoint()`
/// fires on a pause (rule2: 0.8s trailing silence — the settle-on-pause feel)
/// or a 20s utterance cap (rule3 — bounds the pass-2 segment length).
pub struct Streaming {
    recognizer: OnlineRecognizer,
    stream: OnlineStream,
}

impl Streaming {
    pub fn new(encoder: &str, decoder: &str, joiner: &str, tokens: &str) -> Result<Streaming, String> {
        let mut cfg = OnlineRecognizerConfig::default();
        cfg.model_config.transducer.encoder = Some(encoder.to_string());
        cfg.model_config.transducer.decoder = Some(decoder.to_string());
        cfg.model_config.transducer.joiner = Some(joiner.to_string());
        cfg.model_config.tokens = Some(tokens.to_string());
        cfg.model_config.num_threads = 2;
        cfg.model_config.provider = Some("cpu".to_string());
        cfg.enable_endpoint = true;
        cfg.rule1_min_trailing_silence = 2.4;
        cfg.rule2_min_trailing_silence = 0.8;
        cfg.rule3_min_utterance_length = 20.0;
        let recognizer = OnlineRecognizer::create(&cfg)
            .ok_or_else(|| "failed to create streaming recognizer (check zipformer paths)".to_string())?;
        let stream = recognizer.create_stream();
        Ok(Streaming { recognizer, stream })
    }

    /// Feed a 16 kHz mono chunk and decode whatever is ready.
    pub fn push(&mut self, samples: &[f32]) {
        self.stream.accept_waveform(16_000, samples);
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
    }

    /// The current partial hypothesis, lowercased (the 20M export shouts).
    pub fn partial(&self) -> String {
        self.recognizer
            .get_result(&self.stream)
            .map(|r| r.text.to_lowercase())
            .unwrap_or_default()
    }

    pub fn endpoint(&self) -> bool {
        self.recognizer.is_endpoint(&self.stream)
    }

    /// Reset for the next utterance (after taking the endpointed text).
    pub fn reset(&mut self) {
        self.recognizer.reset(&self.stream);
    }
}
```

- [ ] **Step 2: worker rewrite** in `src/listen/mod.rs`. The worker owns `Streaming` + `Stt` (base) + a **segment sample buffer**: every resampled chunk is appended to `seg_buf` after feeding `Streaming`. Loop body (replacing the Segmenter logic; capture/resample/channel scaffolding unchanged):

```rust
                    Ok(chunk) => {
                        let resampled = resample_to_16k(&chunk, cap_rate);
                        streaming.push(&resampled);
                        seg_buf.extend_from_slice(&resampled);

                        let partial = streaming.partial();
                        if partial != last_partial {
                            sp.store(true, Ordering::Relaxed);
                            last_partial = partial.clone();
                            last_change = std::time::Instant::now();
                            let _ = tx.send(Event::Partial(partial));
                        } else if last_change.elapsed() > std::time::Duration::from_millis(900) {
                            sp.store(false, Ordering::Relaxed);
                        }

                        if streaming.endpoint() {
                            let text1 = std::mem::take(&mut last_partial);
                            streaming.reset();
                            let segment = std::mem::take(&mut seg_buf);
                            if !text1.trim().is_empty() {
                                let _ = tx.send(Event::Commit(text1));
                                let text2 = stt::transcribe_chunked(&stt, &segment, 16_000);
                                if !text2.trim().is_empty() {
                                    let _ = tx.send(Event::Revise(text2));
                                }
                            }
                            let _ = tx.send(Event::Partial(String::new()));
                        }
                    }
```

The finish-flag flush mirrors it: take the current partial as a final `Commit` (if non-empty), pass-2 the remaining `seg_buf`, `Revise`, then `Done`. The `speaking` handle is now partial-activity-based (rule 7); `LiveSource::new` constructs `Streaming` + `Stt` instead of `Segmenter` + `Stt`. Delete `src/listen/vad.rs` and its `pub mod vad;`. Keep `resample_to_16k` + its tests.

Note: `seg_buf` includes leading silence (harmless to Moonshine) and is bounded by rule3's 20s cap + `transcribe_chunked`. A `Revise` for an empty-pass-2 result is skipped (the streaming text stands).

- [ ] **Step 3: machine verification (this environment can run it).** Re-create the throwaway wav probe (pattern of Plan 5's de-risk probe, now against the REAL `LiveSource` internals or the facade directly): feed `/tmp/talk_long.wav` (43s pause-free) and the short two-sentence wav through `Streaming`+pass-2; expect: partials stream, endpoints fire (incl. the 20s cap), every committed segment gets a non-empty pass-2 revision. Then the acoustic-loopback live session (`script` pty + `afplay` + `TALK_BASE_DIR` temp, as in Plan 2): the written file's text must be the PASS-2 (mixed-case, punctuated) text, not the shouty streaming text. Delete throwaway probes after.
- [ ] **Step 4:** suites + clippy green → commit `feat(listen): streaming two-pass worker (zipformer partials → base revise)`.

---

## Task 5: Live loop + main wiring [CI + machine]

**Files:** `src/live.rs`, `src/main.rs`

- [ ] **Step 1: live.rs.** `Event::Partial(p)` in the drain arm: `settle.on_partial(&p)` (replacing the ignore; while paused, drop partials as today). The `Revise` arm from T2 Step 3 is already in place. The latch logic stays (driven by the `speaking` handle).
- [ ] **Step 2: main.rs.** `run_live_session` constructs `Streaming` (zipformer extracted paths) + `Stt` (base paths) → `LiveSource::new(capture, streaming, stt)`. Model-not-ready messages unchanged. `written_entry_count`/close flow unchanged.
- [ ] **Step 3:** full suites + clippy (3 feature sets) → commit `feat: live streaming session wiring`.

---

## Task 6: Privacy proofs + acceptance sweep [machine]

**Files:** `examples/ffi_probe.rs`, `tests/privacy.rs`, plan/spec notes

- [ ] **Step 1:** `ffi_probe` updated: loads `Streaming` + base `Stt` from the models dir, pushes synthesized audio through BOTH passes, prints ok — the FFI-under-deny-network-sandbox test now proves the full new inference stack makes zero outbound connections. Run it here for real (models present).
- [ ] **Step 2:** Spec delta note: append to the spec's §7 amendment trail — streaming live-jitter is now the default experience (user decision 2026-06-09); settle-on-pause retired; the settle machine, never-re-flow invariant, and §17 criteria unchanged.
- [ ] **Step 3:** Final sweep: full suites ×3 feature sets + clippy; the long-monologue probe (no data loss); acoustic loopback (file content = pass-2 text); `talk download verify` green against the new manifest. Commit `test: plan 5 verification sweep`.

---

## Self-Review (completed during authoring)

- **Coverage vs the user's asks:** live transcription while talking (T4 partials + T1 edge rendering) · retroactive correction with more context (T4 pass-2 + T1 `revise_committing` + T2 `Revise`) · accuracy (base everywhere pass-2 runs; the live edge is explicitly a preview) · the long-monologue bug stays fixed (rule3 20s cap + `transcribe_chunked` unchanged).
- **Placeholder scan:** the only deferrals are the `PIN_AT_IMPL` extracted-file hashes (computed in T3 from the already-verified local extractions — same one-command pattern as every prior pin step). All code blocks are complete; the worker rewrite shows the full new loop body and names what's unchanged.
- **Type consistency:** `revise_committing(&str, &str) -> bool` consumed by session.rs/live.rs `Revise` arms · `Event::Revise(String)` added to the one `Event` enum (source.rs) used by both paths + `FakeTranscript` scripts it (T2 test) · `Streaming::{new, push, partial, endpoint, reset}` consumed only by the worker · `LiveSource::new(capture, streaming, stt)` signature change has exactly one caller (main.rs) · `EXTRACTED` replaces `MOONSHINE_EXTRACTED` (grep for the old name in main.rs + privacy tests — T3 Step 2/3 cover both).
- **What is deliberately NOT here:** no config flag for streaming-vs-settle (one path; the doc-review may challenge — recorded rationale: a mode matrix doubles every live-path test and the user chose streaming as THE experience) · no beam-search/hotwords tuning (greedy default measured fine) · pass-2 is synchronous in the serial worker (no thread pool — ordering is the correctness argument; revisit only if pass-2 latency ever exceeds inter-utterance gaps in practice) · tiny/silero are removed, not kept as fallbacks (downloads are cheap to re-run; `talk download models` migrates users in one command).
- **Risks named:** users upgrading from v1.0 have stale tiny/silero caches — `models_ready` fails against the new manifest and the fetch offer/`talk download models` heals it (the old files are orphaned on disk, ~30MB; acceptable, note in the close-out) · the 20M streaming model's rough live text is visible product surface (dim + lowercase mitigates; pass-2 lands within ~0.3–1.5s) · rule2=0.8s commits feel identical to today's settle-on-pause cadence.

---

## Execution Handoff

Subagent-driven (recommended) — T1→T2→T3 are CI-pure; T4→T6 verify on this machine (models already extracted in /tmp for pinning; the cache re-downloads via `talk download models`). Per the house flow: run `/ce-doc-review` on this plan before building.
