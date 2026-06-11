---
title: talk-cli on-device formatter (SmolLM2 Medium/High cleanup + paragraphs)
date: 2026-06-11
status: design
origin: user request — journal entries are one wall of text; Medium/High cleanup never shipped (Candle formatter, Plan 3 T7, was never built)
---

# talk on-device formatter

**Goal:** Make the Medium and High cleanup levels real. Medium removes disfluencies
and false starts and joins fragments; High additionally breaks the entry into
paragraphs at topic shifts. Both run a small on-device LLM (SmolLM2-360M-Instruct)
once over the whole entry **after the session ends** — never touching the live edge
or the verbatim raw — behind a widened guard that forbids meaning change.

**Identity note:** the verbatim raw (`<!-- raw: -->`, the `u` toggle) remains the
canonical record of exactly what was said. The LLM reshapes only the written *clean*
body, and only for `journal` (High) by default; `reflect` stays Light, so "your exact
words" remains true for reflection out of the box. This is a deliberate, surfaced
identity decision (`reflect`=Light honors DREAMING.md's "preserve the ums"; `journal`,
a long-form artifact you re-read, earns paragraphs).

**Architecture:** Keep the entire live/commit pipeline deterministic and unchanged.
Run the LLM as a **post-session pass in the binary callers** (`run_live_session` for
the mic path, `run_and_report` for `--from-text`), on the joined *clean* text, **after
the alternate `Screen` has dropped**, when the level is Medium/High and the model is
present — behind a **deletions-only subsequence guard** with sound negation handling
and a deletion budget. Any failure falls back to the Light join. The LLM lives behind
the `Formatter` trait in a new `format` cargo feature; the model downloads lazily on
first need.

**Tech stack:** Rust, Candle (`candle-core` + `candle-transformers`), SmolLM2-360M-Instruct
GGUF (Q4_K_M default), behind a new `format` feature (default-on). Reuses `rewrite_prompt`
(already in `talk-core/src/cleanup.rs`) and the existing download/verify pipeline.

---

## Background

Cleanup has four levels (`talk-core::cleanup::Level`): `None / Light / Medium / High`.
Only **Light** is implemented (`deterministic_light`). Medium and High were designed
(Plan 3, T7) to run an on-device LLM via the `Formatter` trait, but that inference was
never built — there is no `src/format/`, and `main.rs` always injects
`DeterministicFormatter`, so every level collapses to Light. Two pieces of
infrastructure are reused:

- **Per-mode level selection.** `config.rs` has `cleanup_for(mode)`, which returns
  `journal_cleanup` for `"journal"` and `reflect_cleanup` for **every other mode**.
  So a single change — `journal_cleanup` default `"medium"` → `"high"` — gives
  `journal → High` while `reflect`/`thread`/own-question/`unburden` all map through the
  `else` branch to Light. (There is no per-mode table beyond journal-vs-else; the
  defaults are expressed entirely through that one branch.)
- **The guarded `Formatter` seam.** `guarded_format` (format.rs) enforces a guard with
  a deterministic-Light fallback per phrase; the `Flip` test proves a misbehaving
  formatter cannot corrupt output.

### The two paths and where the pass runs

There are two transcript pipelines, and the pass must serve both — but it runs in the
**binary callers, after the session loop**, not inside the loop:

- **Live mic (`src/live.rs::run_loop`, the journal=High case).** `run_loop` finalizes
  and returns `LiveResult { raw, clean, cancelled }` (the per-phrase Light join), then
  its `Screen` RAII guard drops on return. `run_live_session` (main.rs) then writes.
  **The document pass runs here, after `run_loop` returns** — so the alternate screen
  is gone and a `polishing…` line on stderr is safe.
- **`--from-text` (`src/session.rs::run`).** Already holds `formatter: &dyn Formatter`
  and `level` in `RunConfig`, and builds `clean_joined` before `write_entry`. The pass
  runs inside `run`, on `clean_joined`, before the write.

