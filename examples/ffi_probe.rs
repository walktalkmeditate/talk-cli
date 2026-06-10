//! No-egress FFI probe: drives the REAL sherpa-onnx inference stack (Silero VAD +
//! Moonshine STT) from cached model files, with NO network surface of its own. The
//! `inference_stack_runs_under_deny_network_sandbox` privacy test shells out to this
//! under a deny-network sandbox to prove the FFI path makes zero outbound calls.
//!
//! Unlike Plan 2's throwaway loopback probes, this one stays committed: it is the
//! load-bearing helper the privacy test invokes.
//!
//! It re-includes the production `vad.rs`/`stt.rs` verbatim (`#[path]`) so the probe
//! exercises exactly the code the live session runs, not a parallel copy.

// The probe re-includes production modules wholesale but only drives the
// segment/transcribe path; their other public methods (e.g. `is_speaking`) are
// legitimately unused here.
#[path = "../src/listen/vad.rs"]
#[allow(dead_code)]
mod vad;
#[path = "../src/listen/stt.rs"]
#[allow(dead_code)]
mod stt;

use std::path::Path;

fn main() {
    let models = std::env::args()
        .nth(1)
        .expect("usage: ffi_probe <models_dir>");
    let models = Path::new(&models);
    let moonshine = models.join("sherpa-onnx-moonshine-tiny-en-quantized-2026-02-27");
    let silero = models.join("silero_vad.onnx");

    let mut seg = vad::Segmenter::new(silero.to_str().expect("silero path is UTF-8"))
        .expect("VAD loads from the cached silero model");
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
    .expect("Moonshine loads from the cached model dir");

    // ~2s of synthesized 16 kHz mono audio: 1s of silence, then a 440 Hz tone so the
    // VAD has both a quiet stretch and a speech-like onset to segment.
    let mut audio = vec![0.0f32; 16_000];
    for n in 0..16_000 {
        let t = n as f32 / 16_000.0;
        audio.push((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.3);
    }

    for segment in seg.push(&audio) {
        let _ = recognizer.transcribe(&segment.samples, segment.sample_rate);
    }
    for segment in seg.flush() {
        let _ = recognizer.transcribe(&segment.samples, segment.sample_rate);
    }

    println!("ok");
}
