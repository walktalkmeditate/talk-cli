# talk-cli — Threaded Whisper Pass-2 Design

**Status:** Approved design (2026-06-10). Feeds an implementation plan via `writing-plans`.

**Goal:** Replace the pass-2 transcription model (Moonshine base) with **Whisper small.en**, which is materially more accurate on connected speech and emits punctuation/casing itself — and run it on its own thread so the live edge never freezes despite Whisper's higher latency.

**Origin:** User field-test feedback (2026-06-10). The deterministic post-processor was proven incapable of the reported word-substitution errors ("sometimes"→"times", "oh well that worked"→"without that work", "edge cases"→"educations"); those are Moonshine base ASR mishearings. A better pre-trained model — not fine-tuning, not a smarter text pass — is the lever.

---

## Probe evidence (measured 2026-06-09/10, this machine, release build)

Three-way bake-off, Moonshine base vs Whisper small.en int8 vs SenseVoice int8, on synthesized phrases at 16 kHz:

| Model | w1 (1.3s) | w2 (6.9s) | w3 (4.3s) | Notes |
|---|---|---|---|---|
| Moonshine base | 74 ms | 428 ms | 354 ms | length-proportional; no punctuation/casing |
| **Whisper small.en int8** | 1809 ms | 3842 ms | 2415 ms | **adds punctuation + casing**; fixed ~30s-window cost |
| SenseVoice int8 | 96 ms | 443 ms | 289 ms | **REJECTED** — garbled English ("IHINKAT'S PROBABL…"), all-caps |

**Conclusions baked into this design:**
- Whisper small.en is the only probed model with the accuracy we want; it also self-punctuates and self-cases (e.g. `"...their product. All these edge cases get sorted out..."` with proper sentence breaks and a capitalized proper noun "Whisper").
- Whisper pays a **fixed ~30-second-window cost** regardless of segment length: even a 1.3s phrase takes ~1.8s; a 6.9s phrase ~3.8s. This is 5–25× Moonshine. It is the core constraint this design works around.
- SenseVoice's speed was attractive but its English accuracy is unusable; it is not pursued.
- Model load: Whisper small.en int8 ~1.5s (vs Moonshine ~0.6s) — a one-time startup cost.
- Deployed size: Whisper small.en int8 ≈ 357 MB (encoder 107 MB + decoder 250 MB + tokens) vs Moonshine base 135 MB.

---

## Design decisions (resolved during brainstorming)

