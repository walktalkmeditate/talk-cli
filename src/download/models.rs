/// One downloadable artifact with a pinned hash. Fill `sha256` from the actual
/// release asset (run `shasum -a 256 <file>` after a manual download) at pin time.
pub struct Artifact {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
}

/// Plan-5 models: Moonshine **base** (en, quantized) for the pass-2 transcription
/// and the streaming **Zipformer-20M** transducer for the live first pass. URLs are
/// k2-fsa release assets; the SHAs are release-stable and corroborated below.
///
/// Replaces the Plan-2 manifest (moonshine-tiny + silero_vad): the streaming
/// model's built-in endpoint rules do the VAD's old job, so silero is gone, and
/// base supersedes tiny for accuracy (~239 MB combined, stated honestly in the
/// fetch offer). Old tiny/silero caches are left in place on disk (~30 MB,
/// harmless — never loaded again); a `talk download clean` is deliberately NOT
/// added (YAGNI: there is no downgrade path once the code stops referencing them).
///
/// Pin corroboration (2026-06-09, required by spec §11 for weights that run on
/// private audio): both archive SHAs were independently re-derived by downloading
/// from the URLs below and hashing. The moonshine-base SHA additionally matches
/// GitHub's stored release-asset digest (`sha256:43232c1d…`); the zipformer asset
/// predates GitHub's digest field (uploaded 2024-01-03, `digest: null`), so its
/// archive pin rests on the download-and-rehash above — but all three of its
/// pinned WEIGHT files in `EXTRACTED` (encoder/decoder/joiner) were corroborated
/// 2026-06-09 against an independent channel: the Hugging Face mirror
/// `csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17` serves
/// byte-identical files (LFS sha256 `3810755c…`/`45a7f940…`/`e085d73b…` match).
/// NOTE: a similarly-named `…-20M-2023-02-17-mobile.tar.bz2`
/// (107.6 MB) is a DIFFERENT asset — this manifest pins the non-mobile 127.9 MB
/// archive, which is what `9c559283…` hashes to.
pub const MODELS: &[Artifact] = &[
    // Moonshine base, quantized merged-decoder. Extracts to
    // `sherpa-onnx-moonshine-base-en-quantized-2026-02-27/`; the loader reads
    // `encoder_model.ort`, `decoder_model_merged.ort`, `tokens.txt` (drop-in for
    // the `Stt` that previously loaded tiny — identical file layout).
    Artifact {
        name: "sherpa-onnx-moonshine-base-en-quantized-2026-02-27.tar.bz2",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-moonshine-base-en-quantized-2026-02-27.tar.bz2",
        // Corroborated against the GitHub release-asset digest, 2026-06-09.
        sha256: "43232c1d13013d37317163baec3135bd771a186a4356f28c889bab453bb0e891",
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
        "sherpa-onnx-moonshine-base-en-quantized-2026-02-27/encoder_model.ort",
        "7c66495948d0d08ec1af454cd4b5514862ae6511e94712a60e6d83eaec8dc8cf",
    ),
    (
        "sherpa-onnx-moonshine-base-en-quantized-2026-02-27/decoder_model_merged.ort",
        "d9d7b333af34bc552580576ddcf248a1c6c839e0d3b43b09afb9376ed009899d",
    ),
    (
        "sherpa-onnx-moonshine-base-en-quantized-2026-02-27/tokens.txt",
        "2870d843e14c1e187bf1913a521562a63b53933814bd7f2145120468f494a049",
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