Running the pass on the already-joined clean text (not inside the event loop) means
`run_loop`, `apply_event`, and `LiveConfig` are **unchanged** — no `Formatter` or
`Level` threading through the live loop. The binary constructs the formatter once and
applies `guarded_document` to the joined clean.

### Raw/clean contract is preserved by the existing layout

The writer already emits **one whole-entry raw comment + one clean block** per entry
(`write_entry` receives `raw: Some(&raw_joined)`, `clean: &clean_joined`), not
per-phrase. Reshaping `clean` into paragraphs therefore leaves the single
`<!-- raw: -->` verbatim comment intact and still covering the whole entry. The live
`u` raw⇄clean toggle operates **pre-settle, per block, during the session**
(`render_model` over `settle.settled()`), so it is entirely unaffected by a
post-session pass. No new file layout is needed; this is called out so the contract is
explicit and tested.

---

## Architecture

```
LIVE/COMMIT (unchanged):  each phrase ──► deterministic_light ──► settle ──► live edge + raw
                          run_loop returns LiveResult{raw, clean}; Screen drops
                                                                        │
POST-SESSION (new, binary caller, Screen already gone):                 ▼
   if level ∈ {Medium, High} and model present:
       light_join (immutable) ──► guarded_document(level, light_join, &SmolFormatter)
                                       │ pass: model present? guard ok? not timed out?
                                       ├─ yes ──► reshaped body ──► write
                                       └─ no  ──► light_join     ──► write   (fail-safe)
   else: light_join ──► write   (today's behavior)
```

**Invariants:** the live edge and the verbatim raw are never touched by the LLM. The
fallback (`light_join`) is bound to an **immutable local before** the formatter is
invoked and returned verbatim on every failure branch (model absent, load/inference
error, guard reject, timeout) — so the worst case is byte-identical to today.

### The Medium/High guard — the correctness core

`guard_accepts` (Light) demands identical content words — too strict for Medium/High.
A new sibling **`guard_accepts_deletions(input, output) -> bool`** in
`talk-core/src/cleanup.rs` (coexisting with `guard_accepts`; Light keeps `guard_accepts`
in `guarded_format`, Medium/High use the new one in `guarded_document`) accepts iff:

1. **Subsequence.** The output's content words are a subsequence of the input's — every
   output word appeared in the input, in order. Permits deleting disfluencies/false
   starts and reflowing whitespace into paragraphs (whitespace is not a content word);
   forbids substitution, addition, and reordering.
2. **Negations are sound, including contractions.** This is computed **on the raw text
   with word-bounded matching, NOT via `content_words`** — because `content_words`
   splits on every non-alphanumeric char, so `can't` → `["can","t"]` and a pinned token
   `n't` could never appear (`"i can't go"` → `"i can go"` would pass the subsequence
   test, a meaning inversion). Instead: a `PINNED_NEGATIONS` constant (`not, never, no,
   none, nor, cannot, neither, nobody, nothing, nowhere, without`) **plus** any token
   matching the contraction pattern `*n't` (`can't, won't, don't, isn't, aren't, wasn't,
   weren't, didn't, doesn't, hasn't, haven't, hadn't, shouldn't, wouldn't, couldn't,
   ain't`) are counted by word-bounded scan on input and output; the output's count for
   each must be **≥** the input's. Any deleted negation fails closed.
3. **Deletion budget.** Reject when the output's content-word count drops below 60% of
   the input's, so a wholesale clause collapse (e.g. deleting a subject or a concessive
   `but`/`however` that flips meaning) fails closed rather than passing as "filler
   removal." Documented as a known limit (mirroring the existing `FILLERS` KNOWN LIMIT
   note): subsequence-preservation bounds, but does not perfectly capture, semantic
   fidelity — the budget backstops it.

Guard fails → fall back to the Light join.

### New units

