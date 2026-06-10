# talk-cli — Threaded Whisper Pass-2 Design (v2)

**Status:** Approved design, hardened after ce-doc-review (7 personas, 2026-06-10). Feeds an implementation plan via `writing-plans`.

**Goal:** Replace the pass-2 transcription model (Moonshine base) with **Whisper small.en**, which is materially more accurate on connected speech and emits punctuation/casing itself — and run it on its own thread so the live edge never freezes despite Whisper's higher latency.

**Origin:** User field-test feedback (2026-06-10). The deterministic post-processor was proven incapable of the reported word-substitution errors ("sometimes"→"times", "oh well that worked"→"without that work", "edge cases"→"educations"); those are Moonshine base ASR mishearings. A better pre-trained model — not fine-tuning, not a smarter text pass — is the lever.

---

## Probe evidence (measured 2026-06-09/10, this machine, release build)

Three-way bake-off, Moonshine base vs Whisper small.en int8 vs SenseVoice int8, on synthesized phrases at 16 kHz:

| Model | w1 (1.3s) | w2 (6.9s) | w3 (4.3s) | Notes |
|---|---|---|---|---|
| Moonshine base | 74 ms | 428 ms | 354 ms | length-proportional; **already casing-bare/punctuation-light** |
| **Whisper small.en int8** | 1809 ms | 3842 ms | 2415 ms | adds punctuation + casing; fixed ~30s-window cost |
| SenseVoice int8 | 96 ms | 443 ms | 289 ms | **REJECTED** — garbled English, all-caps |

**Conclusions baked into this design:**
- Whisper small.en is the only probed model with the accuracy we want; it self-punctuates and self-cases.
- Whisper pays a **fixed ~30-second-window cost** regardless of segment length (1.3s phrase → ~1.8s; 6.9s → ~3.8s), 5–25× Moonshine. This drives the entire async design.
- SenseVoice's English accuracy is unusable.
- Whisper small.en int8: load ~1.5s; deployed ≈ 357 MB (encoder 107 + decoder 250 + tokens) vs Moonshine base 135 MB.

**PRE-BUILD VALIDATION (review-mandated, do before/early in implementation):**
- **V1.** Re-confirm Whisper small.en actually fixes the *reported* errors. The bake-off used synthesized audio (both models got the tricky words right on clean TTS); only "oh well that worked" was reproduced. Verify on real voice — either the user records the failing phrases, or the first acoustic loopback of the built tool is treated as a go/no-go on accuracy.
- **V2.** Spot-check Whisper **base.en** (≈74 MB archive / ~150 MB int8, roughly half the window cost) on the same audio. The choice of small.en over base.en was asserted ("only marginally better than Moonshine"), never measured. If base.en is "good enough," it materially lightens footprint and latency. small.en remains the default; base.en is the documented fallback knob.

---

## Design decisions (resolved during brainstorming)

1. **Settle model — stack dim, brighten in order.** A Whisper revise lands 2–4s after a phrase commits, so several just-finished phrases can be awaiting pass-2 at once. Each renders **dim** (provisional) until *its* revise lands, then brightens to **bright** (final). Multiple dim phrases may stack and brighten **front-to-back**. **Invariant preserved: bright (settled) text is final and never moves.**

2. **Two models, drop Moonshine.** Pipeline: `zipformer streaming (live edge, dim) → Whisper small.en (final, bright)`. Moonshine removed entirely (rescue path uses Whisper). No intermediate fast revise.

3. **Trust Whisper's formatting; thin layer.** Whisper's casing/punctuation is the pass-2 output. The Whisper-output path drops the now-redundant force-capitalize-sentence-starts and force-terminal-punctuation; keeps spoken commands and `scratch that`. See §Formatter for the (deliberately conservative) continuation de-capitalizer and the separation from the `--from-text` path.

4. **Async architecture — single dedicated pass-2 thread (Approach A).** Rejected: Whisper thread-pool (memory) and serial-with-freeze.

---

## Positioning note (product-lens, recorded)

