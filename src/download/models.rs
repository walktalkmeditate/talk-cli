/// One downloadable artifact with a pinned hash. Fill `sha256` from the actual
/// release asset (run `shasum -a 256 <file>` after a manual download) at pin time.
pub struct Artifact {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
}

/// Models for live transcription: **Whisper base.en** (int8 encoder/decoder) for
/// pass-2 transcription and the streaming **Zipformer-20M** transducer for the
/// live first pass. URLs are k2-fsa release assets; the SHAs are release-stable
/// and corroborated below. Combined download ≈ 327 MB (zipformer 128 MB + whisper
/// base 199 MB), stated honestly in the fetch offer.
///
/// Pin corroboration (required by spec §11 for weights that run on private audio):
/// The zipformer archive predates GitHub's digest field (uploaded 2024-01-03,
/// `digest: null`), so its archive pin rests on download-and-rehash (2026-06-09);
/// all three of its pinned weight files in `EXTRACTED` (encoder/decoder/joiner)
/// were corroborated against the Hugging Face mirror
/// `csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17` (LFS sha256
/// `3810755c…`/`45a7f940…`/`e085d73b…` match). NOTE: a similarly-named
/// `…-20M-2023-02-17-mobile.tar.bz2` (107.6 MB) is a DIFFERENT asset — this
/// manifest pins the non-mobile 127.9 MB archive (`9c559283…`).
///
/// The Whisper base.en archive also predates GitHub's stored-digest field
/// (`digest: null`), so its archive pin (`475bc705…`) rests on
/// download-and-rehash (2026-06-10); all three extracted files it loads — the
/// int8 encoder, int8 decoder, and tokens — were corroborated against the
/// Hugging Face mirror `csukuangfj/sherpa-onnx-whisper-base.en` (sha256
/// `ef6b936f…` / `f7162ad6…` / `306cd27f…` match).
pub const MODELS: &[Artifact] = &[
    // Whisper base.en (int8 encoder/decoder + tokens) for pass-2 transcription.
    // Extracts to `sherpa-onnx-whisper-base.en/`; the loader reads
    // `base.en-encoder.int8.onnx`, `base.en-decoder.int8.onnx`, `base.en-tokens.txt`.
    Artifact {
        name: "sherpa-onnx-whisper-base.en.tar.bz2",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-whisper-base.en.tar.bz2",
        // GitHub asset digest is null (predates the field); corroborated by
        // download-and-rehash, with the int8 weights matching the HF mirror below.
        sha256: "475bc7052ce299c007f6d5d5407ba8601f819a2867f6eecee510ed17df581542",
    },
    // Streaming Zipformer-20M transducer (encoder/decoder/joiner + tokens).
    Artifact {
        name: "sherpa-onnx-streaming-zipformer-en-20M-2023-02-17.tar.bz2",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17.tar.bz2",
        // Corroborated against the GitHub release asset by download-and-rehash,
        // 2026-06-09 (the asset predates GitHub's stored-digest field).
        sha256: "9c559283e8498d3fe95913c79ca1cb454bb26281ac2b102b41306c7d752765d9",
    },
];

/// Extracted files the session actually loads — verified at load time (the archive
/// hash alone doesn't cover post-extraction tampering). Sessions never read the
/// archive, so the verify gate must hash these, not just the tarball. Replaces the
/// Plan-2 `MOONSHINE_EXTRACTED` constant.
///
/// The streaming transducer uses the **int8** encoder/joiner with the **fp32**
/// decoder (the standard sherpa-onnx recipe). Pins computed from the verified
/// archives' extractions and corroborated by re-extracting a fresh download,
/// 2026-06-09.
pub const EXTRACTED: &[(&str, &str)] = &[
    (
        "sherpa-onnx-whisper-base.en/base.en-encoder.int8.onnx",
        "ef6b936f4c9b1d90a3b68634b60c4ed8576b26172b33c2535ec0e933c9edb823",
    ),
    (
        "sherpa-onnx-whisper-base.en/base.en-decoder.int8.onnx",
        "f7162ad6db2dbef16cfaeaa7f945b9d7dd9c1b8d472f6aca82f2273d185e4d41",
    ),
    (
        "sherpa-onnx-whisper-base.en/base.en-tokens.txt",
        "306cd27f03c1a714eca7108e03d66b7dc042abe8c258b44c199a7ed9838dd930",
    ),
    (
        "sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/encoder-epoch-99-avg-1.int8.onnx",
        "3810755ce7c3ab26b42a8bcf39d191308fa27fb0f53358823ba46141d03b7eb3",
    ),
    (
        "sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/decoder-epoch-99-avg-1.onnx",
        "45a7f940ecfb53d89fa270ad11b88b961e53a317203eb24b1c8e95ed208b0f30",
    ),
    (
        "sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/joiner-epoch-99-avg-1.int8.onnx",
        "e085d73b593cf9b0707f370dbd656d58327d3fe36d80d849202ef81df02cb01e",
    ),
    (
        "sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/tokens.txt",
        "49e3c2646595fd907228b3c6787069658f67b17377c60aeb8619c4551b2316fb",
    ),
];