| Unit | File | Responsibility |
|------|------|----------------|
| `guarded_document(level, full_text, &Formatter) -> String` | `talk-core/src/format.rs` | Empty-input short-circuit; bind fallback; call `Formatter::format(level, full_text)`; normalize; accept via `guard_accepts_deletions` else fallback |
| `guard_accepts_deletions` + `PINNED_NEGATIONS` | `talk-core/src/cleanup.rs` | Subsequence + sound-negation + deletion-budget guard |
| `strip_model_preamble(text) -> String` | `talk-core/src/cleanup.rs` | Strip a leading `Sure, here…`/`Here is…` line and wrapping quotes/code fences before the guard |
| `SmolFormatter` | `src/format/mod.rs` (NEW, `format` feature) | Candle load + bounded generation (max-new-tokens ∝ input); implements `Formatter` |
| `document_format(level, light_join) -> String` | `src/main.rs` (or `src/format/`) | Binary helper: construct formatter (Smol if feature+model+level warrants, else skip), run `guarded_document` on a worker thread with a 5 s deadline, spinner; shared by both callers |
| live-path call site | `src/main.rs::run_live_session` | After `run_loop` returns (Screen gone), apply `document_format` to `result.clean` before write |
| text-path call site | `src/session.rs::run` | Apply `guarded_document` to `clean_joined` before `write_entry` (RunConfig already carries formatter+level) |
| formatter model artifact | `src/download/models.rs` | Pinned SmolLM2-360M GGUF with a `lazy: bool` flag (excluded from first-run `fetch_all_models`/`models_ready`, no extraction step) |
| `journal_cleanup` default + template comment | `src/config.rs` | `"medium"` → `"high"`; fix the stale comment (below) |
| `--clean <level>` flag | `src/cli.rs` | Per-invocation level override |

`guarded_document` calls the existing `Formatter::format(level, text)` with
whole-document text; the trait doc is updated to note `format` may receive either a
single phrase (`guarded_format`) or a whole document (`guarded_document`) and must be
whole-text-safe (both `SmolFormatter` and `DeterministicFormatter` are). It is only
ever invoked with a model-backed formatter — when the model is absent the caller skips
the pass entirely, so `DeterministicFormatter` never flows through `guarded_document`.

---

## Model and inference

- **SmolLM2-360M-Instruct**, GGUF **Q4_K_M (~230 MB) by default** (Q8_0 ~380 MB is an
  opt-in upgrade). Llama2-architecture. **The exact Candle loader must be confirmed by
  a load spike during planning** — the prior formatter plan (Plan 3 T1) hit a Qwen2
  RoPE mismatch and warned `quantized_qwen2` ≠ `quantized_llama`; SmolLM2 is Llama-arch
  so `candle-transformers`' `quantized_llama` is the expected loader, but it is verified,
  not assumed.
- **Prompt reuse.** `rewrite_prompt(level, text)` (cleanup.rs) already encodes the
  restraint (`system`) and per-level rule (`user`); `SmolFormatter` executes it. The
  Medium rule ("remove disfluencies/false starts and join fragments") produces only
  deletions + whitespace reflow, which the subsequence guard accepts.
- **Bounded generation.** `max_new_tokens` is capped proportional to input length, so a
  runaway generation is impossible by construction. Greedy / very low temperature.
- **Output normalization.** `strip_model_preamble` runs before the guard, so a
  well-formed-but-prefixed reply (`"Sure, here is the cleaned text:"`) is not needlessly
  rejected into a Light fallback.
- The guard — not model size — enforces the quality floor, which is why Q4/360M is
  acceptable: a weak rewrite is rejected and falls back to Light.

---

## Packaging, download, and defaults

- **`format` cargo feature, on by default:** `default = ["listen", "format"]`. So
  `cargo install talk-cli`, Homebrew, and `install.sh` all get paragraphs out of the
  box. `--no-default-features` stays text-only. (Surfaced tradeoff: reflect-only users
  compile Candle they may not invoke; accepted for an out-of-box-complete experience.)
- **Lazy model download, separate from the first-run fetch.** The GGUF is a
  `download/models.rs` artifact with a `lazy: bool` flag: **excluded** from `MODELS` /
  `models_ready()` / the first-run `fetch_all_models` 330 MB offer, and **not** a
  `.tar.bz2` (no extraction/`EXTRACTED` entry). It flows through the same hardened
  `download::fetch` + `download::verify` (HTTPS-only, redirect-guarded, SHA-256-gated,
  read-capped). It downloads on **first actual need** (the first `talk journal`) via the
  existing one-key consent; `talk download models` fetches it up front; `talk download
  verify` checks it.