This design trades some of v1's "settle into stillness, instantly" texture for accuracy and polish: the user will see one or more dim provisional phrases linger 2–4s and brighten Wispr-style, and the close has a short settling wait. The user has knowingly accepted this tradeoff (chose Whisper after seeing the latency and the dim-stack preview). Footprint also nearly doubles (~263 MB → ~485 MB deployed), re-inflating what Plan 3 deliberately shed — accepted for the accuracy win. Recorded here so it is an explicit bet, not an accident.

---

## Architecture

### Threading and data flow

```
 capture thread (cpal) ─► sync_channel(1024) ─► PASS-1 WORKER THREAD
                                                  • resample → zipformer.push (live edge, RTF 0.01)
                                                  • Partial(text) on change
                                                  • on endpoint: id = next_id();
                                                    Commit(id, streaming_text);
                                                    push (id, segment_audio) to pass-2 queue
                                                  • on pause epoch: drop audio, reset streaming/
                                                    resampler/seg_buf — id counter is NOT reset
                                                  • never blocks on Whisper
                                                        │  pass-2 queue (FIFO, serial consumer)
                                                        ▼
                                                  PASS-2 WORKER THREAD (owns Whisper)
                                                  • pop (id, audio) in order
                                                  • Whisper.transcribe(audio)  (~2–4s)
                                                  • ALWAYS emit exactly one resolution:
                                                      Revise(id, Some(text))  if non-empty
                                                      Revise(id, None)        if empty  (drop block)
                                                        │  results channel
                                                        ▼
                                                  UI LOOP (run_loop)
                                                  • Commit(id) → settle.commit(id, …)   [dim]
                                                  • Revise(id, Some) → settle.revise(id,…) [→ bright]
                                                  • Revise(id, None) → settle.drop(id)
                                                  • Partial → settle.on_partial
```

Pass-1 runs in realtime and never blocks on Whisper → the live edge stays fluid and the capture channel never backs up. The pass-2 thread is **serial and FIFO**, so revises are *emitted* in commit order; no out-of-order completion is possible (this is what makes "brighten front-to-back" hold without reorder logic).

### Settle machine (`crates/talk-core/src/settle.rs`)

Replace the single `committing: Option<Block>` with an ordered list keyed by id:
- `settled: Vec<Block>` — bright, final, immutable (unchanged semantics).
- `pending: VecDeque<PendingBlock>` where `PendingBlock { id: u64, block: Block }` — dim, awaiting Whisper, in commit order.
- `live: String` — the streaming edge.

Operations:
- `commit(id, raw, clean)` — push `{id, block}` to the back of `pending` (dim). Clear `live`.
- `revise(id, raw, clean)` — find the pending block with `id`, replace its text, and move it (and any now-resolved blocks ahead of it, though FIFO makes it the front) from `pending` → `settled`, preserving order. No-op if no block has that id.
- `drop(id)` — remove the pending block with that id (used for empty Whisper / rescue-miss). No-op if absent.
- `drain_pending()` — **new name** (NOT `finalize`): at session end, promote any still-pending blocks to settled with their current text, flagged as rough (see Finish). The existing `finalize()` keeps its current per-block meaning and its live.rs call sites are unaffected.

**Resolution contract (resolves the P0):** every enqueued segment produces *exactly one* resolution event — `Revise(id, Some)` or `Revise(id, None)` — never "nothing." So no pending block dangles, and because the pass-2 thread is serial the front always resolves first. This is the ordering guarantee.

### Block ids (privacy-load-bearing)

`id` is a **session-lifetime monotonic `u64`** allocated only by pass-1's commit. The pause epoch reset (worker-side `streaming.reset()` / `seg_buf.clear()` / `resampler.reset()`) **must not** touch the id counter. Ids are never reused, so a stale or off-record `Revise(id)` can never collide with a live block. This replaces the `commit_dropped` pairing flag: a `Revise(id)` for a block that was never committed simply no-ops.

### Rendering (`crates/talk-core/src/render_model.rs`)

Order: `settled` (bright) → `pending` (dim, in order) → live edge. Interaction states (design-lens):
- **Brighten = instant palette swap, no animation** (a block moves pending→settled; the render loop carries no transient per-block state).
- **Visible dim cap:** show at most `N_DIM_VISIBLE` (e.g. 3) most-recent pending phrases; if more accumulate, collapse the overflow into a single dim line `… N more settling` at the top of the pending region, so the frame can't grow unbounded or bounce.
- **Layout shift on brighten:** Whisper text replacing the rough text may change a block's line count; the live edge sits below and may shift once when a block brightens. This one-time shift is accepted as deliberate (settled text above never re-flows; only the boundary moves). Documented, not animated.
- **Raw toggle (`u`)** on a pending block shows its rough pass-1 (zipformer) text; on a settled block, the stored Whisper raw. The existing raw-toggle test updates accordingly.

