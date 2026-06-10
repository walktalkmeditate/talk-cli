pub mod capture;
pub mod streaming;
pub mod stt;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use crate::source::{Event, TranscriptSource};
use capture::Capture;
use streaming::Streaming;
use stt::Stt;

/// Live mic → streaming Zipformer (pass 1) → Moonshine base (pass 2), running both
/// passes on a WORKER THREAD so the UI loop never blocks. `next()` is non-blocking:
/// it drains a results channel. The worker is serial — it emits `Commit(streaming)`,
/// runs pass-2, emits `Revise(better)`, then resumes feeding — so a Revise always
/// targets the block it just committed (no races, no extra threads).
pub struct LiveSource {
    _stream: cpal::Stream,            // kept alive HERE on the owning thread (!Send)
    results: Receiver<Event>,
    speaking: Arc<AtomicBool>,
    finish_flag: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl LiveSource {
    pub fn new(capture: Capture, mut streaming: Streaming, stt: Stt) -> Self {
        // Only the Receiver (Send) + streaming + stt (both Send per sherpa-onnx) move
        // to the worker; the cpal Stream (!Send) stays on this thread.
        let (stream, samples, cap_rate) = capture.into_parts();
        let (tx, rx): (Sender<Event>, Receiver<Event>) = std::sync::mpsc::channel();
        let speaking = Arc::new(AtomicBool::new(false));
        let finish_flag = Arc::new(AtomicBool::new(false));
        let (sp, ff) = (speaking.clone(), finish_flag.clone());
        let worker = thread::spawn(move || {
            // Worker-local state: the last partial we emitted (so we only push on
            // change), when it last changed (drives the partial-activity latch +
            // its silence decay), and the audio of the current segment (fed to
            // pass-2 on endpoint).
            let mut last_partial = String::new();
            let mut last_change = Instant::now();
            let mut seg_buf: Vec<f32> = Vec::new();
            loop {
                match samples.recv_timeout(Duration::from_millis(50)) {
                    Ok(chunk) => {
                        let resampled = resample_to_16k(&chunk, cap_rate);
                        streaming.push(&resampled);
                        seg_buf.extend_from_slice(&resampled);

                        let partial = streaming.partial();
                        if partial != last_partial {
                            sp.store(true, Ordering::Relaxed);
                            last_partial = partial.clone();
                            last_change = Instant::now();
                            let _ = tx.send(Event::Partial(partial));
                        } else if last_change.elapsed() > Duration::from_millis(900) {
                            sp.store(false, Ordering::Relaxed);
                        }

                        if streaming.endpoint() {
                            let text1 = std::mem::take(&mut last_partial);
                            streaming.reset();
                            let segment = std::mem::take(&mut seg_buf);
                            if !text1.trim().is_empty() {
                                let _ = tx.send(Event::Commit(text1));
                                let text2 = stt::transcribe_chunked(&stt, &segment, 16_000);
                                if !text2.trim().is_empty() {
                                    let _ = tx.send(Event::Revise(text2));
                                }
                            } else if plausibly_speech(&segment) {
                                // The 20M streaming model is the weakest link — speech it
                                // mishears as nothing must still reach the strong model.
                                let text2 = stt::transcribe_chunked(&stt, &segment, 16_000);
                                if !text2.trim().is_empty() {
                                    let _ = tx.send(Event::Commit(text2)); // no prior block: a fresh commit
                                }
                            }
                            let _ = tx.send(Event::Partial(String::new()));
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        // No new audio: decay the listening latch after a pause.
                        if last_change.elapsed() > Duration::from_millis(900) {
                            sp.store(false, Ordering::Relaxed);
                        }
                    }
                    Err(_) => { let _ = tx.send(Event::Done); break; } // capture stopped → end cleanly
                }
                if ff.load(Ordering::Relaxed) {
                    // Up to one pass-2's worth of chunks may be parked in the channel —
                    // feed them all before flushing, or the last words are lost.
                    while let Ok(chunk) = samples.try_recv() {
                        let resampled = resample_to_16k(&chunk, cap_rate);
                        streaming.push(&resampled);
                        seg_buf.extend_from_slice(&resampled);
                    }
                    let text1 = streaming.partial();
                    let segment = std::mem::take(&mut seg_buf);
                    if !text1.trim().is_empty() {
                        let _ = tx.send(Event::Commit(text1));
                        let text2 = stt::transcribe_chunked(&stt, &segment, 16_000);
                        if !text2.trim().is_empty() { let _ = tx.send(Event::Revise(text2)); }
                    } else if plausibly_speech(&segment) {
                        let text2 = stt::transcribe_chunked(&stt, &segment, 16_000);
                        if !text2.trim().is_empty() { let _ = tx.send(Event::Commit(text2)); }
                    }
                    let _ = tx.send(Event::Done);
                    break;
                }
            }
        });
        LiveSource { _stream: stream, results: rx, speaking, finish_flag, worker: Some(worker) }
    }

    /// Cloneable control handles, so the live loop can drive finish / read
    /// "speaking" WITHOUT borrowing the LiveSource (which run_loop holds as
    /// `&mut dyn TranscriptSource`). Passing these Arcs avoids the double-borrow.
    pub fn finish_handle(&self) -> Arc<AtomicBool> { self.finish_flag.clone() }
    pub fn speaking_handle(&self) -> Arc<AtomicBool> { self.speaking.clone() }
}

impl Drop for LiveSource {
    fn drop(&mut self) {
        self.finish_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.worker.take() { let _ = h.join(); }
        // `_stream` drops last, stopping capture after the worker has joined.
    }
}

impl TranscriptSource for LiveSource {
    fn next(&mut self) -> Option<Event> {
        match self.results.try_recv() {
            Ok(ev) => Some(ev),
            Err(TryRecvError::Empty) => None,            // non-blocking: nothing ready yet
            Err(TryRecvError::Disconnected) => Some(Event::Done), // worker gone → end the session
        }
    }
}

/// A segment is plausibly speech (not a silence-only endpoint) when it runs longer
/// than a second AND its RMS clears a quiet-room floor. The rule1 2.4s trailing-
/// silence cycle endpoints on pure silence, which fails this — so those cost zero
/// Moonshine runs; only segments that might carry words the 20M misheard as nothing
/// are rescued by pass-2.
fn plausibly_speech(samples: &[f32]) -> bool {
    if (samples.len() as f32) <= 16_000.0 { return false; } // ≤ 1s
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    let rms = (sum_sq / samples.len() as f32).sqrt();
    rms > 0.01
}

/// Linear resample to 16 kHz (anti-aliasing is a known follow-up; the on-machine
/// WER check in T11 decides whether to swap in a filtered resampler like `rubato`).
fn resample_to_16k(input: &[f32], from_hz: u32) -> Vec<f32> {
    if from_hz == 16_000 || input.is_empty() { return input.to_vec(); }
    let ratio = 16_000.0 / from_hz as f32;
    let out_len = (input.len() as f32 * ratio) as usize;
    (0..out_len).map(|i| {
        let src = i as f32 / ratio;
        let lo = src.floor() as usize;
        let hi = (lo + 1).min(input.len() - 1);
        let frac = src - lo as f32;
        input[lo] * (1.0 - frac) + input[hi] * frac
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::{plausibly_speech, resample_to_16k};

    #[test]
    fn resample_is_identity_at_16k() {
        let s = vec![0.1, 0.2, 0.3];
        assert_eq!(resample_to_16k(&s, 16_000), s);
    }
    #[test]
    fn resample_downsamples_length_proportionally() {
        let s = vec![0.0; 48_000];
        let out = resample_to_16k(&s, 48_000);
        assert!((out.len() as i32 - 16_000).abs() < 10);
    }

    #[test]
    fn plausibly_speech_accepts_a_loud_tone() {
        let mut tone = vec![0.0f32; 0];
        for n in 0..32_000 { // 2s of 440 Hz at 0.3 amplitude → RMS ≈ 0.21
            let t = n as f32 / 16_000.0;
            tone.push((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.3);
        }
        assert!(plausibly_speech(&tone));
    }

    #[test]
    fn plausibly_speech_rejects_near_silence() {
        // 2s of near-zeros (well below the 0.01 RMS floor) — a silence-only endpoint.
        let quiet = vec![0.0001f32; 32_000];
        assert!(!plausibly_speech(&quiet));
        // And a short-but-loud blip fails the duration gate.
        let blip = vec![0.5f32; 8_000]; // 0.5s
        assert!(!plausibly_speech(&blip));
    }
}
