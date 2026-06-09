# talk-cli Formatter + Restraint Implementation Plan (Plan 3 of 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the optional on-device LLM formatter — a quantized 0.5B model that prettifies a settled phrase — gated so hard by content-word restraint (and an instant deterministic fallback) that it can never change what you said.

**Architecture:** `talk-core` owns the restraint **policy** (the `Formatter` trait, the per-level prompt, the always-safe `DeterministicFormatter`, the diff-guarded `guarded_format` call site, and the checked-in **eval set**) — all pure and CI-tested. The binary's `src/format/` is a thin **Candle** façade (`CandleFormatter`) behind a cargo `formatter` feature that loads a quantized Qwen2.5-0.5B-Instruct GGUF and implements that same trait. At runtime deterministic-Light still settles **instantly**; the LLM runs async on a worker thread and, *iff* it returns inside the swap window **and** passes the content-word guard, replaces the committing block via `settle::upgrade_committing` (never re-flowing settled text). Miss the window or fail the guard → deterministic-Light stays, permanently.

**Tech Stack:** Rust 1.82; `candle-core` + `candle-transformers` (pure-Rust ML, quantized Qwen2 GGUF, CPU) + `tokenizers` (behind the `formatter` feature); the Plan-2 `download` machinery (`ureq` + `sha2`) for the model assets. No new network surface — Candle is compute-only.

**Origin spec:** `docs/superpowers/specs/2026-06-08-talk-cli-design.md` (§7 settle/async-swap, §10 formatter & restraint, §11 model integrity, §14 eval set). **Roadmap source:** `docs/superpowers/plans/2026-06-08-talk-cli-foundation.md` (Plan 3 entry). Note the roadmap's `upgrade_block` is stale — the real method is `settle::upgrade_committing`.

---

## Execution venue (read first)

This plan splits exactly like Plan 2 did:

- **CI here (pure / no model):** Tasks **2–6** build and unit-test `talk-core`'s formatter policy + eval set, the config cleanup-level wiring, and the diff-guard seam in the non-interactive `session::run` path. They need no Candle, no model, no mic — `cargo test` proves them. They are **independent of the latency spike's result** and can land immediately.
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
# Add to the existing [features] table:
formatter = ["dep:candle-core", "dep:candle-transformers", "dep:tokenizers"]