1. **Settle model — stack dim, brighten in order.** Because a Whisper revise lands 2–4s after a phrase commits, several just-finished phrases can be awaiting their pass-2 at once. Each committed phrase renders **dim** (provisional) until *its* Whisper revise lands, then brightens to **bright** (final). Multiple dim phrases may stack and brighten top-down. **The existing invariant holds: bright text is final and never moves.** (This extends spec §7's "settled never re-flows / bright never moves" — see Amendment below.)

2. **Two models, drop Moonshine.** Pipeline is `zipformer streaming (live edge, dim) → Whisper small.en (final, bright)`. Moonshine base is removed entirely (the rescue path uses Whisper too). No intermediate fast revise.

3. **Trust Whisper's formatting; thin deterministic layer.** Whisper's casing/punctuation becomes the output. Drop the now-redundant force-capitalize-sentence-starts and force-terminal-punctuation steps. Keep spoken commands (`new line`, `period`, …) and `scratch that` backtrack as features. Add a **continuation de-capitalizer**: lowercase a phrase's first letter when the previous block did not end in `.!?` and the first word isn't "I" — so a sentence split by a pause doesn't get a mid-sentence capital. *Caveat:* this reduces but may not fully eliminate the mid-sentence-capital case, since it relies on Whisper's own punctuation of the prior segment as the continuation signal.

4. **Async architecture — single dedicated pass-2 thread (Approach A).** Rejected: a Whisper thread-pool (each loaded model ~357 MB RAM; out-of-order completion needs reordering) and keeping the serial worker (the freeze).

---

## Architecture

### Threading and data flow

```
 capture thread (cpal callback)
   └─► sync_channel(1024) ─► PASS-1 WORKER THREAD
                               • resample → zipformer.push (live edge, RTF 0.01)
                               • emit Partial(text) on change
                               • on endpoint: assign block id N, emit Commit(N, streaming_text)
                                 and push (N, segment_audio) onto the pass-2 FIFO queue
                               • never blocks on Whisper
                                     │
                            pass-2 FIFO queue (bounded)
                                     ▼
                            PASS-2 WORKER THREAD
                               • pop (N, audio) in order
                               • Whisper.transcribe(audio)  (~2–4s)
                               • emit Revise(N, whisper_text)
                                     │
                              results channel ─► UI LOOP (run_loop)
                                     • Commit(N) → settle.commit(N, …)  [dim]
                                     • Revise(N) → settle.revise(N, …)  [→ bright, in order]
                                     • Partial    → settle.on_partial
```

Pass-1 runs in realtime and is never blocked by Whisper, so the live edge stays fluid and the capture channel never backs up. Whisper runs entirely on the pass-2 thread; its latency only delays the dim→bright brighten, never the live edge.

### Settle machine (`crates/talk-core/src/settle.rs`)

Generalize the single `committing: Option<Block>` to an ordered list of pending blocks, each carrying an id:

- `settled: Vec<Block>` — bright, final, immutable (unchanged semantics).
- `pending: Vec<PendingBlock>` where `PendingBlock { id, block }` — dim, awaiting Whisper.
- `live: String` — the streaming edge (unchanged).

Operations:
- `commit(id, raw, clean)` — append `{id, block}` to `pending` (renders dim). Clears `live`.
- `revise(id, raw, clean)` — find the pending block with `id`; replace its text and move it from `pending` → `settled`, preserving order (front-to-back). Returns false (no-op) if no block with that id exists — which is exactly what makes pause-dropped commits safe (see Pause).
- `finalize()` — at session end, promote any still-pending blocks to settled with their current (rough) text. (Normal finish drains the queue first; see Finish.)

Because the pass-2 queue is FIFO and the pass-2 thread is serial, revises arrive in commit order; the front pending block is always the next to settle, so blocks brighten top-down.

### Rendering (`crates/talk-core/src/render_model.rs`)

Order: `settled` blocks (bright) → `pending` blocks (dim) → live edge. The existing dim/bright tones (`LineKind::Settled` / `LineKind::Edge`) carry over: settled = bright, each pending block = dim. The single-committing-block special case is replaced by iterating `pending`.

### Worker (`src/listen/mod.rs`)

Split the current single worker:
- **Pass-1 thread** keeps capture-drain, resample, `streaming.push`, partial emission, endpoint detection. On endpoint it allocates a monotonically increasing block id, emits `Commit(id, streaming_text)`, and pushes `(id, segment_audio)` to the pass-2 queue. The quiet-speech rescue (empty pass-1 over `plausibly_speech`) still enqueues a segment (with a fresh id) so Whisper can rescue it.
- **Pass-2 thread** owns the `Whisper` recognizer; pops `(id, audio)` from the FIFO queue, transcribes, emits `Revise(id, text)`. On empty Whisper output it emits nothing (the dim block stays until finalize) — or, for parity with today's rescue, emits `Revise(id, "")` to drop the block; decided in the plan.
- The pass-2 queue is a bounded channel; if Whisper falls behind a fast talker, the queue (and the dim stack) grows. Endpoints fire on pauses, so this is naturally bounded by speech cadence; a cap + drop-oldest policy is a plan-level decision with a logged warning if it ever triggers.

### Model + envelope (`src/listen/stt.rs`, `src/download/models.rs`, `src/main.rs`)

- `Stt` becomes a Whisper wrapper: `OfflineRecognizerConfig` with `model_config.whisper.{encoder,decoder,language="en",task="transcribe"}` + `model_config.tokens`, int8 encoder/decoder.
- The 8s `transcribe_chunked` envelope is removed for Whisper. Whisper takes one window up to 30s; segments are ≤20s by the rule3 cap, so each is a single Whisper call. A defensive guard handles any segment >30s (split at 30s), though rule3 should prevent it.
- Manifest: keep the streaming zipformer (pass-1); replace the Moonshine base pins with Whisper small.en **int8** pins (encoder + decoder + tokens), hash-corroborated like the others. Drop the Moonshine pins.
- **Download-source decision (for the plan):** the official GitHub archive is 606 MB because it bundles fp32 + int8; we only need int8 (~357 MB extracted). Two options, plan picks one: **(a)** fetch the 606 MB archive and keep only the int8 files — simplest, matches the existing archive+extract+verify pattern, but a large download; **(b)** fetch the individual int8 files (~357 MB) from a hash-corroborated mirror (e.g. the `csukuangfj` Hugging Face repo), pinning files directly without an archive — smaller download, but a new source to corroborate and a manifest that pins files rather than an archive. The download figure below spans both.

### Pause (privacy)

Pause stays off-record at the audio level (worker-side `PauseSignal` epoch reset of the resampler/streaming/seg_buf, established in the prior fix). New: because revises route by **block id**, a Commit dropped during pause never creates a pending block, so its later `Revise(id)` finds no match and no-ops. This replaces the `commit_dropped` pairing flag — the id lookup is the guard. Segments already enqueued before a pause are on-record and complete normally.

### Finish (`[space]`)

`drain_until_done` waits for the pass-2 queue to drain — i.e., until every pending block has been revised (or the worker signals the queue empty) — so the written file is all Whisper-quality, not rough dim text. The "settling…" frame covers the wait. Typical pending depth at finish is 0–2 (≈0–8s); a fast monologue can be longer. A liveness deadline still bounds a genuinely hung Whisper pass. If the deadline fires with blocks still pending, `finalize()` writes their rough text rather than hanging.

---

## Error handling / failure modes

- **Whisper load failure** at startup → clear stderr message + non-zero exit (same pattern as today's model-load failures), before the mic opens.
- **Whisper FFI panic / hang mid-session** → the pass-2 thread is isolated; the UI loop's finish deadline still terminates, and `LiveSource::drop` joins with a grace-then-detach (established) so the terminal never freezes.
- **Pass-2 queue backpressure** (fast talker) → bounded queue; dim stack grows; if a cap is hit, drop-oldest with a logged warning (no audio loss on pass-1, which is realtime).
- **Empty Whisper result** over a real segment → dim block resolved at finalize (or dropped via `Revise(id,"")`), never left dangling.
- **Segment >30s** (shouldn't occur under rule3=20s) → split defensively at 30s.

---

## Testing strategy

**Unit-testable without hardware (talk-core + pure logic):**
- Settle multi-pending: commit(1), commit(2), revise out-of-arrival-safety, revise(1) brightens block 1 and moves it to settled while block 2 stays dim; revise(2) then settles 2; bright blocks never mutate.
- Pause-drop via id: a `Revise(id)` for an id that was never committed is a no-op (privacy).
- Continuation de-capitalizer: previous block ends without `.!?` → next block's first letter lowercased; ends with `.` → kept; first word "I" → kept.
- Formatter thin-layer: spoken commands + backtrack still apply on Whisper-style mixed-case input; redundant re-capitalization removed.
- Finish drain ordering: scripted source delivering Commit(1),Commit(2),Revise(1),Revise(2),Done settles both in order.

**Machine-verified only (acknowledged):**
- Whisper FFI load + transcription quality + per-call latency (probe-style).
- Live feel: the dim-stack brighten cadence, finish-wait, no live-edge freeze (acoustic loopback).
- No-egress proof for the new model under the deny-network sandbox (extend the existing privacy test to the Whisper stack).

---

## Costs and tradeoffs (stated plainly)

- **Footprint:** deployed model size ~263 MB → ~485 MB (zipformer 128 + Whisper 357). Download grows from ~239 MB to **~360 MB (int8-only mirror)** or **~600 MB (official fp32+int8 archive)** depending on the source decision above. Disclosure/offer copy updated honestly to whichever is chosen.
- **Latency:** each phrase brightens 2–4s after committing (inherent Whisper cost). The live edge stays instant.
- **Finish-wait:** proportional to pending phrases (typically a few seconds).
- **Startup:** +~1s model load (Whisper vs Moonshine).

**Wins:** the accuracy the user is after; punctuation + casing for free; the mid-sentence-capital artifact largely eliminated; the snappy live edge preserved; simpler pause privacy (id-routed); the deterministic formatter shrinks.

---

## Spec §7 amendment

The main design (`2026-06-08-talk-cli-design.md` §7) and its Plan-5 amendment described a single committing block that brightens on its pass-2. This design generalizes that to **a FIFO list of dim pending blocks, each brightening to bright when its asynchronous Whisper pass lands, in commit order.** The core texture is unchanged and reaffirmed: **bright (settled) text is final and never moves; only dim (pending) blocks and the live edge change.** The dim region may now hold more than one phrase at a time.

---

## Out of scope / future

- Cross-segment context for perfect sentence casing (would require feeding Whisper a rolling window with overlap-dedup). The continuation de-capitalizer is the pragmatic substitute.
- Larger/smaller Whisper variants or distil-whisper as size/latency knobs.
- Parallel Whisper workers for faster catch-up (memory-prohibitive at this model size).
- An optional intermediate fast revise (Moonshine) for nicer dim text — deliberately omitted (YAGNI).
