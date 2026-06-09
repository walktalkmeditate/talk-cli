/// One downloadable artifact with a pinned hash. Fill `sha256` from the actual
/// release asset (run `shasum -a 256 <file>` after a manual download) at pin time.
pub struct Artifact {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
}

/// Plan-2 models: Moonshine tiny (en, quantized) + Silero VAD. URLs are k2-fsa
/// release assets; HASHES MUST be filled in at pin time (they are release-stable).
pub const MODELS: &[Artifact] = &[
    // Quantized merged-decoder Moonshine tiny. Extracts to the subdir
    // `sherpa-onnx-moonshine-tiny-en-quantized-2026-02-27/`; the loader (T8) uses
    // `encoder_model.ort`, `decoder_model_merged.ort`, `tokens.txt` from it.
    Artifact {
        name: "sherpa-onnx-moonshine-tiny-en-quantized-2026-02-27.tar.bz2",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-moonshine-tiny-en-quantized-2026-02-27.tar.bz2",
        sha256: "FILL_AT_PIN_TIME",
    },
    Artifact {
        name: "silero_vad.onnx",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx",
        sha256: "FILL_AT_PIN_TIME",
    },
];
