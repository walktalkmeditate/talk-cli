# talk-cli Formatter + Restraint Implementation Plan (Plan 3 of 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the optional on-device LLM formatter — a quantized 0.5B model that prettifies a settled phrase — gated so hard by content-word restraint (and an instant deterministic fallback) that it can never change what you said.

**Architecture:** `talk-core` owns the restraint **policy** (the `Formatter` trait, the per-level prompt, the always-safe `DeterministicFormatter`, the diff-guarded `guarded_format` call site, and the checked-in **eval set**) — all pure and CI-tested. The binary's `src/format/` is a thin **Candle** façade (`CandleFormatter`) behind a cargo `formatter` feature that loads a quantized Qwen2.5-0.5B-Instruct GGUF and implements that same trait. At runtime deterministic-Light still settles **instantly**; the LLM runs async on a worker thread and, *iff* it returns inside the swap window **and** passes the content-word guard, replaces the committing block via `settle::upgrade_committing` (never re-flowing settled text). Miss the window or fail the guard → deterministic-Light stays, permanently.

**Scope of the LLM rewrite in Plan 3 — Light only.** The content-word guard (`guard_accepts`) requires the output to preserve *every* content word, so it accepts only meaning-preserving edits (caps/punctuation/filler) — exactly the **Light** level. **Medium** ("remove disfluencies, join fragments") and **High** ("paragraphs, bulleted lists") edit by *removing* content words, which the strict guard rejects by design. So Plan 3 routes **only Light** through the LLM; Medium/High behave as deterministic-Light this plan (their authored prompts ship for forward-compat). Making Medium/High actually function needs a **level-aware subsequence guard** (Medium permits output content-words to be an ordered *subsequence* of input — drops allowed, substitutions/additions/reorders/dropped-negations still forbidden), which is **deferred to a follow-on plan**. This keeps restraint airtight now and matches the spec's framing of Light as the realtime async enhancement.

**Tech Stack:** Rust 1.82; `candle-core` + `candle-transformers` (pure-Rust ML, quantized Qwen2 GGUF, CPU) + `tokenizers` (behind the `formatter` feature); the Plan-2 `download` machinery (`ureq` + `sha2`) for the model assets. No new network surface — Candle is compute-only.

**Origin spec:** `docs/superpowers/specs/2026-06-08-talk-cli-design.md` (§7 settle/async-swap, §10 formatter & restraint, §11 model integrity, §14 eval set). **Roadmap source:** `docs/superpowers/plans/2026-06-08-talk-cli-foundation.md` (Plan 3 entry). Note the roadmap's `upgrade_block` is stale — the real method is `settle::upgrade_committing`.

---

## Execution venue (read first)

This plan splits exactly like Plan 2 did:

- **CI here (pure / no model):** Tasks **2–6** build and unit-test `talk-core`'s formatter policy + eval set, the config cleanup-level wiring, and the diff-guard seam in the non-interactive `session::run` path. They need no Candle, no model, no mic — `cargo test` proves them, and they are **independent of the latency spike's result** so they can land immediately. **Order within the CI group:** T2→T3→T4 are mutually independent, but **T6 depends on T5** (it calls `Config::cleanup_for`, introduced in T5) and on T3 (it calls `guarded_format`). Do T2–T5 before T6.
- **Your machine (model + ML + interactive):** Tasks **1, 7–9** — the latency spike, the real Candle façade, the model download, and the live-loop async swap — need the GGUF model, CPU inference, and (for T9) a real session. Each says so and gives the exact on-machine check.

**Dependency on Plan 2's audio half.** Tasks 7–9 build on the Plan-2 audio half (`src/listen/`, `src/download/`, `src/live.rs`, the `listen`/`download` features) that is verified on your machine. If that half isn't merged yet, do the CI tasks (2–6) now and run 1, 7–9 after Plan 2's audio half lands. Tasks 2–6 touch none of it.

**Why the spike is Task 1 (per spec §7).** Whether the realtime async swap is even worth shipping depends on one unknown number: can a quantized 0.5B clean a phrase fast enough to fit the swap window on a commodity CPU? Task 1 measures it before T7–T9 commit to the realtime path. If the number is bad, T9 widens the *committing* window (the committing block is allowed to linger; only **settled** text is immutable) rather than abandoning the feature — the deterministic-Light layer ships regardless.

---

## File structure

```
talk-cli/
  Cargo.toml                         # + [features] formatter; candle-core, candle-transformers, tokenizers (optional)
  crates/talk-core/src/
    cleanup.rs                       # + parse_level(), rewrite_prompt() (policy + prompt); drop the Level dead_code allow
    format.rs                        # NEW: Formatter trait + DeterministicFormatter + guarded_format (the moat) [here-testable]
    eval.rs                          # NEW: Fixture + score() + vendored FIXTURES + must-fail over-editing mock [here-testable]
    lib.rs                           # + pub mod format; pub mod eval;
  src/
    config.rs                        # + reflect_cleanup / journal_cleanup + cleanup_for() [here-testable]
    session.rs                       # route Commit through guarded_format via an injected &dyn Formatter [here-testable]
    main.rs                          # construct DeterministicFormatter + per-mode Level for run_and_report
    format/
      mod.rs                         # NEW: CandleFormatter (quantized Qwen2 GGUF) behind feature "formatter" [machine]
    download/
      models.rs                      # + FORMATTER_MODELS manifest (GGUF + tokenizer.json, pinned SHA-256) [network]
    live.rs                          # async LLM swap: spawn formatter per phrase, upgrade_committing on guard-pass [machine]
  examples/
    format_latency.rs                # NEW (throwaway): the gating latency spike [machine]
```

---

## Task 1: Formatter latency spike [needs your machine — model + CPU; GATING]

**Files:**
- Modify: `Cargo.toml` (add the `formatter` feature + Candle deps)
- Create (throwaway): `examples/format_latency.rs`

This is a **measurement**, not shipped code. It answers the one question that gates the realtime swap (spec §7): on a commodity CPU, what is the p50/p95 wall-clock to clean one phrase with a quantized 0.5B? The example inlines the minimal Candle path (the same API `CandleFormatter` will use in T7), so it doubles as API validation before you write the real façade.

- [ ] **Step 1: Cargo.toml — add the `formatter` feature + Candle deps**

Plan 2 created the `[features]` table (`listen`, `download`). Add `formatter` to it and the three optional deps. Use these exact lines:

```toml
# Add to the existing [features] table — formatter pulls in `download` so the
# verify-before-load machinery (T8) compiles whenever the formatter is enabled:
formatter = ["download", "dep:candle-core", "dep:candle-transformers", "dep:tokenizers"]

# Add to [dependencies] — candle-core and candle-transformers MUST share the same
# minor or Cargo links two incompatible candle-core copies (from_gguf then sees a
# 0.9 Content where it wants 0.8 — a type mismatch that won't compile):
candle-core = { version = "0.9.2", optional = true }            # crates.io stable; main is ahead (0.10.x)
candle-transformers = { version = "0.9.2", optional = true }    # MUST match candle-core's minor; provides models::quantized_qwen2
tokenizers = { version = "0.23.1", optional = true }            # Tokenizer::from_file (local-file load only)
```