# Add to [dependencies]:
candle-core = { version = "0.9.2", optional = true }          # crates.io stable; main is ahead (0.10.x) — pin to a released tag
candle-transformers = { version = "0.8.3", optional = true }  # provides models::quantized_qwen2
tokenizers = { version = "0.23.1", optional = true }          # Tokenizer::from_file
```

> **Pin step (do this first, on your machine):** confirm the latest released `candle-core`/`candle-transformers`/`tokenizers` on crates.io and that they build on Rust 1.82 (neither declares an MSRV; 1.82 is known-good). Pin exact patch versions and record them in a comment. Read `candle-examples/examples/quantized-qwen2-instruct/main.rs` at the matching tag — it is the authoritative reference for every Candle symbol in this plan. Use `quantized_qwen2`, **not** `quantized_llama` (the latter has a wrong-RoPE issue for Qwen2, k2/candle #3410).

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
    let eos = *tokenizer.get_vocab(true).get("<|im_start|>assistant").unwrap_or(&0);
    let _ = eos;
    let eos = *tokenizer.get_vocab(true).get("<|im_end|>").ok_or("no <|im_end|>")?;
    println!("cold load: {} ms", load0.elapsed().as_millis());

    let system = "You clean up raw voice transcripts. Return ONLY the cleaned text. NEVER change meaning. Fix only capitalization and punctuation, and drop leading filler.";
    let mut times = Vec::new();
    for phrase in PHRASES {
        let prompt = format!(
            "<|im_start|>system\n{system}<|im_end|>\n<|im_start|>user\nClean this transcript:\n{phrase}<|im_end|>\n<|im_start|>assistant\n");
        let ids: Vec<u32> = tokenizer.encode(prompt, true).map_err(|e| e.to_string())?.get_ids().to_vec();

        let t0 = Instant::now();
        model.clear_kv_cache();
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

> Confirm `ModelWeights::from_gguf(content, &mut file, &device)`, `model.forward(&input, pos)` (returns last-position logits), `model.clear_kv_cache()`, and `LogitsProcessor::from_sampling(seed, Sampling::ArgMax)` against the pinned `quantized-qwen2-instruct` example. ArgMax = deterministic decode (restraint: no sampling randomness).

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

The seam every formatter implements, plus `guarded_format` — the moat applied at the **call site**: run the deterministic pre-layer (spoken commands, backtrack), then the formatter, then **accept the formatter's output only if the content-word guard passes**, else fall back to deterministic-Light. The returned text is *always* guard-safe, so a hallucinating LLM can never change meaning in a file. This consolidates the pre-layer that Plan 1 inlined into `session.rs` (and Plan 2 into `live.rs`) into one place.

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
        assert_eq!(guarded_format(&Faithful, Level::Light, "um so i keep avoiding it"), "I keep avoiding it.");
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

The checked-in eval set makes "the formatter never changes meaning" a **falsifiable** CI gate (spec §10/§14). Each fixture lists impermissible substrings a faithful cleanup must never introduce; `score` returns the fraction with zero impermissible edits. A faithful formatter scores **green (1.0)**; a deliberately over-editing mock **must score red (<1.0)** — that test is the proof the metric can fail. A third test shows the runtime guard makes even the bad mock safe (belt + suspenders).

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

Spec §12: config pins a per-mode cleanup level; §7: the template documents each level with a one-line example so the choice is informed. Reflect defaults Light, journal defaults Medium.

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
             journal_cleanup = \"{jc}\"       # medium: + disfluencies/false-starts removed, fragments joined\n",
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

Update the four `run_and_report` call sites to pass the level from config:
- Journal (`Command::Journal`, line ~35): append `, cfg.cleanup_for("journal")`.
- Unburden/Vent (line ~39): append `, cfg.cleanup_for("journal")`.
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
use std::cell::RefCell;
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
/// the content-word diff-guard at the call site (`guarded_format`). One session =
/// one thread, so `RefCell` around the model (whose `forward` needs `&mut`) is fine
/// and keeps `format(&self, …)` matching the trait.
pub struct CandleFormatter {
    model: RefCell<ModelWeights>,
    tokenizer: Tokenizer,
    device: Device,
    eos: u32,
}

impl CandleFormatter {
    /// `gguf` = quantized model file; `tokenizer` = Qwen2.5 tokenizer.json (a
    /// SEPARATE asset, not bundled in the GGUF). Both MUST be SHA-256 verified by
    /// the caller (T8) before this is constructed.
    pub fn load(gguf: &str, tokenizer: &str) -> Result<CandleFormatter, String> {
        let device = Device::Cpu;
        let mut file = File::open(gguf).map_err(|e| e.to_string())?;
        let content = gguf_file::Content::read(&mut file).map_err(|e| e.to_string())?;
        let model = ModelWeights::from_gguf(content, &mut file, &device).map_err(|e| e.to_string())?;
        let tokenizer = Tokenizer::from_file(tokenizer).map_err(|e| e.to_string())?;
        let eos = tokenizer.get_vocab(true).get("<|im_end|>").copied()
            .ok_or_else(|| "tokenizer missing <|im_end|>".to_string())?;
        Ok(CandleFormatter { model: RefCell::new(model), tokenizer, device, eos })
    }

    fn generate(&self, level: Level, text: &str) -> Result<String, String> {
        let p = rewrite_prompt(level, text);
        let prompt = format!(
            "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
            p.system, p.user
        );
        let ids: Vec<u32> = self.tokenizer.encode(prompt, true).map_err(|e| e.to_string())?
            .get_ids().to_vec();

        let mut model = self.model.borrow_mut();
        model.clear_kv_cache(); // each phrase is independent
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

> Confirm against the pinned `quantized-qwen2-instruct` example: `ModelWeights::from_gguf`, `forward(&input, pos)` returning **last-position** logits of shape `(1, vocab)` (hence `squeeze(0)`), `clear_kv_cache()`, `LogitsProcessor::from_sampling(seed, Sampling::ArgMax)`, and `tokenizer.get_vocab(true)`/`decode(skip_special=true)`. If `forward` returns full-sequence logits in your version, take the last row before `squeeze`.

- [ ] **Step 2: Build with the feature**

Run: `cargo build --features formatter` and a bare `cargo build` (no features).
Expected: both compile cleanly on macOS. There are no unit tests for the inference path (it needs the model); its restraint is covered by the eval set (T4) running against mocks, and its real behavior is checked in Step 3.

- [ ] **Step 3: Smoke-test against a real phrase (your machine)**

Add a temporary `examples/format_probe.rs` that constructs `CandleFormatter::load(gguf, tok)` and prints `guarded_format(&fmt, Level::Light, "um so the thing i keep avoiding is the hard conversation")`. Run `cargo run --features formatter --example format_probe`. Confirm the output is clean, meaning-preserving prose (e.g. "The thing I keep avoiding is the hard conversation."), and that a deliberately tricky negation ("i never said i was fine") keeps its negation. Delete the example.

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

> **HF redirect caveat (decide here).** Plan 2's `download::fetch` uses `.redirects(0)` (a redirect could downgrade HTTPS). HuggingFace `resolve/main` URLs **302-redirect to a CDN**, so they will fail that gate. **Recommended:** re-host the GGUF + tokenizer.json as assets on the project's own GitHub release (redirect-free, and it satisfies §11's "model host + pinned versions live in a lockfile so a fetch is reproducible") and pin those URLs. **Alternative:** relax `fetch` to follow redirects but refuse any non-`https://` redirect target — the pinned SHA-256 is the real integrity guarantee, so an HTTPS→HTTPS redirect can't substitute weights. Pick one and note it; the manifest below assumes re-hosted URLs.

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