- **Decline / absent / offline → graceful Light.** If the model is missing and the user
  declines or is offline, the journal saves at Light (no paragraphs) and talk offers
  again next time — the entry is never lost. (Surfaced: a brand-new user's first journal
  may save at Light before the model lands; the close screen should not promise
  paragraphs that aren't there.)
- **Defaults:** `journal_cleanup` default `"medium"` → `"high"`; everything else stays
  Light through `cleanup_for`'s `else` branch. `--clean <none|light|medium|high>`
  overrides per invocation; `[cleanup]` config pins persist.
- **Config template comment fix** (config.rs commented_template): replace
  `"medium/high: deterministic-only in v1 (LLM enhances light); full LLM rewrite is
  future work"` with `"medium: LLM removes filler/joins fragments · high: + paragraphs;
  falls back to light if the model is absent"`.

---

## Latency and failure UX

- The pass runs **once, after the session, after the `Screen` guard drops**, on a worker
  thread with a **5 s wall-clock deadline**; `max_new_tokens` also bounds it by
  construction. On deadline the in-flight result is abandoned and the Light join is
  written.
- A TTY-gated `polishing…` line on stderr (`is_terminal()`), printed by `document_format`
  after the alternate screen is gone — never inside it. Non-TTY stays silent.
- **The write never blocks indefinitely:** the deadline + token cap back the
  guarantee. Every failure path (model missing, load/inference error, guard rejection,
  timeout) returns the immutable Light-join local and writes it.

---

## Testing

LLM output is not bit-deterministic, so tests never assert exact model text.

**Guard (`guard_accepts_deletions`, pure, exhaustive):**
- Filler deletion accepted; substitution/addition/reordering rejected.
- **Contraction negations: `"i can't go"` → `"i can go"` REJECTED**, `"i won't"` →
  `"i will"` rejected, `"don't"` → `"do"` rejected (the P0 hole), plus
  `"i am not sure"` → `"i am sure"` rejected.
- Deletion-budget: an output below 60% of input content words rejected even if a valid
  subsequence.
- Whitespace/paragraph reflow accepted (content words unchanged).

**`guarded_document` wiring (fake formatters, `Flip`-style):**
- A paragraph-inserting fake lands `\n\n` breaks in the written file at High.
- An over-editing fake (substitution) and a negation-dropping fake are both rejected →
  file equals the **unmodified Light join** (assert byte-identical via an erroring fake).
- Empty / whitespace-only input short-circuits to the Light join and never invokes the
  formatter.
- `strip_model_preamble` removes a leading `"Sure, here…"` line + wrapping quotes/fences.
- Light/None never invoke the pass — today's join is unchanged.

**Writer/contract:**
- A High-formatted entry has paragraphs in the clean body and the complete verbatim in a
  single `<!-- raw: -->` comment (the whole-entry-raw layout holds under reshaping).

**Real-model smoke (`format` feature, `#[ignore]` by default):**
- Loads the real Q4 GGUF once over a small fixture set (not one sample) and reports the
  guard **acceptance rate**, so "a 360M model is acceptable" has evidence rather than a
  single happy-path assertion.

**Release / CI:**
- A `format`-feature build leg (like `listen-build`); bare `test`/`clippy`/no-egress
  stay `--no-default-features`. SmolLM2 artifact SHA-256 pinned in `download/models.rs`.

---

## Out of scope

- Per-phrase live LLM formatting (the live edge stays deterministic by design).
- Spoken-list → bullet conversion beyond what the High prompt requests.
- GPU/Metal acceleration (CPU is sufficient at this size).
- Perfect semantic-fidelity guarantees beyond subsequence + negation + deletion-budget
  (a content-word guard cannot see sarcasm, quoted speech, or paragraph-segmentation
  quality — documented limits, not solved here).
- Any change to the speech-recognition passes (Spec A handles transcript corrections).