> **Pin step (do this first, on your machine):** run `cargo +1.82 build --features formatter` and confirm a SINGLE `candle-core` resolves (`cargo tree -p candle-core` shows one version) — version skew between `candle-core` and `candle-transformers` is the #1 Candle build failure. Pin both to the same released minor (0.9.2 above; bump together if you move). Read `candle-examples/examples/quantized-qwen2-instruct/main.rs` at the matching tag — the authoritative reference for every Candle symbol here. Use `quantized_qwen2`, **not** `quantized_llama` (wrong RoPE for Qwen2, candle #3410).
>
> **Network audit (privacy — spec §11/§14):** the `formatter` deps are compute-only, but verify it: `cargo tree -e features --features formatter | grep -Ei 'hf.?hub|reqwest|^.*\bhttp\b'` must show **no** `hf-hub`/`http`/network feature on `candle-*` or `tokenizers` (the session path stays zero-network even with `formatter` on; `Tokenizer::from_file` is local-only and `tokenizers`' `http`/`hf-hub` features are off by default — assert they stay off). Do **not** blanket `default-features = false` on `tokenizers` — that drops `onig`, which Qwen's pretokenizer needs.

- [ ] **Step 2: Download the model assets (manual, one-time)**

The GGUF and tokenizer are two **separate** assets (the tokenizer is not bundled in the GGUF):

```
# ~491 MB quantized model:
curl -L -o /tmp/qwen2.5-0.5b-instruct-q4_k_m.gguf \
  https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf
# tokenizer.json from the BASE instruct repo (not the GGUF repo):
curl -L -o /tmp/qwen2.5-0.5b-tokenizer.json \
  https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/resolve/main/tokenizer.json
```

- [ ] **Step 3: Write the spike example**

Create `examples/format_latency.rs`:

```rust
//! THROWAWAY latency spike (spec §7). Measures quantized-0.5B Light-cleanup
//! wall-clock so we can decide whether the realtime async swap fits its window.
//! Run on M1 AND a commodity x86. Delete after recording the numbers in the plan.
//! Usage: cargo run --release --features formatter --example format_latency -- <gguf> <tokenizer.json>

use std::fs::File;
use std::time::Instant;
use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::quantized_qwen2::ModelWeights;
use tokenizers::Tokenizer;

const PHRASES: &[&str] = &[
    "um so the thing i keep coming back to is the weight of other peoples expectations",
    "i guess what im really trying to say is that i feel stuck",
    "and then i realized that maybe i was the one avoiding the conversation",
    "theres this part of me that wants to just walk away from all of it",
    "honestly i dont know if im making the right call here",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (gguf, tok) = (&args[1], &args[2]);
    let device = Device::Cpu;

    let load0 = Instant::now();
    let mut file = File::open(gguf)?;
    let content = gguf_file::Content::read(&mut file)?;
    let mut model = ModelWeights::from_gguf(content, &mut file, &device)?;
    let tokenizer = Tokenizer::from_file(tok).map_err(|e| e.to_string())?;
    let eos = *tokenizer.get_vocab(true).get("<|im_end|>").ok_or("no <|im_end|>")?;
    println!("cold load: {} ms", load0.elapsed().as_millis());

    let system = "You clean up raw voice transcripts. Return ONLY the cleaned text. NEVER change meaning. Fix only capitalization and punctuation, and drop leading filler.";
    let mut times = Vec::new();
    for phrase in PHRASES {
        let prompt = format!(
            "<|im_start|>system\n{system}<|im_end|>\n<|im_start|>user\nClean this transcript:\n{phrase}<|im_end|>\n<|im_start|>assistant\n");
        let ids: Vec<u32> = tokenizer.encode(prompt, true).map_err(|e| e.to_string())?.get_ids().to_vec();

        let t0 = Instant::now();
        // No clear_kv_cache() — it doesn't exist in candle-transformers 0.9.x;
        // forward(&input, 0) (the prefill below) resets the KV cache for each phrase.
        let mut lp = LogitsProcessor::from_sampling(0, Sampling::ArgMax);
        let input = Tensor::new(ids.as_slice(), &device)?.unsqueeze(0)?;
        let logits = model.forward(&input, 0)?.squeeze(0)?;
        let mut next = lp.sample(&logits)?;
        let mut pos = ids.len();
        let mut out = Vec::new();
        while next != eos && out.len() < 96 {
            out.push(next);
            let input = Tensor::new(&[next], &device)?.unsqueeze(0)?;
            let logits = model.forward(&input, pos)?.squeeze(0)?;
            next = lp.sample(&logits)?;
            pos += 1;
        }
        let ms = t0.elapsed().as_millis();
        times.push(ms);
        println!("[{ms:>4} ms] {}", tokenizer.decode(&out, true).map_err(|e| e.to_string())?);
    }
    times.sort_unstable();
    println!("p50 {} ms · p95 {} ms ({} phrases)", times[times.len()/2], times[times.len()*95/100], times.len());
    Ok(())
}
```

> Confirm `ModelWeights::from_gguf(content, &mut file, &device)`, `model.forward(&input, pos)` (returns last-position logits), and `LogitsProcessor::from_sampling(seed, Sampling::ArgMax)` against the pinned `quantized-qwen2-instruct` example. ArgMax = deterministic decode (restraint: no sampling randomness). The prefill `forward(&input, 0)` resets the KV cache per phrase — there is no `clear_kv_cache` at 0.9.x.

- [ ] **Step 4: Measure on two CPUs and record the decision**

```
cargo run --release --features formatter --example format_latency -- \
  /tmp/qwen2.5-0.5b-instruct-q4_k_m.gguf /tmp/qwen2.5-0.5b-tokenizer.json
```

Run on **M1** (or your Apple-silicon floor) and a **commodity x86**. Record `cold load`, `p50`, `p95` in this plan's Self-Review. Decision gate for T9:
- **p95 ≤ ~250 ms:** ship the realtime swap as specced (~250 ms window).
- **p95 > window:** in T9, set the committing-block swap window to the measured p95 (the committing block may linger; only **settled** text is immutable) so the swap still lands without ever re-flowing settled text. If p95 is wildly off (multiple seconds), ship deterministic-Light as the live experience and keep the LLM as an opt-in non-realtime polish — note that explicitly. Either way the feature stays *safe*; only its *liveness* bends.

- [ ] **Step 5: Delete the example, commit the deps + decision**

```bash
rm examples/format_latency.rs
git add Cargo.toml Cargo.lock
git commit -m "feat: formatter cargo feature + Candle deps (latency spike recorded in plan)"
```

---

## Task 2: Cleanup policy — parse_level + rewrite_prompt [here-testable]

**Files:**
- Modify: `crates/talk-core/src/cleanup.rs`

`talk-core` owns the restraint **wording** (the prompt) and the level parsing. The Candle façade (T7) only wraps `rewrite_prompt` in the model's chat template — the policy never leaves core. Also drop the `#[allow(dead_code)]` on `Level`: it becomes live here.

- [ ] **Step 1: Write the failing test**

In `crates/talk-core/src/cleanup.rs`, change the `Level` definition to remove the dead-code allow:

```rust
/// Cleanup intensity. Plan 3 wires this into the LLM rewrite; deterministic-Light
/// is the instant, always-present layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level { None, Light, Medium, High }
```

Add at the end of the file, above the `#[cfg(test)]` module:

```rust
/// Parse a config string into a `Level` (defaults to Light — the safe, restrained
/// default — on anything unrecognized).
pub fn parse_level(s: &str) -> Level {
    match s.trim().to_lowercase().as_str() {
        "none" => Level::None,
        "medium" => Level::Medium,
        "high" => Level::High,
        _ => Level::Light,
    }
}

/// The constrained-rewrite prompt for the LLM formatter (consumed by the Candle
/// façade in T7). `system` is hard restraint that holds at every level; the
/// per-level rule only *widens* which edits are permitted. Restraint is the
/// wording, so it lives here in the pure core, not in the inference façade.
pub struct RewritePrompt {
    pub system: String,
    pub user: String,
}

pub fn rewrite_prompt(level: Level, text: &str) -> RewritePrompt {
    let restraint = "You clean up raw voice transcripts. Return ONLY the cleaned text, nothing else — no preamble, no quotes. NEVER change meaning: never swap a word for a different one, never add words that change meaning, never drop a negation, never reorder clauses. When unsure, leave it as it is.";
    let rule = match level {
        Level::None => "Return the text exactly as given.",
        Level::Light => "Fix only capitalization and punctuation, and drop leading filler (um, uh, like). Remove no other words.",
        Level::Medium => "Also remove disfluencies and false starts and join fragments into sentences. Keep every meaning-bearing word.",
        Level::High => "Also break into paragraphs at topic shifts and turn spoken lists into bullets. Keep every meaning-bearing word.",
    };
    RewritePrompt {
        system: format!("{restraint} {rule}"),
        user: format!("Clean this transcript:\n{text}"),
    }
}
```

> The Medium/High prompts are authored here for forward-compatibility, but Plan 3 does **not** invoke the LLM at those levels (the strict content-word guard would reject their word-removing edits — see the Architecture "Light only" note and Task 3). They cost nothing to keep and document the intended policy for the follow-on subsequence-guard work.

Add these tests to the `#[cfg(test)] mod tests` block in `cleanup.rs`:

```rust
    #[test]
    fn parse_level_maps_known_and_defaults_to_light() {
        assert_eq!(parse_level("none"), Level::None);
        assert_eq!(parse_level("Medium"), Level::Medium);
        assert_eq!(parse_level("HIGH"), Level::High);
        assert_eq!(parse_level("light"), Level::Light);
        assert_eq!(parse_level("nonsense"), Level::Light);
    }

    #[test]
    fn rewrite_prompt_widens_by_level_and_carries_the_text() {
        assert!(rewrite_prompt(Level::Light, "x").system.to_lowercase().contains("capitalization"));
        assert!(rewrite_prompt(Level::Medium, "x").system.to_lowercase().contains("disfluencies"));
        assert!(rewrite_prompt(Level::High, "x").system.to_lowercase().contains("paragraph"));
        assert!(rewrite_prompt(Level::Light, "the raw phrase").user.contains("the raw phrase"));
    }

    #[test]
    fn rewrite_prompt_always_states_the_restraint() {
        for lvl in [Level::Light, Level::Medium, Level::High] {
            assert!(rewrite_prompt(lvl, "x").system.to_lowercase().contains("never change meaning"));
        }
    }
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p talk-core cleanup`
Expected: PASS (the existing cleanup tests + the 3 new ones).

- [ ] **Step 3: Commit**

```bash
git add crates/talk-core/src/cleanup.rs
git commit -m "feat(core): cleanup level parsing + constrained rewrite prompt"
```

---

## Task 3: The Formatter trait + the diff-guarded moat [here-testable]

**Files:**
- Create: `crates/talk-core/src/format.rs`
- Modify: `crates/talk-core/src/lib.rs` (`pub mod format;`)

The seam every formatter implements, plus `guarded_format` — the moat applied at the **call site**: run the deterministic pre-layer (spoken commands, backtrack), then the formatter, then **accept the formatter's output only if the content-word guard passes**, else fall back to deterministic-Light. The returned text is *always* guard-safe, so a hallucinating LLM can never change meaning in a file. `guarded_format` is the single entry point for the **non-interactive** `session::run` path (T6). The live loop (T9) runs the pre-layer inline — synchronous pre-processing must finish before the async formatter thread is spawned — but calls the same `guard_accepts`/`deterministic_light` from `cleanup`, so the restraint logic lives in one module even though the call structure differs.

**Guard strictness = Light only (see Architecture).** `guard_accepts` demands every content word survive, so it accepts only Light-grade edits. A Medium/High rewrite that *removes* words is rejected and falls back to deterministic-Light — which is why Plan 3 routes only Light to the LLM (T9). The `guard_rejects_a_dropped_negation_and_falls_back` test below already exercises the word-removal rejection that makes this true.

- [ ] **Step 1: Write the failing test**

Create `crates/talk-core/src/format.rs`:

```rust
//! The formatter seam (pure). `talk-core` owns the restraint POLICY: the
//! `Formatter` contract, the always-safe deterministic fallback, and the
//! diff-guarded call site. The real Candle 0.5B inference lives in the binary's
//! `src/format/` and implements this same trait (Plan 3 T7).

use crate::cleanup::{apply_backtrack, apply_spoken_commands, deterministic_light, guard_accepts, Level};

/// Turn one phrase into cleaned text at a given level. Implementors do ONLY their
/// transform — the deterministic pre-layer and the diff-guard are applied by
/// `guarded_format`, never here. (So a formatter receives already-pre-processed
/// text and must not re-apply spoken commands / backtrack.)
pub trait Formatter {
    fn format(&self, level: Level, text: &str) -> String;
}

/// The always-safe formatter: deterministic-Light, no model. Guard-safe by
/// construction (caps / punctuation / leading-filler only).
pub struct DeterministicFormatter;

impl Formatter for DeterministicFormatter {
    fn format(&self, _level: Level, text: &str) -> String {
        deterministic_light(text)
    }
}

/// The moat. Pre-layer → format → accept iff the content-word guard passes, else
/// deterministic-Light. `None` short-circuits to the pre-processed text (no
/// formatting). The result is ALWAYS guard-safe relative to the pre-processed
/// phrase — fail-safe is always your words.
pub fn guarded_format(f: &dyn Formatter, level: Level, raw: &str) -> String {
    let pre = apply_backtrack(&apply_spoken_commands(raw));
    if level == Level::None {
        return pre;
    }
    let candidate = f.format(level, &pre);
    if guard_accepts(&pre, &candidate) {
        candidate
    } else {
        deterministic_light(&pre)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Restraint incarnate — deterministic-Light never substitutes a word.
    struct Faithful;
    impl Formatter for Faithful {
        fn format(&self, _l: Level, text: &str) -> String { deterministic_light(text) }
    }

    /// Substitutes meaning words / drops negations — exactly what the guard rejects.
    struct OverEditing;
    impl Formatter for OverEditing {
        fn format(&self, _l: Level, text: &str) -> String {
            format!(" {} ", text).replace(" love ", " hate ").replace(" not ", " ").trim().to_string()
        }
    }

    #[test]
    fn none_level_returns_pre_layer_unchanged() {
        assert_eq!(guarded_format(&Faithful, Level::None, "i am not done"), "i am not done");
    }

    #[test]
    fn pre_layer_runs_before_formatting() {
        // backtrack drops the clause before "scratch that"
        let out = guarded_format(&Faithful, Level::Light, "the answer is yes scratch that the answer is no");
        assert!(!out.contains("yes"));
        assert!(out.contains("answer is no"));
    }

    #[test]
    fn guard_rejects_a_meaning_substitution_and_falls_back() {
        let out = guarded_format(&OverEditing, Level::Light, "i love her");
        assert!(!out.contains("hate"));               // the over-edit was rejected
        assert!(out.to_lowercase().contains("love")); // deterministic-Light kept the meaning
    }

    #[test]
    fn guard_rejects_a_dropped_negation_and_falls_back() {
        let out = guarded_format(&OverEditing, Level::Light, "i am not angry");
        assert!(out.to_lowercase().contains("not"));  // negation preserved by the fallback
    }

    #[test]
    fn faithful_output_passes_the_guard_unchanged() {
        assert_eq!(guarded_format(&Faithful, Level::Light, "um so i keep avoiding it"), "Keep avoiding it.");
    }
}
```

Add to `crates/talk-core/src/lib.rs`: `pub mod format;`

- [ ] **Step 2: Run the tests**

Run: `cargo test -p talk-core format`
Expected: PASS (5 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/talk-core/src/format.rs crates/talk-core/src/lib.rs
git commit -m "feat(core): Formatter trait + diff-guarded format (the restraint moat)"
```

---

## Task 4: The restraint eval set [here-testable]

**Files:**
- Create: `crates/talk-core/src/eval.rs`
- Modify: `crates/talk-core/src/lib.rs` (`pub mod eval;`)

The checked-in eval set is a **regression harness** for known meaning-flip classes (sentiment swaps, dropped negations, intensity-softening). Each fixture lists impermissible substrings a faithful cleanup must never introduce; `score` returns the fraction with zero impermissible edits. A faithful formatter scores **green (1.0)**; a deliberately over-editing mock **must score red (<1.0)** — proof the metric can detect those classes. A third test shows the runtime guard makes even the bad mock safe (belt + suspenders).

> **Honest scope of this CI gate.** It runs against *mock* formatters only — the real `CandleFormatter` needs the model and is your-machine-only, so CI never scores it. So this is a guard against *regressions in the known meaning-flip classes*, not a proof the real model is restrained. The real model's restraint is enforced at **runtime** by `guard_accepts` (fail-safe to deterministic-Light) and **spot-checked on-machine** in T7 Step 3 (which records `score(&CandleFormatter, …)` against `FIXTURES`). Expand `FIXTURES` whenever you observe a new over-edit class on-machine.

- [ ] **Step 1: Write the failing test**

Create `crates/talk-core/src/eval.rs`:

```rust
//! The checked-in restraint eval set (pure). Makes "the formatter never changes
//! meaning" a FALSIFIABLE gate, not a vibe: each fixture lists impermissible
//! substrings (meaning-changing edits) that must never appear in a formatter's
//! output. `score` is the fraction of fixtures with zero impermissible edits. A
//! deliberately over-editing mock MUST score red (see tests).

use crate::cleanup::Level;
use crate::format::Formatter;

/// One eval case: a phrase + substrings a faithful cleanup must never introduce.
/// Fixtures are written already pre-processed (no spoken commands) so they isolate
/// the formatter's restraint, not the deterministic pre-layer.
pub struct Fixture {
    pub raw: &'static str,
    pub impermissible: &'static [&'static str],
}

/// The vendored fixtures: a phrase paired with the meaning-flips a careless rewrite
/// tends to introduce (sentiment swaps, dropped negations).
pub const FIXTURES: &[Fixture] = &[
    Fixture { raw: "i think i love this plan", impermissible: &["hate", "loathe"] },
    Fixture { raw: "i always make time for this", impermissible: &["never"] },
    Fixture { raw: "maybe i should reach out to her", impermissible: &["shouldn't", "should not"] },
    Fixture { raw: "the result felt good for everyone", impermissible: &["bad", "terrible"] },
    Fixture { raw: "i am not angry about it anymore", impermissible: &["i am angry", "still angry"] },
    Fixture { raw: "um so the thing i keep avoiding is the call", impermissible: &["easy", "trivial"] },
    Fixture { raw: "i was furious about the whole thing", impermissible: &["annoyed", "frustrated", "upset"] }, // intensity-softening, not just sentiment-flip
];

/// Fraction of fixtures the formatter cleans without an impermissible edit
/// (1.0 = perfect restraint). Scores the formatter's DIRECT output (unguarded), so
/// the metric measures the model, not the moat.
pub fn score(f: &dyn Formatter, level: Level, fixtures: &[Fixture]) -> f32 {
    if fixtures.is_empty() {
        return 1.0;
    }
    let passed = fixtures.iter().filter(|fx| {
        let out = f.format(level, fx.raw).to_lowercase();
        fx.impermissible.iter().all(|bad| !out.contains(&bad.to_lowercase()))
    }).count();
    passed as f32 / fixtures.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleanup::deterministic_light;
    use crate::format::{guarded_format, DeterministicFormatter};

    struct Faithful;
    impl Formatter for Faithful {
        fn format(&self, _l: Level, text: &str) -> String { deterministic_light(text) }
    }

    /// The must-fail mock: flips sentiment + drops negations.
    struct OverEditing;
    impl Formatter for OverEditing {
        fn format(&self, _l: Level, text: &str) -> String {
            format!(" {} ", text)
                .replace(" love ", " hate ")
                .replace(" always ", " never ")
                .replace(" should ", " shouldn't ")
                .replace(" good ", " bad ")
                .replace(" not ", " ")
                .trim().to_string()
        }
    }

    #[test]
    fn faithful_formatter_scores_green() {
        assert_eq!(score(&Faithful, Level::Light, FIXTURES), 1.0);
    }

    #[test]
    fn over_editing_mock_scores_red() {
        // The whole point of the eval: a careless model is CAUGHT.
        assert!(score(&OverEditing, Level::Light, FIXTURES) < 1.0);
    }

    #[test]
    fn the_guard_makes_even_the_over_editing_mock_safe() {
        // Belt (eval) + suspenders (guard): at runtime guarded_format falls back,
        // so the over-editing mock never lands an impermissible edit in a file.
        for fx in FIXTURES {
            let out = guarded_format(&OverEditing, Level::Light, fx.raw).to_lowercase();
            for bad in fx.impermissible {
                assert!(!out.contains(&bad.to_lowercase()), "guard let {:?} through on {:?}", bad, fx.raw);
            }
        }
        // and the deterministic fallback itself is green
        assert_eq!(score(&DeterministicFormatter, Level::Light, FIXTURES), 1.0);
    }
}
```

Add to `crates/talk-core/src/lib.rs`: `pub mod eval;`

- [ ] **Step 2: Run the tests**

Run: `cargo test -p talk-core eval`
Expected: PASS (3 tests). The `over_editing_mock_scores_red` test is the falsifiable gate — if it ever passes (mock scores 1.0) the eval is broken.

- [ ] **Step 3: Commit**

```bash
git add crates/talk-core/src/eval.rs crates/talk-core/src/lib.rs
git commit -m "feat(core): checked-in restraint eval set (faithful green, over-edit red)"
```

---

## Task 5: Per-mode cleanup level in config [here-testable]

**Files:**
- Modify: `src/config.rs`

Spec §12: config pins a per-mode cleanup level; §7: the template documents each level with a one-line example so the choice is informed. Reflect defaults Light, journal defaults Medium. The defaults preserve the spec's intent, but in Plan 3 Medium/High resolve to deterministic-Light at runtime (the LLM enhances Light only — see Architecture); the config still parses and stores all four levels so the follow-on subsequence-guard plan activates Medium/High without a config change.

- [ ] **Step 1: Write the failing test**

Replace `src/config.rs` with:

```rust
use serde::{Deserialize, Serialize};
use talk_core::cleanup::{parse_level, Level};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub base_dir: Option<String>,
    pub default_mode: String,        // "reflect" | "journal"
    pub keep_raw: bool,
    pub auto_end_silence_seconds: u32, // 0 = off
    pub default_pack: String,
    pub reflect_cleanup: String,     // none | light | medium | high
    pub journal_cleanup: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            base_dir: None,
            default_mode: "reflect".into(),
            keep_raw: true,
            auto_end_silence_seconds: 0,
            default_pack: "spine".into(),
            reflect_cleanup: "light".into(),
            journal_cleanup: "medium".into(),
        }
    }
}