### Worker (`src/listen/mod.rs`)

Split the current single worker:
- **Pass-1 thread:** capture-drain, resample, `streaming.push`, partial emission, endpoint detection. On endpoint: allocate `id`, emit `Commit(id, streaming_text)`, push `(id, segment_audio)`. The quiet-speech rescue (empty pass-1 over `plausibly_speech`) still enqueues a segment with a fresh id (Whisper rescues; if Whisper is also empty → `Revise(id, None)` drops the block).
- **Pass-2 thread:** owns the `Whisper` recognizer; pops `(id, audio)` FIFO, transcribes, emits exactly one resolution. **Loaded before the mic opens** (same ordering as today — all models load pre-mic, so no segment queues against an unloaded recognizer).
- **Queue:** an unbounded (or generously bounded) channel. No drop-oldest policy — dropping a pending phrase would silently delete it from the final file, and the "naturally bounded by pauses" claim does not hold for a fast reader (arrival ~0.8s endpoints vs ~1.8–3.8s service). Instead the dim-stack visible cap (above) bounds the *display*, and the queue simply holds the backlog (a few segments of audio, each ≤20s ≈ 1.3 MB). If real backlog pressure is ever observed, capping is a follow-up — explicitly *not* in scope now (scope-guardian).

### Model + envelope (`src/listen/stt.rs`, `src/download/models.rs`, `src/main.rs`)

- `Stt` becomes a Whisper wrapper: `OfflineRecognizerConfig` with `model_config.whisper.{encoder,decoder,language="en",task="transcribe"}` + `model_config.tokens`, int8 encoder/decoder. (sherpa API confirmed present; `OfflineRecognizer` is `Send+Sync`.)
- The 8s `transcribe_chunked` envelope is removed for Whisper: one `accept_waveform`+`decode` per segment, up to a 30s window. Segments are ≤20s by the rule3 cap (the 20s ceiling is measured/asserted, not assumed). A defensive guard splits any segment >30s.
- **Hallucination guard (security-lens):** Whisper hallucinates text on silence/non-speech, which most threatens the quiet-speech rescue path (pass-1 empty → Whisper over near-silence). Mitigations, layered: keep the existing `plausibly_speech` energy/duration gate before enqueuing a rescue; if sherpa exposes `no_speech_prob` / segment confidence, threshold on it; and a cheap sanity check (reject output that is a single token repeated, or wildly long for the audio duration). A rescue producing suspected hallucination → `Revise(id, None)`.

### Pass-2 model fetch + manifest (download-source decision — RESOLVED to option (a))

- **Download source:** the official GitHub archive (`sherpa-onnx-whisper-small.en.tar.bz2`, 606 MB fp32+int8). Keep only the int8 encoder/decoder/tokens after extraction. This preserves the existing archive→extract→verify→heal machinery (option (b)'s file-only fetch would break `models_ready`'s `.tar.bz2` heal path and several privacy tests).
- **Corroboration (two-source preserved):** GitHub's whisper asset digest is likely null (like zipformer); corroborate the **int8 extracted files** against the independent `csukuangfj` Hugging Face mirror's LFS sha256s. Download channel (GitHub) ≠ corroboration channel (HF) — the two-source guarantee holds, unlike option (b).
- **Fetch RAM:** `download::fetch` buffers the whole body before hashing; the 606 MB archive is a ~606 MB transient spike (under the 1 GiB `read_capped`). Switching to a streaming hash is a recommended plan-level improvement to avoid the spike.
- Manifest: keep zipformer pins; replace Moonshine pins with Whisper small.en int8 archive + extracted pins. Drop Moonshine pins. Disclosure/offer copy updated to ~600 MB download / ~485 MB deployed.

### Pause (privacy) — clarified

