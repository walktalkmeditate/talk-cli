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
        let eos = tokenizer.get_vocab(true).get("<|im_end|>").copied()
            .ok_or("tokenizer has no <|im_end|> token")?;
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
        if prompt_tokens.len() + budget > MAX_SEQ_LEN {
            return Err("prompt + budget exceeds the context window".into());
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatml_wraps_system_and_user_turns() {
        let p = build_chatml("SYS", "USR");
        assert_eq!(p, "<|im_start|>system\nSYS<|im_end|>\n<|im_start|>user\nUSR<|im_end|>\n<|im_start|>assistant\n");
    }

    #[test]
    fn cap_new_tokens_is_bounded_and_proportional() {
        assert_eq!(cap_new_tokens(100), 132);
        assert!(cap_new_tokens(10_000) < MAX_SEQ_LEN); // clamped under the context window
        assert!(cap_new_tokens(0) >= 16);
    }

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
        eprintln!("model output: {out:?}");
        assert!(talk_core::cleanup::guard_accepts_deletions(raw, &out),
            "model output must pass the deletions guard: {out:?}");
    }
}