impl Config {
    pub fn load(text: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(text)
    }

    /// The cleanup `Level` for a mode ("journal" → journal_cleanup, else reflect).
    pub fn cleanup_for(&self, mode: &str) -> Level {
        let s = if mode == "journal" { &self.journal_cleanup } else { &self.reflect_cleanup };
        parse_level(s)
    }

    /// The fully-commented template `talk config init` writes.
    pub fn commented_template() -> String {
        let d = Config::default();
        format!(
            "# talk config — every line is optional; zero-config still launches.\n\
             # base_dir = \"~/talk\"          # where reflections land\n\
             default_mode = \"{mode}\"          # bare `talk` runs this\n\
             keep_raw = {keep}                 # store verbatim transcript in a hidden comment\n\
             auto_end_silence_seconds = {silence}  # 0 = off; you press space to finish\n\
             default_pack = \"{pack}\"\n\
             # cleanup levels: none · light · medium · high\n\
             reflect_cleanup = \"{rc}\"        # light: caps + punctuation + leading filler. \"um so i guess\" → \"I guess.\"\n\
             journal_cleanup = \"{jc}\"       # medium/high: deterministic-only in v1 (LLM enhances light); full LLM rewrite is future work\n",
            mode = d.default_mode, keep = d.keep_raw,
            silence = d.auto_end_silence_seconds, pack = d.default_pack,
            rc = d.reflect_cleanup, jc = d.journal_cleanup,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::cleanup::Level;

    #[test]
    fn zero_config_uses_defaults() {
        let c = Config::load("").unwrap();
        assert_eq!(c.default_mode, "reflect");
        assert!(c.keep_raw);
        assert_eq!(c.cleanup_for("reflect"), Level::Light);
        assert_eq!(c.cleanup_for("journal"), Level::Medium);
    }

    #[test]
    fn template_is_loadable() {
        let c = Config::load(&Config::commented_template()).unwrap();
        assert_eq!(c.auto_end_silence_seconds, 0);
        assert_eq!(c.cleanup_for("reflect"), Level::Light);
    }

    #[test]
    fn pins_override_defaults() {
        let c = Config::load("default_mode = \"journal\"\nkeep_raw = false\njournal_cleanup = \"high\"\n").unwrap();
        assert_eq!(c.default_mode, "journal");
        assert!(!c.keep_raw);
        assert_eq!(c.cleanup_for("journal"), Level::High);
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test config`
Expected: PASS (3 tests).

- [ ] **Step 3: Commit**

```bash
git add src/config.rs
git commit -m "feat: per-mode cleanup level in config (documented template)"
```

---

## Task 6: Wire the diff-guard into the session path [here-testable]

**Files:**
- Modify: `src/session.rs`
- Modify: `src/main.rs`

The Plan-1 roadmap requires `guard_accepts` wired into the session path **before** any LLM rewrite. Route `session::run`'s commit through `guarded_format` via an injected `&dyn Formatter` (defaulting to `DeterministicFormatter`). Behavior is byte-identical for the default (deterministic always passes the guard), but the seam now exists and is provably safe — an over-editing formatter cannot corrupt a file.

> **Depends on Task 5** (`Config::cleanup_for`) and Task 3 (`guarded_format`/`DeterministicFormatter`). Do T2–T5 first.

- [ ] **Step 1: Update session.rs**

In `src/session.rs`, change the imports and the `RunConfig`/`run` to thread a formatter + level. Replace the top of the file (imports + `RunConfig` + the `Event::Commit` arm) so it reads:

```rust
use crate::source::{Event, TranscriptSource};
use crate::writer::{write_entry, Target, WriteRequest};
use std::path::{Path, PathBuf};
use talk_core::cleanup::Level;
use talk_core::format::{guarded_format, Formatter};
use talk_core::settle::Settle;

pub struct RunConfig<'a> {
    pub base: &'a Path,
    pub date: &'a str,
    pub time: &'a str,
    pub keep_raw: bool,
    pub ephemeral: bool,
    pub formatter: &'a dyn Formatter,
    pub level: Level,
}
```

And the `Event::Commit` arm inside `run` becomes (the pre-layer now lives in `guarded_format`):

```rust
            Event::Commit(raw) => {
                let clean = guarded_format(cfg.formatter, cfg.level, &raw);
                settle.commit(&raw, &clean); // raw stored verbatim for recovery
            }
```

Update the test helper and the inline `RunConfig` in `session.rs`'s test module. The `cfg` helper:

```rust
    fn cfg(base: &Path, ephemeral: bool) -> RunConfig<'_> {
        RunConfig {
            base, date: "2026-06-08", time: "08:14", keep_raw: true, ephemeral,
            formatter: &talk_core::format::DeterministicFormatter, level: Level::Light,
        }
    }
```

The inline `RunConfig` in `spoken_command_words_do_not_survive` gains the two fields:

```rust
        let p = run(&mut src, Target::Journal, &RunConfig {
            base: dir.path(), date: "2026-06-08", time: "08:14", keep_raw: false, ephemeral: false,
            formatter: &talk_core::format::DeterministicFormatter, level: Level::Light,
        }).unwrap().unwrap();
```

Add a new test to `session.rs` proving the guard is wired in (an over-editing formatter still yields guard-safe file output):

```rust
    #[test]
    fn an_over_editing_formatter_cannot_corrupt_the_file() {
        struct Flip;
        impl talk_core::format::Formatter for Flip {
            fn format(&self, _l: Level, text: &str) -> String {
                format!(" {} ", text).replace(" love ", " hate ").trim().to_string()
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let mut src = FakeTranscript::new(vec![
            Event::Commit("i love this".into()),
            Event::Done,
        ]);
        let cfg = RunConfig {
            base: dir.path(), date: "2026-06-08", time: "08:14", keep_raw: true, ephemeral: false,
            formatter: &Flip, level: Level::Light,
        };
        let p = run(&mut src, Target::Journal, &cfg).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("hate"));             // the guard rejected the substitution
        assert!(text.to_lowercase().contains("love")); // your word survived
    }
```

- [ ] **Step 2: Update main.rs**

`run_and_report` must pass a formatter + the per-mode level. Change its signature and body, and each call site, to thread the resolved `Level`. Replace `run_and_report` with:

```rust
fn run_and_report(base: &Path, target: Target, date: &str, time: &str, text: &str, keep_raw: bool, ephemeral: bool, level: talk_core::cleanup::Level) -> std::io::Result<()> {
    let path = run(&mut FakeTranscript::from_text(text), target,
        &RunConfig {
            base, date, time, keep_raw, ephemeral,
            formatter: &talk_core::format::DeterministicFormatter, level,
        })?;
    if let Some(p) = path {
        println!("→ {}", p.display());
    }
    Ok(())
}
```

Update the four `run_and_report` call sites to pass the level:
- Journal (`Command::Journal`, line ~35): append `, cfg.cleanup_for("journal")`.
- Unburden/Vent (line ~39): append `, talk_core::cleanup::Level::None`. Ephemeral shows text momentarily then discards it — spending any formatting (let alone LLM inference, T9) on text that is never saved is wasted work. `guarded_format` with `Level::None` returns the pre-processed text directly (spoken commands + backtrack still apply), which is the right transient display for "this keeps nothing."
- Bare-`talk`-journal branch (line ~50): append `, cfg.cleanup_for("journal")`.
- Inside `reflect` (line ~88): append `, cfg.cleanup_for("reflect")`.

For example the `reflect` call becomes:

```rust
    run_and_report(base, target, date, time, text, cfg.keep_raw, false, cfg.cleanup_for("reflect"))?;
```

- [ ] **Step 3: Run the full suite**

Run: `cargo test`
Expected: PASS — every existing test plus the new `an_over_editing_formatter_cannot_corrupt_the_file`. Output text for the deterministic default is byte-identical, so the integration tests (`Keep avoiding it.`, etc.) still pass.

- [ ] **Step 4: Commit**

```bash
git add src/session.rs src/main.rs
git commit -m "feat: wire guarded_format seam into the session path (deterministic default)"
```

---

## Task 7: CandleFormatter — quantized 0.5B façade [needs your machine — model + Candle]

**Files:**
- Create: `src/format/mod.rs`
- Modify: `src/main.rs` (`#[cfg(feature = "formatter")] mod format;`)

The thin Candle façade behind the `formatter` feature: load the quantized Qwen2.5-0.5B GGUF + tokenizer, build a ChatML prompt from `cleanup::rewrite_prompt`, decode greedily (argmax = restraint), implement `talk_core::format::Formatter`. Pure compute — no network — so it doesn't weaken the zero-network session guarantee. Modeled on `candle-examples/examples/quantized-qwen2-instruct/main.rs` (the authoritative source for the pinned versions).

- [ ] **Step 1: Implement the façade**

Create `src/format/mod.rs`:

```rust
use std::path::Path;
use std::sync::Mutex;
use std::fs::File;
use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::quantized_qwen2::ModelWeights;
use tokenizers::Tokenizer;
use talk_core::cleanup::{rewrite_prompt, Level};
use talk_core::format::Formatter;

const MAX_NEW_TOKENS: usize = 96;

/// On-device constrained rewrite via a quantized Qwen2.5-0.5B-Instruct GGUF.
/// Restraint is enforced twice: argmax decoding (no sampling randomness) here, and
/// the content-word diff-guard at the call site (`guarded_format`). The model sits
/// behind a `Mutex` (its `forward` needs `&mut`) so `CandleFormatter` is `Send +
/// Sync` from the start — the live loop (T9) shares it with a worker thread via
/// `Arc<dyn Formatter + Send + Sync>`. One phrase formats at a time, so the lock is
/// uncontended.
pub struct CandleFormatter {
    model: Mutex<ModelWeights>,
    tokenizer: Tokenizer,
    device: Device,
    eos: u32,
}

impl CandleFormatter {
    /// `gguf` = quantized model file; `tokenizer` = Qwen2.5 tokenizer.json (a
    /// SEPARATE asset, not bundled in the GGUF). Both MUST be SHA-256 verified by
    /// the caller (T8) before this is constructed. Takes `&Path` (not `&str`) so a
    /// non-UTF-8 models dir can't force a `to_str().unwrap()` panic at startup.
    pub fn load(gguf: &Path, tokenizer: &Path) -> Result<CandleFormatter, String> {
        let device = Device::Cpu;
        let mut file = File::open(gguf).map_err(|e| e.to_string())?;
        let content = gguf_file::Content::read(&mut file).map_err(|e| e.to_string())?;
        let model = ModelWeights::from_gguf(content, &mut file, &device).map_err(|e| e.to_string())?;
        let tokenizer = Tokenizer::from_file(tokenizer).map_err(|e| e.to_string())?;
        let eos = tokenizer.get_vocab(true).get("<|im_end|>").copied()
            .ok_or_else(|| "tokenizer missing <|im_end|>".to_string())?;
        Ok(CandleFormatter { model: Mutex::new(model), tokenizer, device, eos })
    }

    fn generate(&self, level: Level, text: &str) -> Result<String, String> {
        let p = rewrite_prompt(level, text);
        let prompt = format!(
            "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
            p.system, p.user
        );
        let ids: Vec<u32> = self.tokenizer.encode(prompt, true).map_err(|e| e.to_string())?
            .get_ids().to_vec();

        let mut model = self.model.lock().unwrap();
        // No clear_kv_cache() — absent in candle-transformers 0.9.x. The prefill
        // forward(&input, 0) below resets the KV cache, so each phrase starts clean.
        let mut lp = LogitsProcessor::from_sampling(0, Sampling::ArgMax);

        // Prefill the whole prompt, then decode one token at a time.
        let input = Tensor::new(ids.as_slice(), &self.device).map_err(|e| e.to_string())?
            .unsqueeze(0).map_err(|e| e.to_string())?;
        let logits = model.forward(&input, 0).map_err(|e| e.to_string())?
            .squeeze(0).map_err(|e| e.to_string())?;
        let mut next = lp.sample(&logits).map_err(|e| e.to_string())?;
        let mut pos = ids.len();
        let mut out = Vec::new();
        while next != self.eos && out.len() < MAX_NEW_TOKENS {
            out.push(next);
            let input = Tensor::new(&[next], &self.device).map_err(|e| e.to_string())?
                .unsqueeze(0).map_err(|e| e.to_string())?;
            let logits = model.forward(&input, pos).map_err(|e| e.to_string())?
                .squeeze(0).map_err(|e| e.to_string())?;
            next = lp.sample(&logits).map_err(|e| e.to_string())?;
            pos += 1;
        }
        self.tokenizer.decode(&out, true).map_err(|e| e.to_string())
    }
}

impl Formatter for CandleFormatter {
    fn format(&self, level: Level, text: &str) -> String {
        // On any inference error, return the input unchanged: the diff-guard accepts
        // identity, deterministic-Light already settled, so a model failure NEVER
        // blocks or corrupts the save (spec §13).
        self.generate(level, text).unwrap_or_else(|_| text.to_string())
    }
}
```

Add to `src/main.rs` (with the other `mod` lines): `#[cfg(feature = "formatter")] mod format;`

> Confirm against the pinned `quantized-qwen2-instruct` example: `ModelWeights::from_gguf`, `forward(&input, pos)` returning **last-position** logits of shape `(1, vocab)` (hence `squeeze(0)`), `LogitsProcessor::from_sampling(seed, Sampling::ArgMax)`, and `tokenizer.get_vocab(true)`/`decode(skip_special=true)`. If `forward` returns full-sequence logits in your version, take the last row before `squeeze`. (`clear_kv_cache` is intentionally **not** called — it doesn't exist at 0.9.x; the `index_pos == 0` prefill resets the cache.)

- [ ] **Step 2: Build with the feature**

Run: `cargo build --features formatter` and a bare `cargo build` (no features).
Expected: both compile cleanly on macOS. There are no unit tests for the inference path (it needs the model); its restraint is covered by the eval set (T4) running against mocks, and its real behavior is checked in Step 3.

- [ ] **Step 3: Smoke-test against a real phrase (your machine)**

Add a temporary `examples/format_probe.rs` that **verifies the assets first** (`download::verify(path, sha)` — never load unverified weights, per `CandleFormatter::load`'s own contract), then constructs `CandleFormatter::load(Path::new(gguf), Path::new(tok))` and prints `guarded_format(&fmt, Level::Light, "um so the thing i keep avoiding is the hard conversation")`. Run `cargo run --features formatter --example format_probe`. Confirm the output is clean, meaning-preserving prose (e.g. "The thing I keep avoiding is the hard conversation."), and that a deliberately tricky negation ("i never said i was fine") keeps its negation. **Also record `talk_core::eval::score(&fmt, Level::Light, talk_core::eval::FIXTURES)`** — the one CI-uncovered measurement of the *real* model's restraint; it should be `1.0`. Note the number in the Self-Review. Delete the example after.

- [ ] **Step 4: Commit**

```bash
git add src/format/mod.rs src/main.rs
git commit -m "feat(format): CandleFormatter — quantized Qwen2.5-0.5B constrained rewrite"
```

---

## Task 8: Formatter model download + integrity [network]

**Files:**
- Modify: `src/download/models.rs` (add `FORMATTER_MODELS`)
- Modify: `src/main.rs` (fetch + load-time verify, gated on the `formatter` feature)

Per spec §11: the formatter model is fetched once, **pinned SHA-256 verified before load**, then offline forever. Reuse the Plan-2 `download::fetch`/`verify` gate verbatim.

> **HF redirect caveat (decide here).** Plan 2's `download::fetch` uses `.redirects(0)` (a redirect could downgrade HTTPS). HuggingFace `resolve/main` URLs **302-redirect to a CDN**, so they will fail that gate. **Recommended:** re-host the GGUF + tokenizer.json as assets on the project's own GitHub release (redirect-free, and it satisfies §11's "model host + pinned versions live in a lockfile so a fetch is reproducible") and pin those URLs. **Alternative:** relax `fetch` to follow redirects but refuse any non-`https://` redirect target — the pinned SHA-256 is the real integrity guarantee, so an HTTPS→HTTPS redirect can't substitute weights.
>
> **Decision (recorded): option A — re-host on the project GitHub release.** The `FORMATTER_MODELS` URLs below are redirect-free release assets; HF `resolve/main` URLs must **not** be used directly (they fail Plan 2's `redirects(0)` gate). This keeps the `download::fetch` redirect policy unchanged and satisfies spec §11's reproducible-lockfile intent. If a future asset must come from a redirecting host, that is a deliberate policy change to `fetch` (the alternative above), not a silent per-URL workaround.

- [ ] **Step 1: Add the formatter manifest**

Add to `src/download/models.rs` (the `Artifact` struct + `download::fetch`/`verify` already exist from Plan 2 T9):

```rust
/// Formatter model (behind the `formatter` feature): quantized Qwen2.5-0.5B GGUF
/// (~491 MB) + its tokenizer.json (a separate asset). URLs are the re-hosted,
/// redirect-free project release assets; HASHES are pinned at fetch time (Step 3).
pub const FORMATTER_MODELS: &[Artifact] = &[
    Artifact {
        name: "qwen2.5-0.5b-instruct-q4_k_m.gguf",
        url: "https://github.com/walktalkmeditate/talk-cli/releases/download/models-v1/qwen2.5-0.5b-instruct-q4_k_m.gguf",
        sha256: "FILL_AT_PIN_TIME",
    },
    Artifact {
        name: "qwen2.5-0.5b-instruct-tokenizer.json",
        url: "https://github.com/walktalkmeditate/talk-cli/releases/download/models-v1/qwen2.5-0.5b-instruct-tokenizer.json",
        sha256: "FILL_AT_PIN_TIME",
    },
];
```

- [ ] **Step 2: Fetch + load-time verify in main.rs**

In `src/main.rs`, extend the `talk download models` handler so that, when the `formatter` feature is on, it also fetches `FORMATTER_MODELS`:

```rust
        #[cfg(feature = "formatter")]
        for art in download::models::FORMATTER_MODELS {
            download::fetch(art, &paths::models_dir()).map_err(|e|
                std::io::Error::new(std::io::ErrorKind::Other, e))?;
        }
```

Add a helper that returns the verified formatter paths (used by the live loop in T9), refusing to run on a hash mismatch — the load-time gate, not just download-time:

```rust
/// Verify both formatter assets and return (gguf_path, tokenizer_path). Errors if
/// a file is missing or fails its pinned SHA-256 — never load an unverified model.
#[cfg(feature = "formatter")]
fn verified_formatter_paths() -> Result<(PathBuf, PathBuf), String> {
    let dir = paths::models_dir();
    let mut paths = Vec::new();
    for art in download::models::FORMATTER_MODELS {
        let p = dir.join(art.name);
        if !download::verify(&p, art.sha256)? {
            return Err(format!("{} failed integrity check — run `talk download models`", art.name));
        }
        paths.push(p);
    }
    Ok((paths[0].clone(), paths[1].clone()))
}
```

- [ ] **Step 3: Pin the hashes + real fetch (your machine)**

Re-host the two assets on the project release (or choose the redirect alternative). Run `talk download models --features formatter` once — it fails the checksum with `FILL_AT_PIN_TIME`; take each reported `got <hash>`, paste into `FORMATTER_MODELS`, re-run until it verifies and keeps the files. Commit the real hashes.

- [ ] **Step 4: Commit**

```bash
git add src/download/models.rs src/main.rs
git commit -m "feat(download): pinned-SHA-256 fetch + load-time verify for the formatter model"
```

---

## Task 9: Async LLM swap in the live loop [needs your machine — Plan 2 audio half + model + mic]

**Files:**
- Modify: `src/live.rs` (the Plan-2 T11 interactive loop)
- Modify: `src/main.rs` (construct the formatter when feature + verified model present)

Wire the async upgrade into the live loop. On each committed phrase, deterministic-Light settles **instantly** (unchanged); then the formatter runs on a **worker thread**, and *iff* its result returns inside the swap window **and** passes the content-word guard, the committing block is upgraded in place via `settle::upgrade_committing`. Settled blocks are never touched (the never-re-flow invariant). The pure decision (`guard_accepts`) is already tested (T3); this task is the concurrent/timed integration, verified on your machine.

> **Depends on Plan 2's audio half.** `src/live.rs` is the Plan-2 T11 loop (built on your machine). The anchors below refer to that file. Do this task only after Plan 2's audio half is merged.

> **Swap window from the spike.** Use the window from Task 1: `~250 ms` if p95 fit it, else the measured p95 (the committing block may linger; only settled text is immutable). Set the `SWAP_WINDOW` constant accordingly.

- [ ] **Step 1: Add a formatter handle to the loop**

In `src/live.rs`, extend `LiveConfig` with an optional formatter and the level. The loop stays compilable without the `formatter` feature — when `formatter` is `None` (or `level != Light`), no swap happens and deterministic-Light is the final text. `CandleFormatter` is already `Send + Sync` (T7 uses a `Mutex`), so **no change to `src/format/mod.rs` is needed here**.

```rust
// From the T1 spike. If measured p95 > 250 ms, set this to that p95: the committing
// block may linger longer (only SETTLED text is immutable), so a slower swap still
// lands without ever re-flowing settled text. If p95 is multiple seconds, take the
// T1 no-go branch (the realtime swap is disabled and this constant goes unused).
const SWAP_WINDOW: Duration = Duration::from_millis(250);

pub struct LiveConfig<'a> {
    pub mode: RMode,
    pub question: Option<&'a str>,
    pub held_label: Option<&'a str>,
    pub cleanup: &'a str,
    pub ephemeral: bool,
    pub level: talk_core::cleanup::Level,
    /// Some(CandleFormatter) with the `formatter` feature + a verified model; None
    /// otherwise → deterministic-Light only. Arc + Send + Sync so the single worker
    /// thread (Step 2) can hold it.
    pub formatter: Option<std::sync::Arc<dyn talk_core::format::Formatter + Send + Sync>>,
}
```

- [ ] **Step 2: One formatter worker; upgrade the committing block on guard-pass**

The async swap uses a **single long-lived worker thread** — not one thread per phrase (which leaked threads and kept inference running after cancel). The worker owns the `Arc<Formatter>`, reads jobs from a channel, and **drains to the newest queued job** before each inference so a fast talker can't build a backlog and stale phrases are skipped. The loop matches results to the current committing block by a monotonic `id`, so a late result for a superseded phrase is dropped.

Near the top of `run_loop`, before the main loop, set up the worker (only when there's a real formatter to run):

```rust
    struct Job { id: u64, level: talk_core::cleanup::Level, pre: String }
    struct Pending { id: u64, pre: String, deadline: Instant }

    let (job_tx, res_rx) = match cfg.formatter.clone() {
        Some(fmt) => {
            let (jtx, jrx) = std::sync::mpsc::channel::<Job>();
            let (rtx, rrx) = std::sync::mpsc::channel::<(u64, String)>();
            std::thread::spawn(move || {
                // recv() ends (Err) when job_tx drops at run_loop return → clean exit.
                while let Ok(first) = jrx.recv() {
                    let mut job = first;
                    while let Ok(newer) = jrx.try_recv() { job = newer; } // newest-wins: skip stale
                    let out = fmt.format(job.level, &job.pre);
                    if rtx.send((job.id, out)).is_err() { break; } // loop gone
                }
            });
            (Some(jtx), Some(rrx))
        }
        None => (None, None),
    };
    let mut pending: Option<Pending> = None;
    let mut next_id: u64 = 0;
```

> Shutdown is implicit and non-blocking: when `run_loop` returns, `job_tx` drops, the worker's `recv()` errors, and the worker exits after finishing at most its current inference (≤ one phrase's latency). It holds only an `Arc` clone of the model, released on exit — so quitting is instant and nothing leaks. No `JoinHandle` to manage.

Replace the `Event::Commit` arm so it settles deterministic-Light instantly **and** queues the async rewrite — **at Light only** (the one level the guard accepts; Medium/High and ephemeral's `Level::None` never queue):

```rust
                Event::Commit(raw) => {
                    let pre = talk_core::cleanup::apply_backtrack(
                        &talk_core::cleanup::apply_spoken_commands(&raw));
                    let clean = talk_core::cleanup::deterministic_light(&pre);
                    settle.commit(&raw, &clean); // instant, locked-in text

                    if let (Some(jtx), talk_core::cleanup::Level::Light) = (job_tx.as_ref(), cfg.level) {
                        next_id += 1;
                        let _ = jtx.send(Job { id: next_id, level: cfg.level, pre: pre.clone() });
                        pending = Some(Pending { id: next_id, pre, deadline: Instant::now() + SWAP_WINDOW });
                    }
                }
```

In the per-tick body (after draining transcript events, before painting), apply any ready result that still matches the current committing block and beat its window:

```rust
        // Async LLM swap: upgrade iff the result is for the CURRENT pending block,
        // arrived within its window, and passes the content-word guard.
        if let Some(rx) = res_rx.as_ref() {
            while let Ok((id, candidate)) = rx.try_recv() {
                if pending.as_ref().is_some_and(|p| p.id == id) {
                    let p = pending.take().unwrap();
                    if Instant::now() <= p.deadline
                        && talk_core::cleanup::guard_accepts(&p.pre, &candidate) {
                        settle.upgrade_committing(&candidate); // no-op if already finalized
                    }
                }
                // id != current pending → a superseded phrase's result; drop it.
            }
        }
```

> **Newest-wins, and the dropped rewrite is intentional.** When a new phrase commits before the previous phrase's rewrite returns, `pending` is reassigned to the new block. The previous block was already finalized by the new `settle.commit` (→ `finalize`), so even if its rewrite arrived `upgrade_committing` would no-op — and the id check drops it anyway. Only the most-recent committing block is ever a swap candidate; superseded rewrites are discarded by design (they'd have nowhere to land). The worker's drain-to-newest means a backlog never builds.

> **The last phrase must still get its chance (finish path).** Plan 2's `drain_until_done` (run after `[space]`) loops on `source.next()` and never reads `res_rx`, so without this the final phrase — the one the user is watching — could never upgrade. In the `Action::Finish` path, **after** `drain_until_done` returns and **before** the closing `settle.finalize()`, give the outstanding `pending` a bounded chance:
> ```rust
>         // Last phrase: wait up to one window for its rewrite before finalizing.
>         if let (Some(p), Some(rx)) = (pending.take(), res_rx.as_ref()) {
>             let wait_until = Instant::now() + SWAP_WINDOW;
>             while Instant::now() < wait_until {
>                 match rx.try_recv() {
>                     Ok((id, candidate)) if id == p.id => {
>                         if talk_core::cleanup::guard_accepts(&p.pre, &candidate) {
>                             settle.upgrade_committing(&candidate);
>                         }
>                         break;
>                     }
>                     Ok(_) => {}                                   // a superseded result; ignore
>                     Err(_) => std::thread::sleep(Duration::from_millis(10)),
>                 }
>             }
>         }
> ```
> If you'd rather keep finish instant, document explicitly instead that the last phrase always stays deterministic-Light — but do not leave it silently broken.

- [ ] **Step 3: Construct the formatter in main.rs**

In `src/main.rs`, where the live session is built (Plan 2 T11, when `--from-text` is absent and `listen` is on), construct the formatter and pass it + the level into `LiveConfig`:

```rust
    #[cfg(feature = "formatter")]
    let formatter: Option<std::sync::Arc<dyn talk_core::format::Formatter + Send + Sync>> =
        match verified_formatter_paths() {
            Ok((gguf, tok)) => match format::CandleFormatter::load(&gguf, &tok) {
                Ok(f) => Some(std::sync::Arc::new(f)),
                Err(e) => { eprintln!("formatter disabled: {e}"); None }
            },
            Err(_) => None, // model not downloaded → deterministic-Light only (no hard failure)
        };
    #[cfg(not(feature = "formatter"))]
    let formatter: Option<std::sync::Arc<dyn talk_core::format::Formatter + Send + Sync>> = None;
```

(`gguf`/`tok` are `PathBuf` from `verified_formatter_paths`; `load` takes `&Path`, so pass them by reference — no `to_str().unwrap()` that would panic on a non-UTF-8 models dir.) Pass `formatter` and the per-mode `level` into the `LiveConfig` the loop receives: reflect → `cfg.cleanup_for("reflect")` (Light), journal → `cfg.cleanup_for("journal")` (Medium → deterministic, no LLM), and **ephemeral/unburden → `Level::None`** (no formatting on discarded text, matching T6). A missing/disabled formatter degrades gracefully to deterministic-Light — never a crash.

- [ ] **Step 4: End-to-end on your machine**

```
talk download models     # now also fetches the formatter model (feature on)
talk reflect             # speak a messy sentence with filler, pause
```
Confirm: the phrase settles **instantly** in deterministic-Light, then within the swap window visibly **upgrades** to the prettier LLM version *without the settled text above it moving*; a phrase the LLM would over-edit keeps your words (guard fallback); finishing fast (before the swap) leaves clean deterministic-Light text permanently with no late pop-in. Verify `u` still toggles raw⇄clean, and that `cargo test` (the `--from-text` deterministic paths) all still pass. Time a few phrases against your T1 numbers — if the upgrade rarely lands, widen `SWAP_WINDOW` to your measured p95.

- [ ] **Step 5: Commit**

```bash
git add src/live.rs src/main.rs src/format/mod.rs
git commit -m "feat: async LLM swap in the live loop (instant deterministic, guarded upgrade)"
```

---

## Self-Review (completed during authoring)

- **Spec coverage (Plan-3 scope):** formatter policy + prompt in `talk-core` §10 (T2) · the `Formatter` seam + diff-guarded `guarded_format` moat (content-word preservation, fail-safe to deterministic-Light/raw) §10 (T3) · the checked-in **eval set** as a regression harness for known meaning-flip classes (faithful-green / over-editing-red / guard-makes-safe), with the real model spot-checked on-machine §10/§14 (T4) · per-mode cleanup level in config with a documented example diff §7/§12 (T5) · guard wired into the session path (the Plan-1 roadmap prerequisite) §10 (T6) · the **Candle 0.5B façade** behind a `formatter` feature, argmax decode, identity-on-error fallback §10/§13 (T7) · model fetch with **pinned SHA-256 verify-before-load** §11 (T8) · async swap via `settle::upgrade_committing` inside the window, settled text never re-flows, instant deterministic-Light layer §7 (T9) · the **latency spike** gating the realtime path §5/§7 (T1). **Scope note (resolved):** the LLM rewrite ships at **Light only** this plan; Medium/High are deterministic-only until a level-aware subsequence guard lands (deferred). **Correctly deferred to Plan 4:** real spine + flagship packs, ephemeral zeroize/mlock, sidecar raw store, streak/thread views, the model-fetch UI, and the **sandboxed no-egress + model-tamper + ephemeral-zero-bytes tests — which MUST run with AND without `--features formatter` (both emit zero outbound connections) and the tamper test MUST cover `FORMATTER_MODELS`** (the link-time check misses runtime HTTP, so the formatter deps need the runtime no-egress test). The roadmap's stale `upgrade_block` is corrected to `upgrade_committing` throughout.
- **Product framing (spec carried this forward to planning):** the formatter is a **quality/satisfaction bet**, not the primary retention lever — the spec flags the 2–3-entry drop-off as the real success case, and that lever is question selection + the held-thread feel (Plan 4), not prettier text. Plan 3 keeps the bet's surface small and honest: deterministic-Light is always the instant, always-present experience; the LLM is an **opt-in, Light-only, off-by-default** enhancement that can never block or corrupt the save. The 491MB model + the solo-maintainer re-hosting/re-pinning obligation (T8) are accepted costs of an opt-in feature, not imposed on the default binary.
- **Placeholder scan:** the only deferrals are the `sha256: "FILL_AT_PIN_TIME"` values (physically unknowable until the asset is hashed — T8 pins them in one command; the verify code is complete and the FILL guard refuses to load until pinned), the **recorded spike numbers** (T1 — hardware-only), and the **on-machine real-model eval score** (T7 Step 3). The Candle symbols are sourced from the `quantized-qwen2-instruct` example at the pinned tag — version-sensitive bindings, not logic placeholders. **To record on-machine: spike p50/p95 (M1 / x86) in T1 Step 4; `score(&CandleFormatter, Light, FIXTURES)` in T7 Step 3.**
- **Type consistency:** `Level` (live — `parse_level`/`rewrite_prompt`/`cleanup_for`/`guarded_format`/`score`/`CandleFormatter`/`LiveConfig` all use the same enum) · `RewritePrompt { system, user }` produced by `cleanup::rewrite_prompt`, consumed by `CandleFormatter::generate` · the `Formatter` trait (`format(&self, Level, &str) -> String`) implemented by `DeterministicFormatter`, the eval mocks, the `session.rs` `Flip` test, and `CandleFormatter` · `guarded_format(&dyn Formatter, Level, &str)` is the entry point for `session::run`; `live::run_loop` runs the same `cleanup` pre-layer + `guard_accepts` inline (async split) · `CandleFormatter` is `Mutex<ModelWeights>` (Send + Sync) and `load(&Path, &Path)` from the start — no T7→T9 churn, no startup panic · `settle::upgrade_committing(&str) -> bool` (no-op after finalize) drives the swap · `RunConfig`/`LiveConfig` gain `formatter` + `level` consistently · `download::{Artifact, fetch, verify}` (Plan 2) reused for `FORMATTER_MODELS`, and the `formatter` feature pulls in `download` so the verify path compiles. Candle pins share one minor (0.9.2); `clear_kv_cache` is not called (absent at 0.9.x; prefill resets the cache).
- **Execution venue:** T2–T6 are pure `talk-core` + config + the session seam — built and unit-tested in CI here (bare `cargo build` and `cargo test` with no features). T1, T7–T9 need the GGUF model, CPU inference, the Plan-2 audio half, and a mic; verified on your machine via the named checks. The `formatter` feature is off by default, so the lean binary and the zero-network session guarantee are preserved (Candle + tokenizers are compute-only, audited via `cargo tree` in T1; the model fetch lives behind `download`).

## Review decisions (resolved 2026-06-09)

A multi-persona `/ce-doc-review` (coherence, feasibility, product, security, scope, adversarial) ran on the draft and surfaced 3 P0s + ~17 more; all were auto-resolved with best judgment:

- **P0 — guard vs Medium/High** (adversarial): the content-word guard rejects every Medium/High rewrite by design. **Resolved:** the LLM ships at **Light only**; Medium/High are deterministic-only; the level-aware subsequence guard is deferred (Architecture + T2/T3/T5/T9).
- **P0 — `clear_kv_cache` absent** at candle-transformers 0.9.x (feasibility): **removed** the calls; the `index_pos == 0` prefill resets the cache (T1, T7).
- **P0 — Candle version skew** (feasibility): `candle-transformers` pinned to **0.9.2** to match `candle-core` 0.9.2 (T1).
- **P1s:** test asserted `"I keep avoiding it."` → `"Keep avoiding it."` (T3); `CandleFormatter` is **`Mutex` from the start** (T7, no T7→T9 churn); T6's dependency on T5 declared; **single drain-to-newest worker** replaces per-phrase spawn (T9, no thread leak / post-cancel inference); **finish-path polls `pending`** so the last phrase can upgrade (T9); `tokenizers` network surface audited via `cargo tree` (T1); smoke-probe **verifies before load** (T7); product 2–3-entry framing acknowledged (above).
- **P2/P3:** `formatter` feature pulls in `download` (T1); ephemeral runs **`Level::None`** (T6/T9); T3's "consolidation" claim scoped to the session path; the stale-rewrite drop documented (T9); `SWAP_WINDOW` tied to the spike (T9); eval "test that can fail" reframed honestly (T4); no-egress test must run `--features formatter` (above); HF-redirect **decision recorded** (option A, T8); `load` takes `&Path` (no non-UTF-8 panic, T7).

---

## Execution Handoff

Two execution options (per superpowers:writing-plans):

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, spec + quality review between tasks. Here that means executing **T2–T6** (CI-testable) now and handing **T1, T7–T9** to your machine with the exact on-machine checks above.

**2. Inline Execution** — execute T2–T6 in this session with checkpoints.

Consistent with the review-gated workflow, I recommend running `/ce-doc-review` on this plan **before** building (it caught ~30 issues including blockers on Plan 2). Which would you like?