Pause is off-record **at the audio level**: the worker's `PauseSignal` epoch reset drops in-flight audio and clears `streaming`/`seg_buf`/`resampler`, so paused speech never reaches an endpoint and therefore **never becomes a queued segment**. Consequently the pass-2 queue contains only on-record segments (committed before the pause); their Whisper passes run and their revises apply normally — correct, because those phrases were spoken on-record. There is no off-record audio in the queue to cancel. The id-no-op is the defensive backstop (a `Revise(id)` for a never-committed/ dropped block does nothing). This is *simpler and no weaker* than today's `commit_dropped`; recorded explicitly because the prior spec hand-waved it.

### Finish (`[space]`) — bounded + cancellable

`drain_until_done` waits for pending blocks to resolve so the file is Whisper-quality, but:
- It **polls for a cancel key**: the settling frame reads `settling N phrases… [esc to keep what's ready]`. Pending blocks brighten in place during the wait (visible progress).
- The liveness deadline resets on each resolution (progress), and there is a **total-wait cap**; on cap or esc, `drain_pending()` promotes the still-pending blocks using their rough pass-1 text, **marked as rough** so a deadline/cancel-degraded transcript is distinguishable from a clean one (provenance recorded in the raw sidecar; the close screen notes "some phrases saved before final pass"). This removes the "trapped for tens of seconds with no escape" and "silent rough text" holes.
- (Considered and deferred: async-finish — write rough immediately, refine in background, re-save when passes land. Cleaner contemplative close but adds file-rewrite complexity; revisit if the synchronous settling wait proves annoying.)

### Pause-with-pending (design-lens)

If the user pauses while blocks are still pending, those pre-pause on-record blocks continue brightening (their Whisper passes complete). The paused status line shows a secondary hint `⏸ paused · settling…` so an actively-resolving pause isn't mistaken for a hang.

---

## Formatter (`crates/talk-core/src/cleanup.rs`) — scope clarified

- The **`--from-text` (no-model) path keeps `deterministic_light` unchanged** (it must still capitalize/terminate raw typed input).
- The **Whisper pass-2 output path uses a thinner formatting**: spoken commands + `scratch that` backtrack, but NOT force-capitalize/force-terminal (Whisper already did them, better). This is the part that needs the existing `format.rs`/`eval.rs` test expectations migrated (e.g. `faithful_output_passes_the_guard_unchanged`), tasked explicitly in the plan.
- **Continuation de-capitalizer (deliberately conservative):** to reduce the mid-sentence-capital artifact when a sentence spans a pause, lowercase a Whisper block's first letter **only when** (a) the previous block's text does not end in `.!?`, AND (b) the block's first word is in a small allow-list of common continuation function words (`and, but, so, or, the, a, an, it, that, this, these, those, all, then, because, which, who`). It never lowercases an arbitrary capitalized token, protecting proper nouns. Documented failure mode: it will *miss* a continuation whose first word isn't in the allow-list (false-keep), and it relies on Whisper's per-segment punctuation. This is honestly partial; the artifact is reduced, not eliminated. (Scope-guardian/product-lens flagged this as separable and fragile; kept because the user wants the artifact addressed, but bounded to the safe cases and independently tested.)

---

## Touched files / migration order (feasibility — was missing)

The block-id change ripples through the `Event` seam, so the plan must sequence a single compiling migration:
1. `src/source.rs` — `Event::Commit`/`Event::Revise` gain an `id: u64` (and Revise carries `Option<String>` for the drop case); update `FakeTranscript` + all match arms.
2. `crates/talk-core/src/settle.rs` — the `pending` VecDeque + `commit(id)`/`revise(id)`/`drop(id)`/`drain_pending()` API.
3. `crates/talk-core/src/render_model.rs` — iterate `pending`; dim cap; raw-toggle.
4. `src/live.rs` — `apply_event` handles id-routed Commit/Revise/drop; `drain_until_done` cancel + rough-mark; the `EventGuards`/`commit_dropped` machinery is removed (id-routing replaces it).
5. `src/session.rs` — **second Event consumer**; migrate its `commit`/`revise` calls in the same step.
6. `src/listen/mod.rs` — two-thread split; id allocation; FIFO queue; pass-2 Whisper; rescue → resolution.
7. `src/listen/stt.rs` — Whisper wrapper; drop `transcribe_chunked`.
8. `src/download/models.rs`, `src/main.rs` — manifest swap (option a), `models_ready` unchanged, disclosure copy.
9. `tests/privacy.rs` + `examples/ffi_probe.rs` — retarget the no-egress + tamper tests to Whisper paths (the Moonshine archive/extracted names are hardcoded); the no-egress proof is otherwise silently voided.

