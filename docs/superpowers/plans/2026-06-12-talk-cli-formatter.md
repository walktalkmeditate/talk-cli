# On-Device Formatter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Medium/High cleanup real — run SmolLM2-360M-Instruct once over the whole entry at session end to remove disfluencies (Medium) and add paragraphs (High), behind a deletions-only guard with a Light fallback, never touching the live edge or verbatim raw.

**Architecture:** Pure-core guard + document seam in `talk-core`; a Candle `SmolFormatter` behind a new default-on `format` cargo feature; a single `crate::document_format` helper that runs the LLM on a worker thread (Metal on macOS, CPU on Linux) with a hung-worker deadline and a `polishing…` spinner, gated on a lazily-downloaded model. Wired into both the live path (`run_live_session`) and the `--from-text` path (`session::run`).

**Tech Stack:** Rust, Candle 0.9.2 (`candle-core`/`candle-transformers`/`candle-nn`) + `tokenizers` 0.22, SmolLM2-360M-Instruct Q4_K_M GGUF.

Spec: `docs/superpowers/specs/2026-06-11-talk-cli-formatter-design.md`. Verified spike facts:
- Loader: `candle_transformers::models::quantized_llama::ModelWeights::from_gguf` — correct for SmolLM2 (llama arch, interleaved RoPE; **no** Qwen2 problem).
- Model: `bartowski/SmolLM2-360M-Instruct-GGUF` → `SmolLM2-360M-Instruct-Q4_K_M.gguf` (271 MB). Tokenizer: separate `tokenizer.json` (~3 MB) from `HuggingFaceTB/SmolLM2-360M-Instruct`.
- **EOS = `<|im_end|>` (id 2)**, NOT `<|endoftext|>` — wrong EOS = generates forever.
- `forward(x, index_pos)`: `index_pos=0` for prefill, +1 per decode step. NOTE: `quantized_llama::ModelWeights` has **no** `clear_kv_cache` method (other candle models do; this one doesn't) — none is needed because we load a fresh model per call and `forward` from `index_pos=0`.
- ChatML prompt: `<|im_start|>system\n…<|im_end|>\n<|im_start|>user\n…<|im_end|>\n<|im_start|>assistant\n`.
- MSRV 1.82 safe; GPU deps opt-in. Metal on macOS via the `metal` feature.

---

## File Structure

- `crates/talk-core/src/cleanup.rs` — add `guard_accepts_deletions`, `PINNED_NEGATIONS`, `negation_count`, `strip_model_preamble` (pure).
- `crates/talk-core/src/format.rs` — add `guarded_document`; widen the `Formatter::format` doc.
- `Cargo.toml` — `format` feature (default-on) + candle/tokenizers deps + macOS `metal` target deps.
- `src/format/mod.rs` — NEW (`format` feature). `SmolFormatter` (Candle inference) + the pure `build_chatml` / `cap_new_tokens` helpers.
- `src/download/models.rs` — add `FORMATTER_MODELS` + `formatter_ready()`.
- `src/main.rs` — `document_format` helper (worker thread + deadline + spinner + lazy consent) + `offer_formatter_fetch`; call sites in `run_live_session` and (via `crate::document_format`) `session::run`; `--clean` override.
- `src/session.rs` — call `crate::document_format` on `clean_joined` before `write_entry`.
- `src/config.rs` — `journal_cleanup` default `"medium"` → `"high"`; fix the template comment.
- `src/cli.rs` — `--clean <level>` flag.

---

### Task 1: The Medium/High guard (`guard_accepts_deletions`)

**Files:** Modify `crates/talk-core/src/cleanup.rs`.

- [ ] **Step 1: Write the failing tests** (add to the `mod tests` block):

```rust
#[test]
fn deletions_guard_accepts_filler_removal_and_reflow() {
    assert!(guard_accepts_deletions("um so i mean the thing", "the thing"));
    assert!(guard_accepts_deletions("a b c", "a b\n\nc"));
}

#[test]
fn deletions_guard_rejects_substitution_addition_reorder() {
    assert!(!guard_accepts_deletions("i love her", "i hate her"));
    assert!(!guard_accepts_deletions("i am tired", "i am very tired"));
    assert!(!guard_accepts_deletions("the cat sat", "sat the cat"));
}

#[test]
fn deletions_guard_rejects_dropped_negations_including_contractions() {
    assert!(!guard_accepts_deletions("i am not sure", "i am sure"));
    assert!(!guard_accepts_deletions("i can't go", "i can go"));
    assert!(!guard_accepts_deletions("i won't do it", "i will do it"));
    assert!(!guard_accepts_deletions("there is no way", "there is way"));
}

#[test]
fn deletions_guard_rejects_wholesale_clause_collapse() {
    assert!(!guard_accepts_deletions("the quick brown fox jumps over", "fox"));
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p talk-core deletions_guard` → FAIL (function not found).

- [ ] **Step 3: Implement** (add near `guard_accepts`):

```rust
/// Words/contractions whose deletion inverts meaning — never droppable by the
/// Medium/High guard.
const PINNED_NEGATIONS: &[&str] = &[
    "not", "never", "no", "none", "nor", "cannot", "neither", "nobody",
    "nothing", "nowhere", "without",
];

/// Count negations in `text`. Scans raw whitespace tokens (NOT `content_words`,
/// which splits on the apostrophe and so can never see "n't"): a token in
/// `PINNED_NEGATIONS` or ending in "n't" (can't, won't, isn't, …) counts.
fn negation_count(text: &str) -> usize {
    text.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').to_lowercase())
        .filter(|w| PINNED_NEGATIONS.contains(&w.as_str()) || w.ends_with("n't"))
        .count()
}

fn is_subsequence(needle: &[String], hay: &[String]) -> bool {
    let mut it = hay.iter();
    needle.iter().all(|w| it.any(|h| h == w))
}

/// The Medium/High moat: accept a rewrite that only DELETES content words (filler /
/// false starts) and reflows whitespace. The output's content words must be a
/// subsequence of the input's; no negation may be dropped (incl. contractions); and
/// at least 60% of content words must survive (a wholesale clause collapse fails
/// closed). KNOWN LIMIT: like `guard_accepts`, deleting a non-negation content word
/// such as a concessive "but" is permitted — the 60% budget backstops gross loss.
pub fn guard_accepts_deletions(input: &str, output: &str) -> bool {
    let inp = content_words(input);
    let out = content_words(output);
    if out.len() * 5 < inp.len() * 3 {
        return false; // out/inp < 0.6
    }
    if negation_count(output) < negation_count(input) {
        return false;
    }
    is_subsequence(&out, &inp)
}
```

- [ ] **Step 4: Run to verify pass** — `cargo test -p talk-core deletions_guard` → PASS (4 tests).

- [ ] **Step 4b: Align the High prompt with the guard.** The guard rejects reordering and additions, but the existing `rewrite_prompt` High rule asks the model to "turn spoken lists into bullets" — which reorders items / adds lead-ins → the output isn't a subsequence → guard rejects → silent Light fallback. Narrow High to the deletion + paragraph-reflow the guard can accept. In `rewrite_prompt`, change the `Level::High` arm from `"Also break into paragraphs at topic shifts and turn spoken lists into bullets. Keep every meaning-bearing word."` to `"Also break into paragraphs at topic shifts. Keep every meaning-bearing word, in its original order, adding nothing."`. The existing `rewrite_prompt_widens_by_level_and_carries_the_text` test still passes (the rule still contains "paragraph"). Run `cargo test -p talk-core rewrite_prompt` → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/talk-core/src/cleanup.rs
git commit -m "feat(core): guard_accepts_deletions — subsequence + pinned-negation + budget; align High prompt"
```

---

### Task 2: `strip_model_preamble`

**Files:** Modify `crates/talk-core/src/cleanup.rs`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn strip_model_preamble_removes_preface_fences_quotes() {
    assert_eq!(strip_model_preamble("Sure, here is the cleaned text:\n\nThe real thing."), "The real thing.");
    assert_eq!(strip_model_preamble("```\nThe real thing.\n```"), "The real thing.");
    assert_eq!(strip_model_preamble("\"The real thing.\""), "The real thing.");
    assert_eq!(strip_model_preamble("Already clean."), "Already clean.");
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p talk-core strip_model_preamble` → FAIL.

- [ ] **Step 3: Implement**

```rust
/// Strip a chat model's conversational wrapper from a cleanup reply — a leading
/// "Sure, here…:"/"Here is…:" preface line, surrounding ``` fences, and one pair of
/// wrapping quotes — so a well-formed-but-prefixed reply isn't needlessly rejected
/// by the guard. Best-effort; the guard + Light fallback are the real safety net.
pub fn strip_model_preamble(text: &str) -> String {
    let mut s = text.trim();
    if let Some(rest) = s.strip_prefix("```") {
        s = rest.split_once('\n').map(|(_, r)| r).unwrap_or("").trim();
    }
    if let Some(rest) = s.strip_suffix("```") {
        s = rest.trim();
    }
    if let Some((first, rest)) = s.split_once('\n') {
        let lower = first.trim().to_lowercase();
        let prefaced = ["sure", "here is", "here's", "here are", "certainly", "okay", "ok"]
            .iter().any(|p| lower.starts_with(p));
        if prefaced && lower.ends_with(':') {
            s = rest.trim();
        }
    }
    for (open, close) in [('"', '"'), ('\'', '\''), ('“', '”')] {
        if s.starts_with(open) && s.ends_with(close) && s.chars().count() >= 2 {
            s = s[open.len_utf8()..s.len() - close.len_utf8()].trim();
            break;
        }
    }
    s.to_string()
}
```

- [ ] **Step 4: Run to verify pass** — `cargo test -p talk-core strip_model_preamble` → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/talk-core/src/cleanup.rs
git commit -m "feat(core): strip_model_preamble — drop chat-model wrapper before the guard"
```

---

### Task 3: `guarded_document` (the whole-document seam)

**Files:** Modify `crates/talk-core/src/format.rs`.

- [ ] **Step 1: Write the failing tests** (add a new `mod doc_tests` in format.rs):

```rust
#[cfg(test)]
mod doc_tests {
    use super::*;

    struct Paragrapher;
    impl Formatter for Paragrapher {
        fn format(&self, _l: Level, text: &str) -> String { text.replace(". ", ".\n\n") }
    }
    struct Substitutor;
    impl Formatter for Substitutor {
        fn format(&self, _l: Level, text: &str) -> String { text.replace("love", "hate") }
    }
    struct NegationDropper;
    impl Formatter for NegationDropper {
        fn format(&self, _l: Level, text: &str) -> String { text.replace(" not", "") }
    }
    struct Prefacer;
    impl Formatter for Prefacer {
        fn format(&self, _l: Level, text: &str) -> String { format!("Sure, here:\n{text}") }
    }

    #[test]
    fn paragraph_reflow_accepted_at_high() {
        let out = guarded_document(Level::High, "One thing. Two thing.", &Paragrapher);
        assert!(out.contains("\n\n") && out.contains("One thing") && out.contains("Two thing"));
    }
    #[test]
    fn substitution_falls_back_to_light_join() {
        assert_eq!(guarded_document(Level::Medium, "i love her", &Substitutor), "i love her");
    }
    #[test]
    fn negation_drop_falls_back() {
        assert_eq!(guarded_document(Level::Medium, "i am not sure", &NegationDropper), "i am not sure");
    }
    #[test]
    fn empty_input_short_circuits() {
        assert_eq!(guarded_document(Level::High, "   ", &Substitutor), "   ");
    }
    #[test]
    fn light_and_none_never_invoke_the_formatter() {
        assert_eq!(guarded_document(Level::Light, "i love her", &Substitutor), "i love her");
        assert_eq!(guarded_document(Level::None, "i love her", &Substitutor), "i love her");
    }
    #[test]
    fn preface_is_stripped_then_accepted() {
        assert_eq!(guarded_document(Level::Medium, "the real thing", &Prefacer), "the real thing");
    }
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p talk-core doc_tests` → FAIL.

- [ ] **Step 3: Implement** (add to format.rs; also append one line to the `Formatter::format` doc-comment: `/// May receive a single phrase (guarded_format) or a whole document (guarded_document); implementors must be whole-text-safe.`):

```rust
/// The whole-document moat (Medium/High). The Light join is bound BEFORE the
/// formatter runs and returned on every non-accept path (Light/None level, empty
/// input, or guard rejection) — so the worst case is byte-identical to today's
/// Light output. Only invoked with a model-backed formatter; `DeterministicFormatter`
/// never flows through here (the caller skips the pass when no model is present).
pub fn guarded_document(level: Level, full_text: &str, f: &dyn Formatter) -> String {
    let fallback = full_text.to_string();
    if matches!(level, Level::None | Level::Light) || full_text.trim().is_empty() {
        return fallback;
    }
    let candidate = crate::cleanup::strip_model_preamble(&f.format(level, full_text));
    if crate::cleanup::guard_accepts_deletions(full_text, &candidate) {
        candidate
    } else {
        fallback
    }
}
```

- [ ] **Step 4: Run to verify pass** — `cargo test -p talk-core doc_tests` → PASS (6 tests). Also `cargo test -p talk-core` (whole crate green).

- [ ] **Step 5: Commit**

```bash
git add crates/talk-core/src/format.rs
git commit -m "feat(core): guarded_document — whole-entry pass behind the deletions guard"
```

---

### Task 4: The `format` cargo feature + Candle deps

**Files:** Modify `Cargo.toml`; modify `.github/workflows/release.yml` and `.github/workflows/ci.yml`.

- [ ] **Step 1: Add the feature + deps** to `Cargo.toml`.

Change `default`:
```toml
default = ["listen", "format"]
```
Add to `[features]`:
```toml
format = ["download", "dep:candle-core", "dep:candle-transformers", "dep:candle-nn", "dep:tokenizers"]
```
Add to `[dependencies]`:
```toml
candle-core = { version = "0.9.2", optional = true }
candle-transformers = { version = "0.9.2", optional = true }
candle-nn = { version = "0.9.2", optional = true }
tokenizers = { version = "0.22", optional = true }     # default features incl. a regex backend for SmolLM2 BPE
```
Add a macOS-only Metal block so the formatter uses the GPU there (CPU elsewhere):
```toml
[target.'cfg(target_os = "macos")'.dependencies]
candle-core = { version = "0.9.2", optional = true, features = ["metal"] }
candle-transformers = { version = "0.9.2", optional = true, features = ["metal"] }
```
(The existing `coreaudio-sys` macOS block stays.)

- [ ] **Step 2: Verify it builds both ways**

Run: `cargo build --no-default-features`  → text-only, no candle. Expected: success.
Run: `cargo build`  → default (listen + format), pulls candle (+ metal on macOS). Expected: success (first build is slow — candle compiles). If the macOS `metal` target block causes a Cargo "optional dependency declared twice" error, drop the `features = ["metal"]` block — the formatter falls back to CPU via `Device::new_metal(0).unwrap_or(Device::Cpu)`, so Metal is best-effort and the build must not depend on it.

- [ ] **Step 3: Update release + CI to build the default features**

In `.github/workflows/release.yml`, change the build invocation from `--features listen` to default features (which now include `format`): use `cargo build --release --locked` (no `--features`) OR `--features listen,format`. In `.github/workflows/ci.yml`, keep the fast `test`/`clippy`/no-egress steps on `--no-default-features`; ensure the `listen-build` (macOS) leg builds default features so the `format`/Metal path is exercised.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock .github/workflows/release.yml .github/workflows/ci.yml
git commit -m "build: format feature (Candle + SmolLM2) default-on; Metal on macOS"
```

---

### Task 5: The lazy formatter-model artifacts

**Files:** Modify `crates/talk-core`-independent `src/download/models.rs`.

- [ ] **Step 1: Add the artifacts + a readiness check**

Append to `src/download/models.rs`:

```rust
/// The on-device formatter model (Medium/High cleanup): SmolLM2-360M-Instruct
/// Q4_K_M GGUF + its tokenizer. Fetched LAZILY on first Medium/High use — NOT part
/// of the first-run speech-model offer (`MODELS`). Raw files: `download::fetch`
/// skips extraction for non-`.tar.bz2` names, and `download::verify` hashes the file
/// directly, so no `EXTRACTED` entries are needed.
#[cfg(feature = "format")]
pub const FORMATTER_MODELS: &[Artifact] = &[
    Artifact {
        name: "SmolLM2-360M-Instruct-Q4_K_M.gguf",
        url: "https://huggingface.co/bartowski/SmolLM2-360M-Instruct-GGUF/resolve/main/SmolLM2-360M-Instruct-Q4_K_M.gguf",
        sha256: "FILL-gguf-sha256",
    },
    Artifact {
        name: "SmolLM2-360M-Instruct-tokenizer.json",
        url: "https://huggingface.co/HuggingFaceTB/SmolLM2-360M-Instruct/resolve/main/tokenizer.json",
        sha256: "FILL-tokenizer-sha256",
    },
];

/// True when both formatter files exist on disk and pass their pinned hashes.
#[cfg(feature = "format")]
pub fn formatter_ready(dir: &std::path::Path) -> bool {
    FORMATTER_MODELS.iter().all(|a| {
        let p = dir.join(a.name);
        p.exists() && crate::download::verify(&p, a.sha256).unwrap_or(false)
    })
}
```

- [ ] **Step 2: Pin the SHAs** (manual, one-time — mirrors the existing model-pinning workflow):

```bash
# Pin to a specific HF revision for integrity, not mutable main. Find the commit:
#   open https://huggingface.co/bartowski/SmolLM2-360M-Instruct-GGUF/commits/main
# then download + hash:
curl -fL -o /tmp/smol.gguf "https://huggingface.co/bartowski/SmolLM2-360M-Instruct-GGUF/resolve/main/SmolLM2-360M-Instruct-Q4_K_M.gguf"
curl -fL -o /tmp/smol-tok.json "https://huggingface.co/HuggingFaceTB/SmolLM2-360M-Instruct/resolve/main/tokenizer.json"
shasum -a 256 /tmp/smol.gguf /tmp/smol-tok.json
```
Replace `FILL-gguf-sha256` / `FILL-tokenizer-sha256` with the printed hashes, and (recommended) replace `/resolve/main/` in both URLs with `/resolve/<commit-sha>/` so a re-upload can't change the bytes under the pin. (`download::fetch` already errors if a hash still starts with `FILL`.)

- [ ] **Step 3: Verify it compiles** — `cargo build` → success; `formatter_ready` resolves.

- [ ] **Step 4: Commit**

```bash
git add src/download/models.rs
git commit -m "feat(download): lazy SmolLM2 formatter artifacts (GGUF + tokenizer), pinned"
```

---

### Task 6: `SmolFormatter` (Candle inference)

**Files:** Create `src/format/mod.rs`; add `#[cfg(feature = "format")] mod format;` to `src/main.rs`.

- [ ] **Step 1: Write the pure-helper tests first** (in `src/format/mod.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatml_wraps_system_and_user_turns() {
        let p = build_chatml("SYS", "USR");
        assert!(p.starts_with("<|im_start|>system\nSYS<|im_end|>\n<|im_start|>user\nUSR<|im_end|>\n<|im_start|>assistant\n"));
    }

    #[test]
    fn cap_new_tokens_is_bounded_and_proportional() {
        assert_eq!(cap_new_tokens(100), 132);
        assert!(cap_new_tokens(10_000) < MAX_SEQ_LEN); // clamped under the context window
        assert!(cap_new_tokens(0) >= 16);
    }
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test build_chatml` (or `cargo test --features format chatml`) → FAIL.

- [ ] **Step 3: Implement** `src/format/mod.rs`:

```rust
//! On-device cleanup formatter: SmolLM2-360M-Instruct via Candle. Loaded and run
//! inside a worker thread by `crate::document_format`; implements the pure-core
//! `talk_core::format::Formatter` so it slots into `guarded_document`.

use std::cell::RefCell;
use std::path::Path;

use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use candle_transformers::models::quantized_llama::{ModelWeights, MAX_SEQ_LEN};
use talk_core::cleanup::{rewrite_prompt, Level};
use tokenizers::Tokenizer;

/// Wrap a system + user turn in SmolLM2's ChatML template (pure, testable).
fn build_chatml(system: &str, user: &str) -> String {
    format!(
        "<|im_start|>system\n{system}<|im_end|>\n\
         <|im_start|>user\n{user}<|im_end|>\n\
         <|im_start|>assistant\n"
    )
}

/// Bound the generation: cleanup output is never much longer than the input, so cap
/// at input + a small margin, clamped under the model's context headroom.
fn cap_new_tokens(input_tokens: usize) -> usize {
    (input_tokens + 32).clamp(16, MAX_SEQ_LEN - 8)
}

#[cfg(target_os = "macos")]
fn best_device() -> Device {
    Device::new_metal(0).unwrap_or(Device::Cpu)
}
#[cfg(not(target_os = "macos"))]
fn best_device() -> Device {
    Device::Cpu
}

pub struct SmolFormatter {
    model: RefCell<ModelWeights>,
    tokenizer: Tokenizer,
    device: Device,
    eos: u32,
    /// Set by the caller on a deadline timeout so the decode loop bails within one
    /// token instead of orphaning a full-budget inference on the GPU/CPU.
    abandoned: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl SmolFormatter {
    /// Load the GGUF weights + tokenizer. Returns an error (so the caller falls back
    /// to Light) on any I/O or format problem.
    pub fn load(
        gguf_path: &Path,
        tokenizer_path: &Path,
        abandoned: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<SmolFormatter, String> {
        let device = best_device();
        let mut file = std::fs::File::open(gguf_path).map_err(|e| e.to_string())?;
        let ct = gguf_file::Content::read(&mut file).map_err(|e| e.to_string())?;
        let model = ModelWeights::from_gguf(ct, &mut file, &device).map_err(|e| e.to_string())?;
        let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(|e| e.to_string())?;
        let eos = tokenizer.get_vocab(true).get("<|im_end|>").copied().unwrap_or(2);
        Ok(SmolFormatter { model: RefCell::new(model), tokenizer, device, eos, abandoned })
    }

    fn generate(&self, level: Level, text: &str) -> Result<String, String> {
        use std::sync::atomic::Ordering;
        let p = rewrite_prompt(level, text);
        let prompt = build_chatml(&p.system, &p.user);
        let encoded = self.tokenizer.encode(prompt.as_str(), false).map_err(|e| e.to_string())?;
        let prompt_tokens: Vec<u32> = encoded.get_ids().to_vec();
        if prompt_tokens.is_empty() || prompt_tokens.len() >= MAX_SEQ_LEN {
            return Err("prompt empty or too long".into());
        }
        // Bound on the RAW TEXT length (cleanup output ≈ input), not the prompt
        // wrapper — the ~60-word system restraint shouldn't inflate the budget. EOS
        // (<|im_end|>) is the primary stop; this cap is the backstop.
        let text_len = self.tokenizer.encode(text, false)
            .map(|e| e.get_ids().len()).unwrap_or(prompt_tokens.len());
        let budget = cap_new_tokens(text_len);

        // Fresh model per call, forward from index_pos=0 → KV cache is empty; there
        // is no clear_kv_cache on quantized_llama::ModelWeights and none is needed.
        let mut model = self.model.borrow_mut();
        let input = Tensor::new(prompt_tokens.as_slice(), &self.device)
            .and_then(|t| t.unsqueeze(0)).map_err(|e| e.to_string())?;
        let logits = model.forward(&input, 0).map_err(|e| e.to_string())?;
        let mut next = argmax(&logits)?;
        let mut out: Vec<u32> = Vec::new();
        let mut index_pos = prompt_tokens.len();

        for _ in 0..budget {
            if next == self.eos || self.abandoned.load(Ordering::Relaxed) { break; }
            out.push(next);
            let input = Tensor::new(&[next], &self.device)
                .and_then(|t| t.unsqueeze(0)).map_err(|e| e.to_string())?;
            let logits = model.forward(&input, index_pos).map_err(|e| e.to_string())?;
            next = argmax(&logits)?;
            index_pos += 1;
        }
        self.tokenizer.decode(&out, true).map_err(|e| e.to_string())
    }
}

fn argmax(logits: &Tensor) -> Result<u32, String> {
    logits.squeeze(0)
        .and_then(|l| l.argmax(candle_core::D::Minus1))
        .and_then(|t| t.to_scalar::<u32>())
        .map_err(|e| e.to_string())
}

impl talk_core::format::Formatter for SmolFormatter {
    /// On inference failure, return the input unchanged — the document guard then
    /// trivially accepts it (identity is a valid subsequence), so the entry keeps its
    /// Light text. Errors never propagate to the write path.
    fn format(&self, level: Level, text: &str) -> String {
        self.generate(level, text).unwrap_or_else(|_| text.to_string())
    }
}
```

Add `#[cfg(feature = "format")] mod format;` to `src/main.rs` near the other `mod` declarations.

- [ ] **Step 4: Run pure tests + the build** — `cargo test --features format cap_new_tokens chatml` → PASS; `cargo build` → success.

- [ ] **Step 5: Real-model smoke test** (`#[ignore]` — needs the downloaded model). Add to the `tests` mod:

```rust
    /// Loads the real model (run `talk download models` first) and asserts a Medium
    /// rewrite passes the document guard. Ignored by default — it needs the ~271 MB
    /// GGUF. Run with: `cargo test --features format smol_smoke -- --ignored`.
    #[test]
    #[ignore]
    fn smol_smoke_passes_the_guard() {
        let dir = crate::paths::models_dir();
        let gguf = dir.join("SmolLM2-360M-Instruct-Q4_K_M.gguf");
        let tok = dir.join("SmolLM2-360M-Instruct-tokenizer.json");
        if !gguf.exists() { eprintln!("skip: model not downloaded"); return; }
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let f = SmolFormatter::load(&gguf, &tok, flag).unwrap();
        let raw = "um so i guess i was you know thinking about the the thing and anyway it went well";
        let out = <SmolFormatter as talk_core::format::Formatter>::format(&f, Level::Medium, raw);
        assert!(talk_core::cleanup::guard_accepts_deletions(raw, &out),
            "model output must pass the deletions guard: {out:?}");
    }
```

Run it once manually after Task 8's download works (or after `talk download models`): `cargo test --features format smol_smoke -- --ignored`. Confirm it passes (or, if the model over-edits and the guard rejects, that's the fallback working — but a healthy model should pass on filler-only input).

- [ ] **Step 6: Commit**

```bash
git add src/format/mod.rs src/main.rs
git commit -m "feat(format): SmolFormatter — Candle SmolLM2 inference behind the Formatter trait"
```

---

### Task 7: `document_format` — worker thread, deadline, spinner, lazy consent

**Files:** Modify `src/main.rs` (add `document_format`, `offer_formatter_fetch`, a small spinner); modify `src/session.rs` (call site); modify `src/main.rs::run_live_session` (call site).

- [ ] **Step 1: Add the helper + its non-format stub** to `src/main.rs`:

```rust
/// Run the whole-document LLM pass on the final clean text, when the level warrants
/// it and the model is present. Runs on a worker thread with a hung-worker deadline;
/// any failure (declined download, load/inference error, timeout) returns the Light
/// join unchanged. Called by run_live_session (live) and session::run (--from-text).
#[cfg(feature = "format")]
fn document_format(level: talk_core::cleanup::Level, light_join: &str) -> String {
    use talk_core::cleanup::Level;
    if matches!(level, Level::None | Level::Light) || light_join.trim().is_empty() {
        return light_join.to_string();
    }
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let dir = paths::models_dir();
    if !download::models::formatter_ready(&dir) && !offer_formatter_fetch(&dir) {
        // Non-interactive runs never block on the 271 MB download, so the formatter
        // is silently skipped — say so once so a piped/cron `talk journal` (High by
        // config) doesn't look like it's reshaping when it isn't.
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            eprintln!("note: formatter model not present — saved at Light. Run `talk download models` to enable paragraphs.");
        }
        return light_join.to_string();
    }
    let gguf = dir.join("SmolLM2-360M-Instruct-Q4_K_M.gguf");
    let tok = dir.join("SmolLM2-360M-Instruct-tokenizer.json");
    let text = light_join.to_string();
    let abandoned = Arc::new(AtomicBool::new(false));
    let worker_flag = abandoned.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let out = match format::SmolFormatter::load(&gguf, &tok, worker_flag) {
            Ok(f) => talk_core::format::guarded_document(level, &text, &f),
            Err(_) => text,
        };
        let _ = tx.send(out);
    });
    let _spin = Spinner::start("  polishing…");
    match rx.recv_timeout(std::time::Duration::from_secs(120)) {
        Ok(out) => out,
        // Hung-worker backstop: signal the orphan to bail within one token, take Light.
        Err(_) => { abandoned.store(true, Ordering::Relaxed); light_join.to_string() }
    }
}

#[cfg(not(feature = "format"))]
fn document_format(_level: talk_core::cleanup::Level, light_join: &str) -> String {
    light_join.to_string()
}
```

- [ ] **Step 2: Add the consent fetch** (mirror the existing `offer_first_run_fetch` / `accept_fetch` pattern already in main.rs):

```rust
/// Offer the one-time formatter-model download (≈ 271 MB). TTY-gated, default-yes;
/// `n`/EOF declines. Returns true only after a successful verified fetch.
#[cfg(feature = "format")]
fn offer_formatter_fetch(dir: &std::path::Path) -> bool {
    use std::io::Write;
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return false; // never block a non-interactive run on a download
    }
    print!("talk's paragraph formatter needs a one-time model — about 271 MB, downloaded \
            once and kept on your machine. Get it now? [Y/n] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let n = std::io::stdin().read_line(&mut line).unwrap_or(0);
    if n == 0 || matches!(line.trim(), "n" | "N") {
        return false;
    }
    for art in download::models::FORMATTER_MODELS {
        eprint!("  ↓ {}…", art.name);
        if let Err(e) = download::fetch(art, dir, &mut |_, _| {}) {
            eprintln!(" failed: {e}");
            return false;
        }
        eprintln!(" ✓");
    }
    download::models::formatter_ready(dir)
}
```

- [ ] **Step 3: Add a minimal spinner** to `src/main.rs` (TTY-gated, stops on drop):

```rust
#[cfg(feature = "format")]
struct Spinner(std::sync::Arc<std::sync::atomic::AtomicBool>, Option<std::thread::JoinHandle<()>>);

#[cfg(feature = "format")]
impl Spinner {
    fn start(label: &'static str) -> Spinner {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let stop = Arc::new(AtomicBool::new(false));
        if !std::io::IsTerminal::is_terminal(&std::io::stderr()) {
            return Spinner(stop, None);
        }
        let s = stop.clone();
        let h = std::thread::spawn(move || {
            let frames = ['⠋','⠙','⠹','⠸','⠼','⠴','⠦','⠧','⠇','⠏'];
            let mut i = 0;
            while !s.load(Ordering::Relaxed) {
                eprint!("\r{} {}", frames[i % frames.len()], label);
                let _ = std::io::Write::flush(&mut std::io::stderr());
                std::thread::sleep(std::time::Duration::from_millis(90));
                i += 1;
            }
            eprint!("\r\x1b[2K"); // clear the line
            let _ = std::io::Write::flush(&mut std::io::stderr());
        });
        Spinner(stop, Some(h))
    }
}

#[cfg(feature = "format")]
impl Drop for Spinner {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.1.take() { let _ = h.join(); }
    }
}
```

- [ ] **Step 4: Call it on the live path.** In `run_live_session`, after `run_loop` returns and after the `result.clean.trim().is_empty()` guard, BEFORE the write-recovery loop, insert:

```rust
    let level = if matches!(shape, Shape::Journal) {
        cfg.cleanup_for("journal")
    } else {
        cfg.cleanup_for("reflect")
    };
    result.clean = document_format(level, &result.clean);
```
(`result` is already `mut`. This runs after the alternate `Screen` has dropped, so the spinner is safe.)

- [ ] **Step 5: Call it on the `--from-text` path.** In `src/session.rs::run`, after `clean_joined` is built and BEFORE `write_entry`, insert:

```rust
    let clean_joined = crate::document_format(cfg.level, &clean_joined);
```
(`crate::document_format` resolves to the main.rs helper; session.rs is part of the binary crate. Reflect runs at Light → the helper short-circuits, so no model load.)

- [ ] **Step 6: Build + test** — `cargo build` and `cargo test` → all pass (the document_format stub is used in `--no-default-features`; under default features the real path compiles). The existing session/live tests run at `Level::Light`, so `document_format` short-circuits and their assertions are unchanged.

- [ ] **Step 7: Manual smoke** (after the model is downloadable):

```bash
cargo build
env HOME=/tmp/fmt-smoke ./target/debug/talk journal --from-text "um so i guess i woke up early today and then you know i went for a walk and anyway it was a good morning new paragraph later i sat down to write"
# Expect: a 'polishing…' spinner, then a written entry that reads cleanly (filler gone,
# a paragraph break) — or, if you decline the 271 MB download, the Light text.
```

- [ ] **Step 8: Commit**

```bash
git add src/main.rs src/session.rs
git commit -m "feat: document_format — worker-thread LLM pass with deadline, spinner, lazy consent"
```

---

### Task 8: journal=High default + `--clean` flag + template comment

**Files:** Modify `src/config.rs`, `src/cli.rs`, `src/main.rs`.

- [ ] **Step 1: Write the failing config test** — update the existing `zero_config_uses_defaults` test in `src/config.rs` to expect `High` for journal:

```rust
    assert_eq!(c.cleanup_for("journal"), Level::High);
```

- [ ] **Step 2: Flip the default + fix the comment.** In `src/config.rs` `Default`, change `journal_cleanup: "medium".into()` → `journal_cleanup: "high".into()`. In `commented_template()`, replace the stale `journal_cleanup` comment with:
```
# medium: LLM removes filler/joins fragments · high: + paragraphs; falls back to light if the model is absent
```
Run: `cargo test -p talk-cli zero_config` (and the other config tests) → PASS.

- [ ] **Step 3: Add the `--clean` flag.** In `src/cli.rs`, add to `Cli`:
```rust
    /// Override cleanup intensity for this run: none · light · medium · high.
    #[arg(long, global = true)]
    pub clean: Option<CleanArg>,
```
and:
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum CleanArg { None, Light, Medium, High }

impl From<CleanArg> for talk_core::cleanup::Level {
    fn from(a: CleanArg) -> Self {
        use talk_core::cleanup::Level;
        match a {
            CleanArg::None => Level::None,
            CleanArg::Light => Level::Light,
            CleanArg::Medium => Level::Medium,
            CleanArg::High => Level::High,
        }
    }
}
```

- [ ] **Step 4: Apply the override.** `args.clean` (an `Option<CleanArg>`) is in scope in `main()` and `run_live_session` (both receive `args`). Let it win wherever a `Level` is chosen:

  **(a) Live path** — replace the `let level = …` you added in Task 7 Step 4 of `run_live_session` with:
  ```rust
      let level = args.clean.map(Into::into).unwrap_or_else(|| {
          if matches!(shape, Shape::Journal) { cfg.cleanup_for("journal") } else { cfg.cleanup_for("reflect") }
      });
  ```

  **(b) `--from-text` journal arms** — in `main()`, the two journal `run_and_report` call sites (the `Some(Command::Journal)` arm and the bare-`talk` journal branch in the `_ =>` arm) set `level: cfg.cleanup_for("journal")`. Change both to:
  ```rust
      level: args.clean.map(Into::into).unwrap_or_else(|| cfg.cleanup_for("journal")),
  ```

  (The `unburden`/`vent` arm stays `Level::None` — ephemeral is never formatted. The `reflect` path runs at Light and routes through `reflect()`, which doesn't take `args`; `--clean` on reflect is out of scope for v1 — note it as a follow-up rather than threading `clean` through `reflect`/`reflect_choice`.)

- [ ] **Step 5: Build + test + smoke**

Run: `cargo test` → all pass.
Run: `cargo build && env HOME=/tmp/fmt-smoke ./target/debug/talk journal --from-text "um the thing" --clean light` → writes the Light text with no formatter invocation (the override forces Light).

- [ ] **Step 6: Commit**

```bash
git add src/config.rs src/cli.rs src/main.rs
git commit -m "feat: journal defaults to High; --clean override; fix config template comment"
```

---

## Notes for the implementer

- **The guard is the safety net.** Every failure mode (declined download, offline, load error, inference error, over-edit, timeout, model preamble) ends at the Light join — the written entry is never worse than today, and the verbatim raw is never touched.
- **First build is slow.** Candle compiles a lot the first time (3–6 min). Subsequent builds are incremental.
- **Latency is real.** A long journal at High is ~7–12 s on macOS (Metal), ~20–40 s on Linux CPU. The spinner covers it; the 120 s deadline is a hung-worker backstop, not a latency budget (the token cap bounds normal runs).
- **Do NOT load the model on the reflect/Light path.** `document_format` short-circuits on `Level::Light`/`None` before any model touch — keep it that way so reflect stays instant and reflect-only users never trigger the 271 MB download.
- **SHA pinning is mandatory + blocking.** `download::fetch` refuses any artifact whose hash still starts with `FILL`, so the feature is non-functional (and the smoke + manual tests can't pass) until a human runs Task 5 Step 2. Pin to a specific HF commit revision, not `main`.
- **Task 7 is add-everything-then-build.** Steps 1–5 only add code (`document_format`, `offer_formatter_fetch`, `Spinner`, and the two call sites); the first `cargo build`/`cargo test` is Step 6. Rust resolves module items regardless of definition order, so the helpers and their caller can be added in any order within `main.rs` — there is no intermediate compile.
- **The 60% deletion budget is provisional.** `content_words` already excludes the FILLERS set, so um/uh/you-know removal doesn't count against it — but heavy false-start/repetition removal might. Before trusting the default, run the `#[ignore]` `smol_smoke` test (Task 6 Step 5) over a few real disfluent-journal samples and check the guard acceptance rate; if healthy filler-heavy input rejects to Light too often, lower the floor (e.g. 50%) and record the measured rate next to the constant.
- **The timed-out worker is signalled, not joined.** On the 120 s backstop, `document_format` sets the `abandoned` flag so the orphan's decode loop bails within one token, then the model drops and frees as the thread exits. The main thread never blocks on the worker.
