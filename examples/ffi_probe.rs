//! No-egress FFI probe: drives the REAL sherpa-onnx inference stack (streaming
//! Zipformer-20M first pass + Moonshine base second pass) from cached model files,
//! with NO network surface of its own. The
//! `inference_stack_runs_under_deny_network_sandbox` privacy test shells out to this
//! under a deny-network sandbox to prove the FFI path makes zero outbound calls.
//!
//! Unlike Plan 2's throwaway loopback probes, this one stays committed: it is the
//! load-bearing helper the privacy test invokes.
//!
//! It re-includes the production `streaming.rs`/`stt.rs` verbatim (`#[path]`) so the
//! probe exercises exactly the code the live session runs, not a parallel copy.

// The probe re-includes production modules wholesale but only drives the
// push/partial/transcribe path; their other public methods (e.g. `reset`,
// `endpoint`) are legitimately unused here.
#[path = "../src/listen/streaming.rs"]
#[allow(dead_code)]
mod streaming;
#[path = "../src/listen/stt.rs"]
#[allow(dead_code)]
mod stt;

use std::path::Path;

fn main() {
    let models = std::env::args()
        .nth(1)
        .expect("usage: ffi_probe <models_dir>");
    let models = Path::new(&models);
    let moonshine = models.join("sherpa-onnx-moonshine-base-en-quantized-2026-02-27");
    let zipformer = models.join("sherpa-onnx-streaming-zipformer-en-20M-2023-02-17");

    let mut live = streaming::Streaming::new(
        zipformer
            .join("encoder-epoch-99-avg-1.int8.onnx")
            .to_str()
            .expect("encoder path is UTF-8"),
        zipformer
            .join("decoder-epoch-99-avg-1.onnx")
            .to_str()
            .expect("decoder path is UTF-8"),
        zipformer
            .join("joiner-epoch-99-avg-1.int8.onnx")
            .to_str()
            .expect("joiner path is UTF-8"),
        zipformer.join("tokens.txt").to_str().expect("tokens path is UTF-8"),
    )
    .expect("streaming Zipformer loads from the cached model dir");
    let recognizer = stt::Stt::new(
        moonshine
            .join("encoder_model.ort")
            .to_str()
            .expect("encoder path is UTF-8"),
        moonshine
            .join("decoder_model_merged.ort")
            .to_str()
            .expect("decoder path is UTF-8"),
        moonshine.join("tokens.txt").to_str().expect("tokens path is UTF-8"),
    )
    .expect("Moonshine base loads from the cached model dir");

    // ~2s of synthesized 16 kHz mono audio: 1s of silence, then a 440 Hz tone so
    // BOTH passes get a quiet stretch and a speech-like onset to chew on.
    let mut audio = vec![0.0f32; 16_000];
    for n in 0..16_000 {
        let t = n as f32 / 16_000.0;
        audio.push((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.3);
    }

    for chunk in audio.chunks(1_600) {
        live.push(chunk);
        let _ = live.partial();
        let _ = live.endpoint();
    }
    let _ = stt::transcribe_chunked(&recognizer, &audio, 16_000);

    println!("ok");
}