---

## Error handling / failure modes

- **Whisper load failure** at startup → clear stderr + non-zero exit before the mic opens.
- **Whisper FFI panic/hang** → the pass-2 thread is isolated; the finish total-wait cap + `LiveSource::drop` grace-then-detach keep the terminal from freezing.
- **Empty Whisper result** → `Revise(id, None)` drops the block (rescue-miss) or, for a block that had pass-1 text, the resolution carries the rough text marked rough — never dangling.
- **Backlog (fast talker)** → display capped; queue holds the backlog; no silent drop.
- **Segment >30s** (shouldn't occur under rule3=20s) → split defensively at 30s.
- **Hallucination on silence** → guarded on the rescue path (above).

---

## Testing strategy

**Unit-testable without hardware:**
- Settle multi-pending: `commit(1)`,`commit(2)`,`revise(1)`→block 1 bright+settled while 2 stays dim, `revise(2)`→2 settles; bright blocks never mutate; **`revise(2)` before `revise(1)` does not promote 2 past 1** (front-first); `drop(id)` removes a pending block; `drain_pending` promotes rough-marked.
- Block-id privacy: `Revise(id)` for a never-committed id is a no-op; **id is not reused across a pause epoch** (commit(N), pause-reset, next commit gets N+1, a stale Revise(N) no-ops).
- Continuation de-capitalizer: lowercases an allow-list continuation after an unterminated prior block; keeps a proper-noun / non-allow-list word; keeps after a terminated prior block.
- Thin-formatter: spoken commands + backtrack apply on Whisper-style mixed-case input; force-cap/force-terminal removed; `--from-text` path still capitalizes/terminates.
- Render: multi-block dim pending brightening front-to-back; dim visible cap + overflow line.

**Machine-verified only (acknowledged):**
- Whisper FFI load + accuracy + per-call latency; the rule3 ≤20s ceiling.
- Live feel: dim-stack brighten cadence, no live-edge freeze, finish settling + esc-cancel (acoustic loopback).
- No-egress proof for the Whisper stack under the deny-network sandbox (retargeted `ffi_probe` + privacy tests).
- Accuracy go/no-go (V1) and the base.en spot-check (V2).

---

## Costs and tradeoffs (stated plainly)

- **Footprint (replacement, not additive):** current ~263 MB deployed (zipformer 128 + Moonshine 135) → new ~485 MB (zipformer 128 + Whisper 357, Moonshine dropped). Download ~239 MB → ~600 MB (official archive). Disclosure copy updated.
- **Latency:** each phrase brightens 2–4s after committing (inherent). Live edge stays instant.
- **Finish-wait:** proportional to pending phrases, now cancellable (esc) with a total-wait cap.
- **Startup:** +~1s model load.

**Wins:** the accuracy the user is after; punctuation + casing; the mid-sentence-capital artifact reduced; snappy live edge preserved; simpler pause privacy (id-routed); thinner formatter on the model path.

---

## Spec §7 amendment

The main design (§7) and its Plan-5 amendment described a *single* committing block brightening on its pass-2 (correct for serial Moonshine at ~0.3s). This design generalizes that to a **FIFO list of dim pending blocks, each brightening to bright when its asynchronous Whisper pass lands, in commit order.** Reaffirmed: **bright (settled) text is final and never moves; only dim pending blocks and the live edge change.** The dim region may now hold more than one phrase, capped in the display.

---

## Out of scope / future

- Cross-segment context for perfect sentence casing (rolling-window Whisper with overlap-dedup). The conservative de-capitalizer is the pragmatic substitute.
- base.en / distil-whisper as size/latency knobs (V2 informs whether to switch).
- Async-finish (background refine + re-save) instead of synchronous settling drain.
- Queue capping / drop-oldest if real backlog pressure ever appears.
- Parallel Whisper workers (memory: 2×357 MB ≈ 714 MB — size the constraint before reconsidering).
