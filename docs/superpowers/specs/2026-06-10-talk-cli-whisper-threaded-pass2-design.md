# talk-cli — Whisper base.en Pass-2 Swap Design (v3)

**Status:** Approved design (2026-06-10), after a base.en spot-check collapsed the scope. Supersedes the v2 "threaded small.en" design (see Scope-collapse note). Feeds an implementation plan via `writing-plans`.

**Goal:** Replace the pass-2 transcription model (Moonshine base) with **Whisper base.en (int8)** — materially more accurate on connected speech and self-punctuating/self-casing — as a **drop-in swap on the existing serial worker**. No threading, no settle redesign.

**Origin:** User field-test feedback (2026-06-10): word-substitution errors ("sometimes"→"times", "oh well that worked"→"without that work", "edge cases"→"educations") that the deterministic post-processor provably cannot cause — they are Moonshine base ASR mishearings. A better pre-trained model is the lever (not fine-tuning, not a smarter text pass).

---

## Probe evidence (measured 2026-06-10, this machine, release build, int8 models)

| Model | w1 (1.3s) | w2 (6.9s) | w3 (4.3s) | Deployed | Output quality |
|---|---|---|---|---|---|
| Moonshine base (current) | 47 ms | 200 ms | 118 ms | 135 MB | bare — no punctuation/casing |
| **Whisper base.en** | **217 ms** | **622 ms** | **324 ms** | **~154 MB** | **punctuated + cased; = small.en here** |
| Whisper small.en | 834 ms | 2242 ms | 1154 ms | 357 MB | punctuated + cased |
| SenseVoice int8 | 96–443 ms | — | — | 226 MB | REJECTED (garbled English) |

