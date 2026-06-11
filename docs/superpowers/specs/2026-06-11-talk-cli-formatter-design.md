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
once over the whole entry at the end of a session — never touching the live edge or
the verbatim raw — behind a widened guard that forbids any meaning change.

**Architecture:** Keep today's per-phrase deterministic-Light path for the live
experience. Add a **whole-document formatting pass** at settle time: when the level
is Medium/High and the model is present, run the LLM over the full clean text under a
**deletions-only subsequence guard** with pinned negations; on any failure fall back
to today's Light join. The LLM lives behind the existing `Formatter` trait seam in a
new `format` cargo feature; the model downloads lazily on first need through the
existing verified-download path.

**Tech stack:** Rust, Candle (`candle-core` + `candle-transformers`, quantized-llama),
SmolLM2-360M-Instruct GGUF, behind a new `format` feature. Reuses `rewrite_prompt`
(already in `talk-core/src/cleanup.rs`) and the existing download/verify pipeline.

---

## Background

Cleanup has four levels (`talk-core::cleanup::Level`): `None / Light / Medium / High`.
Only **Light** is implemented — it is the deterministic, no-model layer
(`deterministic_light`: caps, terminal punctuation, leading-filler strip, standalone
`i`). Medium and High were designed (Plan 3, task T7) to run an on-device LLM via the
`Formatter` trait, but **that inference was never built**: there is no `src/format/`
directory, and `main.rs` always injects `DeterministicFormatter`. Because the
`DeterministicFormatter` ignores its `level` and always emits deterministic-Light,
**every level silently collapses to Light** — no disfluency removal, no paragraphs.

Two pieces of infrastructure already exist and are reused as-is:

- **Per-mode level selection.** `config.rs` has `reflect_cleanup` / `journal_cleanup`
  and `cleanup_for(mode)`. Today: `reflect = light`, `journal = medium`.
- **The injected, guarded `Formatter` seam.** `session.rs::run` calls
  `guarded_format(cfg.formatter, cfg.level, raw)` per committed phrase, and
  `format.rs::guarded_format` already enforces a guard with a deterministic-Light
  fallback. The over-editing `Flip` test proves a misbehaving formatter cannot
  corrupt the file.

### The one structural problem

`guarded_format` runs **per committed phrase**. That is correct for Light and for the
live edge, but **paragraph breaks need the whole entry** — you cannot decide a topic
shift from a single phrase. `session.rs` itself notes this: "structured formatting
(Medium/High) is a follow-on concern; this path only guarantees the literal command
words don't survive." So Medium/High require a final whole-document pass.

---

## Architecture

```
LIVE (unchanged):   each phrase ──► guarded_format(Light) ──► settle ──► live edge + verbatim raw
                                                                            │
SETTLE (new):       after settle.finalize():                                ▼
                    if level ∈ {Medium, High} and model present:
                        full clean text ──► LLM formatter ──► subsequence guard ──► written body
                    else:
                        today's space-join (Light) ───────────────────────────► written body
```

**Invariants preserved:**

- The **live edge** and the **verbatim raw** are never touched by the LLM. The
  streaming experience stays fast and deterministic; the `<!-- raw: -->` comment (and
  `u` toggle) always holds the exact words.
- The LLM reshapes only the **final written body**, once, at the end, guarded, with a
  fallback. Worst case (model missing / slow / rejected) is exactly today's behavior.

### The Medium/High guard — the correctness core

Today's `guard_accepts` demands *identical* content words (no change), which is right
for Light but rejects all of Medium/High (they remove filler). Medium/High get a
**deletions-only subsequence guard**, `guard_accepts_deletions(input, output)`:

- The output's content words must be a **subsequence** of the input's content words —
  every output word appeared in the input, in the same order.
- This **permits**: removing disfluencies/false starts (deletion) and reflowing
  whitespace into paragraphs (whitespace is not a content word, so it is invisible to
  the guard).
- This **forbids**: substituting a word, adding a word, or reordering — the dangerous
  rewrites.
- **Pinned negations.** `not, never, no, none, nor, cannot, n't` may never be deleted:
  if the input contains a negation the output lacks, the guard fails regardless of
  subsequence. Dropping a negation is a meaning inversion, so it is excluded from the
  "deletion is fine" rule.
- Guard fails → fall back to `deterministic_light` over the space-joined Light text.
  Same moat philosophy as Light, widened exactly enough for filler removal.

### New units

| Unit | File | Responsibility |
|------|------|----------------|
| `guarded_document(level, full_text, &Formatter) -> String` | `talk-core/src/format.rs` | Whole-document sibling to `guarded_format`, using the subsequence guard |
| `guard_accepts_deletions(input, output) -> bool` | `talk-core/src/cleanup.rs` | Subsequence + pinned-negation guard |
| `SmolFormatter` | `src/format/mod.rs` (NEW, `format` feature) | Candle quantized-llama load + constrained rewrite via `rewrite_prompt` |
| settle-time document pass | `src/session.rs` | After `finalize()`, run `guarded_document` when level warrants + model present; else today's join |
| formatter model artifact | `src/download/models.rs` | Pinned SmolLM2-360M GGUF; lazy fetch + `verify` |
| `--clean <level>` flag | `src/cli.rs` | Per-invocation level override |

