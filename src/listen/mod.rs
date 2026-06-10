pub mod capture;
pub mod resample;
pub mod streaming;
pub mod stt;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use crate::source::{Event, PauseSignal, TranscriptSource};
use capture::Capture;
use resample::Resampler;
use streaming::Streaming;
use stt::Stt;

/// Live mic → streaming Zipformer (pass 1) → Whisper base.en (pass 2), running both
/// passes on a WORKER THREAD so the UI loop never blocks. `next()` is non-blocking:
/// it drains a results channel. The worker is serial — it emits `Commit(streaming)`,
/// runs pass-2, emits `Revise(better)`, then resumes feeding — so a Revise always
/// targets the block it just committed (no races, no extra threads).
pub struct LiveSource {
    _stream: cpal::Stream,            // kept alive HERE on the owning thread (!Send)
    results: Receiver<Event>,
    speaking: Arc<AtomicBool>,
    finish_flag: Arc<AtomicBool>,
    pause: Arc<PauseSignal>,
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
        let pause = Arc::new(PauseSignal::default());
        let (sp, ff, pa) = (speaking.clone(), finish_flag.clone(), pause.clone());
        let worker = thread::spawn(move || {
            // Worker-local state: the last partial we emitted (so we only push on
            // change), when it last changed (drives the partial-activity latch +
            // its silence decay), and the audio of the current segment (fed to
            // pass-2 on endpoint).
            let mut last_partial = String::new();
            let mut last_change = Instant::now();
            let mut seg_buf: Vec<f32> = Vec::new();
            let mut resampler = Resampler::new(cap_rate);
            let mut seen_epoch = pa.epoch();
            loop {
                // Pause is OFF-RECORD at the audio level, not just an event filter.
                // The EPOCH (counts pause entries) is the trigger, not the flag: a
                // pause+resume window contained entirely inside a pass-2 block would
                // be invisible to a flag poll, but the epoch still advances. On any
                // pause having happened: destroy the in-flight hypothesis, the
                // segment audio, AND the parked chunks (speech in flight at pause is
                // off-record by contract). While paused, chunks are discarded below.
                let paused = pa.is_paused();
                let epoch = pa.epoch();
                if epoch != seen_epoch {
                    seen_epoch = epoch;
                    streaming.reset();
                    resampler.reset();
                    seg_buf.clear();
                    last_partial.clear();
                    sp.store(false, Ordering::Relaxed);
                    while samples.try_recv().is_ok() {}
                }
                match samples.recv_timeout(Duration::from_millis(50)) {
                    Ok(_) if paused => {} // off-record: drop the audio itself
                    Ok(chunk) => {
                        let resampled = resampler.process(&chunk);
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
                            } else if plausibly_speech(&segment, RESCUE_MIN_ENDPOINT_SAMPLES) {
                                // The 20M streaming model is the weakest link — speech it
                                // mishears as nothing must still reach the strong model.
                                let text2 = stt::transcribe_chunked(&stt, &segment, 16_000);
                                let secs = segment.len() as f32 / 16_000.0;
                                if !text2.trim().is_empty()
                                    && !stt::suspect_hallucination(&text2, secs)
                                {
                                    // Self-paired so the rescue renders bright (= pass-2-final)
                                    // like every other finished phrase.
                                    let _ = tx.send(Event::Commit(text2.clone()));
                                    let _ = tx.send(Event::Revise(text2));
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
                    // (While paused they are off-record: drain and discard.)
                    while let Ok(chunk) = samples.try_recv() {
                        if !paused {
                            let resampled = resampler.process(&chunk);
                            streaming.push(&resampled);
                            seg_buf.extend_from_slice(&resampled);
                        }
                    }
                    let text1 = streaming.partial();
                    let segment = std::mem::take(&mut seg_buf);
                    if !text1.trim().is_empty() {
                        let _ = tx.send(Event::Commit(text1));
                        let text2 = stt::transcribe_chunked(&stt, &segment, 16_000);
                        if !text2.trim().is_empty() { let _ = tx.send(Event::Revise(text2)); }
                    } else if plausibly_speech(&segment, RESCUE_MIN_FLUSH_SAMPLES) {
                        let text2 = stt::transcribe_chunked(&stt, &segment, 16_000);
                        let secs = segment.len() as f32 / 16_000.0;
                        if !text2.trim().is_empty()
                            && !stt::suspect_hallucination(&text2, secs)
                        {
                            let _ = tx.send(Event::Commit(text2.clone()));
                            let _ = tx.send(Event::Revise(text2));
                        }
                    }
                    let _ = tx.send(Event::Done);
                    break;
                }
            }
        });
        LiveSource { _stream: stream, results: rx, speaking, finish_flag, pause, worker: Some(worker) }
    }

    /// Cloneable control handles, so the live loop can drive finish / pause / read
    /// "speaking" WITHOUT borrowing the LiveSource (which run_loop holds as
    /// `&mut dyn TranscriptSource`). Passing these Arcs avoids the double-borrow.
    pub fn finish_handle(&self) -> Arc<AtomicBool> { self.finish_flag.clone() }
    pub fn speaking_handle(&self) -> Arc<AtomicBool> { self.speaking.clone() }
    pub fn pause_handle(&self) -> Arc<PauseSignal> { self.pause.clone() }
}

impl Drop for LiveSource {
    fn drop(&mut self) {
        self.finish_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.worker.take() {
            // A wedged FFI call would make a bare join() freeze the terminal forever
            // (the UI loop's 8s drain deadline has already given up by then). Grace
            // period, then detach — the OS reclaims the thread at process exit.
            let deadline = Instant::now() + Duration::from_secs(10);
            while !h.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(50));
            }
            if h.is_finished() {
                let _ = h.join();
            } else {
                thread::spawn(move || { let _ = h.join(); });
            }
        }
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

/// Quiet-room RMS floor for the rescue gate, applied to the loudest window (below).
const RESCUE_RMS_FLOOR: f32 = 0.01;
/// RMS window: 0.5s at 16 kHz. Measured over the LOUDEST window, not the whole
/// segment — every rule1 endpoint necessarily appends ≥2.4s of trailing silence,
/// and a whole-segment RMS diluted by that silence would reject exactly the quiet
/// speech the rescue exists to save.
const RESCUE_RMS_WINDOW: usize = 8_000;
/// Duration floor at an endpoint rescue: 1s. An endpointed segment that short is
/// noise (a blip can't both carry words and evade the streaming model entirely).
const RESCUE_MIN_ENDPOINT_SAMPLES: usize = 16_000;
/// Duration floor at the finish flush: 0.35s. The user deliberately ended here —
/// a short final word ("okay.") must not vanish into the endpoint-sized gate.
const RESCUE_MIN_FLUSH_SAMPLES: usize = 5_600;

/// A segment is plausibly speech (not a silence-only endpoint) when it outlasts the
/// caller's duration floor AND its loudest 0.5s window clears the quiet-room RMS
/// floor. Pure-silence endpoints fail this, so they cost zero Whisper runs; only
/// segments that might carry words the 20M misheard as nothing are rescued by pass-2.
fn plausibly_speech(samples: &[f32], min_samples: usize) -> bool {
    samples.len() > min_samples && peak_window_rms(samples) > RESCUE_RMS_FLOOR
}

/// The highest RMS over any 0.5s window (0.1s hop); whole-buffer RMS when shorter.
fn peak_window_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() { return 0.0; }
    if samples.len() <= RESCUE_RMS_WINDOW {
        let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
        return (sum_sq / samples.len() as f32).sqrt();
    }
    let hop = RESCUE_RMS_WINDOW / 5;
    let mut best = 0.0f32;
    let mut i = 0;
    while i + RESCUE_RMS_WINDOW <= samples.len() {
        let sum_sq: f32 = samples[i..i + RESCUE_RMS_WINDOW].iter().map(|s| s * s).sum();
        best = best.max((sum_sq / RESCUE_RMS_WINDOW as f32).sqrt());
        i += hop;
    }
    best
}