**Conclusions that shape this design:**
- On every probed phrase, **base.en produced output identical to small.en** — same words, same punctuation, same casing (e.g. `"...what Whisper does with their product. All these edge cases get sorted out..."`). It self-punctuates and self-cases, which Moonshine does not.
- **base.en latency (0.2–0.6s) fits the existing serial worker.** It is *faster* than the old Moonshine pass-2 budget of 0.3–1.5s, whose during-pass-2 freeze was already deemed imperceptible (it coincides with the user's pause). So **no threading, no multi-pending settle, no block-ids** — the Plan-5 architecture (serial worker → single committing block → settle on pass-2) carries over unchanged with the model swapped.
- **Footprint barely moves:** ~135 MB → ~154 MB deployed (+19 MB), vs +222 MB for small.en. The "lightweight local" positioning is preserved.
- The accuracy ceiling vs small.en on *hard real-voice* audio is the one open question (these were clean TTS) — settled by validation on the built tool (below), with small.en as a documented escalation if base.en disappoints.

**Whisper base.en int8 files:** encoder 28 MB + decoder 125 MB + tokens; archive `sherpa-onnx-whisper-base.en.tar.bz2` 199 MB (bundles fp32+int8).

---

## Design

This is a **model swap on proven architecture**, comparable in scope to the cleanup / startup / resampler fixes — not a redesign. The serial worker, the settle machine (`committing: Option<Block>`, `revise_committing`), `render_model`, `live.rs`, `source.rs`, and `session.rs` are **unchanged**. Only the pass-2 model, its envelope, the formatter's trust of Whisper output, and the manifest change.

### Pass-2 model (`src/listen/stt.rs`)

`Stt` becomes a Whisper wrapper: `OfflineRecognizerConfig` with `model_config.whisper.{encoder,decoder,language="en",task="transcribe"}` + `model_config.tokens`, int8 encoder/decoder. (sherpa API confirmed present; `OfflineRecognizer` is `Send+Sync`, so it moves to the worker exactly as Moonshine's `Stt` does today.)

**Envelope:** drop the 8s `transcribe_chunked` splitting. Whisper takes one `accept_waveform`+`decode` per segment up to a 30s window; segments are ≤20s by the rule3 cap (assert/measure the 20s ceiling rather than assume). A defensive guard splits any segment >30s. The worker's existing endpoint→commit→pass-2→`Revise` flow is otherwise unchanged.

**Hallucination guard (carried from doc-review):** Whisper hallucinates text on silence/non-speech, most threatening the quiet-speech rescue path (pass-1 empty → Whisper over near-silence). Keep the existing `plausibly_speech` energy/duration gate before a rescue; if sherpa exposes `no_speech_prob`/segment confidence, threshold on it; and a cheap sanity check (reject single-token-repeated or implausibly-long-for-duration output). A suspected hallucination on the rescue path → drop (no commit), matching today's empty-result behavior.

### Formatter (`crates/talk-core/src/cleanup.rs`)

- **`--from-text` (no-model) path keeps `deterministic_light` unchanged** — raw typed input still needs capitalize/terminate.
- **The pass-2 (Whisper) output path is thinned:** keep spoken commands (`new line`, `period`) + `scratch that` backtrack; drop the now-redundant force-capitalize-sentence-starts and force-terminal-punctuation (Whisper does them, respecting real sentence boundaries). Migrate the affected `format.rs`/`eval.rs` test expectations (e.g. `faithful_output_passes_the_guard_unchanged`) — an explicit plan task.
- **Continuation de-capitalizer (conservative):** reduce the mid-sentence-capital artifact (a sentence split by a pause → "just All these") by lowercasing a Whisper block's first letter **only when** (a) the previous block's text doesn't end in `.!?`, AND (b) the first word is in a small allow-list of continuation function words (`and, but, so, or, the, a, an, it, that, this, these, those, all, then, because, which, who`). Never lowercases an arbitrary capitalized token (protects proper nouns). Honest limit: misses continuations whose first word isn't in the allow-list; relies on Whisper's per-segment punctuation. Reduces, doesn't eliminate, the artifact. Independently tested.

### Manifest + fetch (`src/download/models.rs`, `src/main.rs`)

- **Download source:** the official GitHub archive `sherpa-onnx-whisper-base.en.tar.bz2` (199 MB fp32+int8); keep the int8 encoder/decoder/tokens after extraction. Preserves the existing archive→extract→verify→heal machinery (`models_ready` extracted-first, unchanged).
- **Corroboration (two-source preserved):** GitHub's whisper asset digest is likely null (like zipformer); corroborate the int8 **extracted files** against the independent `csukuangfj` Hugging Face mirror's LFS sha256s. Download channel (GitHub) ≠ corroboration channel (HF).
- Manifest: keep zipformer pins; replace Moonshine pins with Whisper base.en int8 archive + extracted pins; drop Moonshine pins. Disclosure/offer copy: download ~239 MB → ~327 MB (zipformer 128 + whisper-base archive 199); deployed ~263 MB → ~282 MB.

### Privacy tests (`tests/privacy.rs`, `examples/ffi_probe.rs`)

Retarget the no-egress sandbox proof and the tamper/heal tests from Moonshine to Whisper base.en paths (the Moonshine archive/extracted names are hardcoded; otherwise the no-egress proof is silently voided after the swap). `ffi_probe` loads Whisper encoder+decoder+tokens and runs the Whisper transcribe path.

### Unchanged (explicitly)

Serial worker structure; `settle.rs` single committing block + `revise_committing` + `committing_revised`; `render_model.rs` dim/bright committing block; `live.rs` `run_loop`/`drain_until_done`/`EventGuards`/`commit_dropped` pause privacy; `source.rs` `Event` enum; `session.rs`; the resampler; the pause epoch. The dim→bright "settle on pass-2" texture (spec §7 + Plan-5 amendment) holds as-is — base.en's sub-second latency keeps it snappy.

---

## Pre-build validation (carried from doc-review)

- **V1 (accuracy go/no-go):** the bake-off was clean TTS (both base.en and small.en got the tricky words right; only "oh well that worked" was reproduced as a real fix). The first acoustic loopback of the built tool, on real voice, is the go/no-go: does base.en fix the reported errors? If it does not and small.en would, escalate the model (small.en, which then reintroduces the threaded design — kept on file as v2 in git history).
- Recorded tradeoff: base.en's accuracy *ceiling* vs small.en on hard audio is unverified; base.en is chosen for the dramatically better footprint/latency given equal output on everything measured.

---

## Error handling / failure modes

- **Whisper load failure** → clear stderr + non-zero exit before the mic opens (existing pattern; models load pre-mic).
- **Empty Whisper result** → no commit / drop, exactly as Moonshine's empty result is handled today.
- **Hallucination on silence** → guarded on the rescue path (above).
- **Segment >30s** (shouldn't occur under rule3=20s) → defensive 30s split.

---

## Testing strategy

**Unit-testable without hardware:**
- Thin-formatter: spoken commands + backtrack apply on Whisper-style mixed-case input; force-cap/force-terminal removed from the Whisper path; `--from-text` path still capitalizes/terminates.
- Continuation de-capitalizer: lowercases an allow-list continuation after an unterminated prior block; keeps a proper-noun / non-allow-list first word; keeps after a terminated prior block.
- Manifest: the existing extracted-first / tamper / heal tests, retargeted to Whisper pins (incl. the happy-path-skips-archives test).

**Machine-verified only (acknowledged):**
- Whisper FFI load + accuracy + latency; the rule3 ≤20s ceiling.
- No-egress proof for the Whisper stack under the deny-network sandbox (retargeted `ffi_probe` + privacy tests).
- Live feel: the dim→bright settle still feels snappy with base.en (acoustic loopback).
- V1 accuracy go/no-go on real voice.

---

## Costs and tradeoffs (stated plainly)

- **Footprint:** deployed ~263 MB → ~282 MB (+19 MB; zipformer 128 + Whisper base.en ~154, Moonshine dropped). Download ~239 MB → ~327 MB.
- **Latency:** pass-2 0.2–0.6s (vs Moonshine 0.07–0.4s) — slightly higher, well within the serial worker's existing budget; no perceptible change to the settle feel.
- **Startup:** Whisper base.en load is comparable to Moonshine (~0.6–1s).

**Wins:** the accuracy the user is after; punctuation + casing for free; the mid-sentence-capital artifact reduced; **none of the v2 threading complexity or its risks**; the lightweight-local positioning preserved.

---

## Scope-collapse note (why v3 supersedes v2)

v2 designed an asynchronous, two-thread, multi-pending-block architecture to hide Whisper **small.en's** 2–4s latency — and the ce-doc-review (7 personas) flagged a P0 (empty-block ordering) and a cluster of P1s (unbounded finish-wait, block-id lifetime, id-routing blast radius) all *intrinsic to that async complexity*. The product-lens and scope-guardian both pushed to measure the lighter rung first. That spot-check showed **base.en equals small.en's output at ~43% of the size and ~27% of the latency**, and at a latency the *serial* worker already handles. So the entire async redesign — and every risk it introduced — is unnecessary. v3 is the minimal change that delivers the accuracy goal. v2 remains in git history as the escalation path if base.en's real-voice accuracy proves insufficient (V1).

---

## Out of scope / future

- small.en (threaded, v2 design) — escalation only if V1 fails.
- Cross-segment context for perfect sentence casing — the conservative de-capitalizer is the pragmatic substitute.
- Streaming hash in `download::fetch` to avoid buffering the 199 MB archive in memory (minor; under the 1 GiB cap).