---

## Model and inference

- **SmolLM2-360M-Instruct**, quantized to GGUF (Q8_0, ~380 MB; Q4_K_M ~230 MB is the
  leaner fallback if footprint matters more than quality). Llama-architecture, so it
  loads through Candle's `quantized-llama`. Apache-2.0.
- **Prompt reuse.** `rewrite_prompt(level, text)` already encodes the restraint
  (`system`) and the per-level rule (`user`) in the pure core. The `SmolFormatter`
  executes that prompt; the restraint wording is not duplicated in the façade.
- **Decoding:** greedy / very low temperature for stability and reproducibility of
  behavior (not bit-exact output, but consistent restraint).
- The guard is the safety net: because a weak rewrite is rejected and falls back to
  Light, a 360M model is acceptable. This is why the model can be small.

---

## Packaging, download, and defaults

- **`format` cargo feature**, on by default: `default = ["listen", "format"]`. So
  `cargo install talk-cli`, Homebrew, and `install.sh` all get paragraphs out of the
  box. `--no-default-features` stays text-only. Candle is pure Rust — lighter to
  build than the sherpa-onnx C++ stack already compiled under `listen`.
- **Lazy model download, never bundled.** The 360 MB model is **not** added to the
  first-run 330 MB speech fetch. It downloads on **first actual need** (the first
  `talk journal`, since journal defaults to High) via the existing one-key consent,
  through the verified path (HTTPS-only, redirect-guarded, SHA-256-gated, read-capped).
  `talk download models` fetches it up front; `talk download verify` checks it.
- **Decline / absent → graceful Light.** If the user declines or the model is missing,
  that session falls back to deterministic-Light (journal still works, no paragraphs),
  and talk offers again next time — no nagging, no silent failure.
- **Per-mode defaults:** `journal → High`, `reflect → Light`, `thread/own-question →
  Light`, `unburden → Light`. Implemented by changing the `journal_cleanup` default
  from `"medium"` to `"high"` in `config.rs`; the per-mode plumbing already exists.
- **`--clean <none|light|medium|high>`** overrides the resolved level for one
  invocation; config `[cleanup]` pins persist it.

---

## Latency and failure UX

- The document pass runs **once at settle** (~1–2 s on CPU for a full journal).
- TTY-gated `polishing…` spinner on stderr while it runs (same `is_terminal()` gating
  as the download progress bar); non-TTY stays silent.
- **The write never blocks on the LLM.** Any failure — model missing, load error,
  inference error, guard rejection, or excessive time — falls back to Light and writes
  the file. Fail-safe is always your words.
- The misleading `config.rs` template comment ("medium/high: deterministic-only in v1
  … full LLM rewrite is future work") is corrected to describe the shipped behavior.

---

## Testing

LLM output is not bit-deterministic, so tests never assert exact model text.

**Subsequence guard (pure, exhaustive):**
- Filler deletion accepted (`"um so i mean the thing"` → `"the thing"`).
- Substitution rejected; addition rejected; reordering rejected.
- Negation drop rejected (`"i am not sure"` → `"i am sure"` fails) even though it is a
  valid subsequence otherwise.
- Whitespace/paragraph reflow accepted (content words unchanged).

**Document-pass wiring (fake formatters, `Flip`-style):**
- A paragraph-inserting fake formatter lands `\n\n` paragraph breaks in the written
  file at High.
- An over-editing fake (word substitution) is rejected by the subsequence guard → the
  file contains the Light fallback, never the corruption.
- A negation-dropping fake is rejected → fallback.
- Level `Light` (and `None`) never invoke the document pass — today's per-phrase join
  is unchanged.

**Real-model smoke test (`format` feature, `#[ignore]` by default):**
- Loads the real SmolLM2 GGUF once and runs a single Medium rewrite, asserting the
  output passes the subsequence guard — mirrors the existing FFI/sandbox proof pattern
  (needs the downloaded model, so excluded from the default test run).

**Release / CI:**
- A `format`-feature build leg (like the existing `listen-build`) exercises the Candle
  path; the bare `test`/`clippy`/no-egress steps keep `--no-default-features`.
- The SmolLM2 artifact SHA-256 is pinned in `download/models.rs`.

---

## Out of scope

- Per-phrase live LLM formatting (the live edge stays deterministic by design).
- Spoken-list → bullet conversion beyond what the High prompt already requests
  (no structured list parser; the model handles it under the guard or it falls back).
- GPU/Metal acceleration (CPU inference is sufficient at this size; a later
  optimization).
- Any change to the speech-recognition passes (Spec A handles transcript corrections).