#[cfg(test)]
mod tests {
    use super::{plausibly_speech, RESCUE_MIN_ENDPOINT_SAMPLES, RESCUE_MIN_FLUSH_SAMPLES};

    fn tone(secs: f32, amplitude: f32) -> Vec<f32> {
        (0..(secs * 16_000.0) as usize)
            .map(|n| {
                let t = n as f32 / 16_000.0;
                (2.0 * std::f32::consts::PI * 440.0 * t).sin() * amplitude
            })
            .collect()
    }

    #[test]
    fn plausibly_speech_accepts_a_loud_tone() {
        // 2s of 440 Hz at 0.3 amplitude → RMS ≈ 0.21
        assert!(plausibly_speech(&tone(2.0, 0.3), RESCUE_MIN_ENDPOINT_SAMPLES));
    }

    #[test]
    fn plausibly_speech_rejects_near_silence() {
        // 4s of near-zeros (well below the 0.01 RMS floor) — a silence-only endpoint.
        let quiet = vec![0.0001f32; 64_000];
        assert!(!plausibly_speech(&quiet, RESCUE_MIN_ENDPOINT_SAMPLES));
        // And a short-but-loud blip fails the duration gate.
        let blip = vec![0.5f32; 8_000]; // 0.5s
        assert!(!plausibly_speech(&blip, RESCUE_MIN_ENDPOINT_SAMPLES));
    }

    #[test]
    fn quiet_speech_survives_the_endpoint_silence_dilution() {
        // 1.5s of quiet speech (RMS ≈ 0.015, above the 0.01 floor) followed by the
        // 2.4s of trailing silence every rule1 endpoint mandates. A whole-segment
        // RMS dilutes to ≈0.009 and would wrongly reject it; the loudest-window
        // RMS must still see the speech.
        let mut seg = tone(1.5, 0.015 * std::f32::consts::SQRT_2);
        seg.extend(vec![0.0f32; (2.4 * 16_000.0) as usize]);
        assert!(plausibly_speech(&seg, RESCUE_MIN_ENDPOINT_SAMPLES));
    }

    #[test]
    fn duration_floor_is_exclusive_at_the_boundary() {
        let exactly_floor = vec![0.5f32; RESCUE_MIN_ENDPOINT_SAMPLES];
        assert!(!plausibly_speech(&exactly_floor, RESCUE_MIN_ENDPOINT_SAMPLES));
        let one_past = vec![0.5f32; RESCUE_MIN_ENDPOINT_SAMPLES + 1];
        assert!(plausibly_speech(&one_past, RESCUE_MIN_ENDPOINT_SAMPLES));
    }

    #[test]
    fn flush_floor_rescues_a_short_final_word() {
        // ~0.5s of speech-level audio: fails the endpoint floor (1s) but must pass
        // the flush floor — the user deliberately ended on a short word.
        let word = tone(0.5, 0.3);
        assert!(!plausibly_speech(&word, RESCUE_MIN_ENDPOINT_SAMPLES));
        assert!(plausibly_speech(&word, RESCUE_MIN_FLUSH_SAMPLES));
    }
}