In `src/live.rs`, extend `LiveConfig` with an optional formatter and the level (the loop stays compilable without the `formatter` feature — when `None`, no swap happens and deterministic-Light is the final text):

```rust
const SWAP_WINDOW: Duration = Duration::from_millis(250); // from the T1 spike; widen to measured p95 if needed

pub struct LiveConfig<'a> {
    pub mode: RMode,
    pub question: Option<&'a str>,
    pub held_label: Option<&'a str>,
    pub cleanup: &'a str,
    pub ephemeral: bool,
    pub level: talk_core::cleanup::Level,
    /// The async formatter (Some(CandleFormatter) with the feature + a verified
    /// model; None otherwise → deterministic-Light only). Arc so the worker thread
    /// can hold it; the trait object is Send + Sync because CandleFormatter's fields
    /// are (model behind a Mutex — see below).
    pub formatter: Option<std::sync::Arc<dyn talk_core::format::Formatter + Send + Sync>>,
}
```

> `CandleFormatter` as written (T7) uses `RefCell`, which is `!Sync`. For the worker thread, swap that `RefCell<ModelWeights>` for `std::sync::Mutex<ModelWeights>` and `borrow_mut()` → `lock().unwrap()` so the type is `Send + Sync`. (One phrase formats at a time, so the Mutex is uncontended.) Make this change in `src/format/mod.rs` as part of this task.

- [ ] **Step 2: Spawn the formatter per phrase; upgrade on guard-pass**

In `run_loop`, replace the `Event::Commit` arm so it settles deterministic-Light instantly **and** kicks off the async rewrite. Add a per-phrase results channel and an "in-flight" record. The commit arm:

```rust
                Event::Commit(raw) => {
                    let pre = talk_core::cleanup::apply_backtrack(
                        &talk_core::cleanup::apply_spoken_commands(&raw));
                    let clean = talk_core::cleanup::deterministic_light(&pre);
                    settle.commit(&raw, &clean); // instant, locked-in text

                    // Kick off the async upgrade for THIS committing block.
                    if let Some(fmt) = cfg.formatter.clone() {
                        let (ftx, frx) = std::sync::mpsc::channel::<String>();
                        let (pre2, lvl) = (pre.clone(), cfg.level);
                        std::thread::spawn(move || {
                            let _ = ftx.send(fmt.format(lvl, &pre2));
                        });
                        pending = Some(Pending { pre, rx: frx, deadline: Instant::now() + SWAP_WINDOW });
                    }
                }
```

Add the `Pending` type and a per-tick poll that applies the upgrade iff it arrives in time and passes the guard. Near the top of `run_loop` (with the other `let mut` state):

```rust
    struct Pending { pre: String, rx: std::sync::mpsc::Receiver<String>, deadline: Instant }
    let mut pending: Option<Pending> = None;
```

And in the per-tick body (after draining transcript events, before painting):

```rust
        // Async LLM swap: apply iff it returned in time AND the guard passes. The
        // committing block is still the one we kicked off for (a NEW commit clears
        // `pending` via the commit arm above, so we never upgrade the wrong block).
        if let Some(p) = pending.as_ref() {
            match p.rx.try_recv() {
                Ok(candidate) => {
                    if Instant::now() <= p.deadline
                        && talk_core::cleanup::guard_accepts(&p.pre, &candidate) {
                        settle.upgrade_committing(&candidate); // no-op if already finalized
                    }
                    pending = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if Instant::now() > p.deadline { pending = None; } // missed the window → keep deterministic
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => { pending = None; }
            }
        }
```

> The `upgrade_committing` no-ops once the block has finalized (its window closed), so a late worker result can never mutate settled text — the settle machine enforces the invariant, the loop just tries. A brand-new `Event::Commit` reassigns `pending` (finalizing the prior committing block first via `settle::commit` → `finalize`), so a stale rewrite for the previous phrase is simply dropped.

- [ ] **Step 3: Construct the formatter in main.rs**

In `src/main.rs`, where the live session is built (Plan 2 T11, when `--from-text` is absent and `listen` is on), construct the formatter and pass it + the level into `LiveConfig`:

```rust
    #[cfg(feature = "formatter")]
    let formatter: Option<std::sync::Arc<dyn talk_core::format::Formatter + Send + Sync>> =
        match verified_formatter_paths() {
            Ok((gguf, tok)) => match format::CandleFormatter::load(gguf.to_str().unwrap(), tok.to_str().unwrap()) {
                Ok(f) => Some(std::sync::Arc::new(f)),
                Err(e) => { eprintln!("formatter disabled: {e}"); None }
            },
            Err(_) => None, // model not downloaded → deterministic-Light only (no hard failure)
        };
    #[cfg(not(feature = "formatter"))]
    let formatter: Option<std::sync::Arc<dyn talk_core::format::Formatter + Send + Sync>> = None;
```

Pass `formatter` and `level: cfg.cleanup_for(mode_str)` into the `LiveConfig` the loop receives. A missing/disabled formatter degrades gracefully to deterministic-Light — never a crash.

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

- **Spec coverage (Plan-3 scope):** formatter policy + prompt in `talk-core` §10 (T2) · the `Formatter` seam + diff-guarded `guarded_format` moat (content-word preservation, fail-safe to deterministic-Light/raw) §10 (T3) · the checked-in **eval set** with faithful-green / over-editing-red / guard-makes-safe, making restraint a test that can fail §10/§14 (T4) · per-mode cleanup level in config with a documented example diff §7/§12 (T5) · guard wired into the session path (the Plan-1 roadmap prerequisite) §10 (T6) · the **Candle 0.5B façade** behind a `formatter` feature, argmax decode, identity-on-error fallback §10/§13 (T7) · model fetch with **pinned SHA-256 verify-before-load** §11 (T8) · async swap via `settle::upgrade_committing` inside the window, settled text never re-flows, instant deterministic-Light layer §7 (T9) · the **latency spike** gating the realtime path §5/§7 (T1). **Correctly deferred to Plan 4:** real spine + flagship packs, ephemeral zeroize/mlock, sidecar raw store, streak/thread views, the sandboxed no-egress + model-tamper + ephemeral-zero-bytes tests, the first-run model-fetch UI. The roadmap's stale `upgrade_block` is corrected to `upgrade_committing` throughout.
- **Placeholder scan:** the only deferrals are the two `sha256: "FILL_AT_PIN_TIME"` values (physically unknowable until the asset is hashed — T8 Step 3 pins them in one command; the verify code that consumes them is complete and a FILL guard refuses to load until pinned) and the **recorded spike numbers** (T1 — a measurement that can only happen on hardware; the example that produces them is complete). The Candle symbols are sourced from the `quantized-qwen2-instruct` example and marked "confirm against the pinned version," exactly as Plan 2 did for sherpa-onnx — version-sensitive bindings, not logic placeholders. **Spike numbers (M1 / x86): _to be recorded by you in T1 Step 4._**
- **Type consistency:** `Level` (now live — `parse_level`/`rewrite_prompt`/`cleanup_for`/`guarded_format`/`score`/`CandleFormatter`/`LiveConfig` all use the same enum) · `RewritePrompt { system, user }` produced by `cleanup::rewrite_prompt`, consumed by `CandleFormatter::generate` · the `Formatter` trait (`format(&self, Level, &str) -> String`) implemented by `DeterministicFormatter`, the eval mocks, the `session.rs` `Flip` test, and `CandleFormatter` · `guarded_format(&dyn Formatter, Level, &str)` called identically from `session::run` and `live::run_loop` · `settle::upgrade_committing(&str) -> bool` (the existing method, no-op after finalize) used for the swap · `RunConfig`/`LiveConfig` gain `formatter` + `level` consistently · `download::{Artifact, fetch, verify}` (Plan 2) reused for `FORMATTER_MODELS`. All cross-task signatures line up.
- **Execution venue:** T2–T6 are pure `talk-core` + config + the session seam — built and unit-tested in CI here (bare `cargo build` and `cargo test` with no features). T1, T7–T9 need the GGUF model, CPU inference, the Plan-2 audio half, and a mic; they are written against the cited Candle example and verified on your machine via the named checks. The `formatter` feature is off by default, so the lean binary and the zero-network session guarantee are preserved (Candle is compute-only; the model fetch lives behind `download`).

---

## Execution Handoff

Two execution options (per superpowers:writing-plans):

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, spec + quality review between tasks. Here that means executing **T2–T6** (CI-testable) now and handing **T1, T7–T9** to your machine with the exact on-machine checks above.

**2. Inline Execution** — execute T2–T6 in this session with checkpoints.

Consistent with the review-gated workflow, I recommend running `/ce-doc-review` on this plan **before** building (it caught ~30 issues including blockers on Plan 2). Which would you like?
