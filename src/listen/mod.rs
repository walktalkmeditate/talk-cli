pub mod capture;
pub mod stt;
pub mod vad;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use crate::source::{Event, TranscriptSource};
use capture::Capture;
use stt::Stt;
use vad::Segmenter;

/// Live mic → VAD → Moonshine, running the VAD+STT on a WORKER THREAD so the UI
/// loop never blocks. `next()` is non-blocking: it drains a results channel.
pub struct LiveSource {
    _stream: cpal::Stream,            // kept alive HERE on the owning thread (!Send)
    results: Receiver<Event>,
    speaking: Arc<AtomicBool>,
    finish_flag: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl LiveSource {
    pub fn new(capture: Capture, mut seg: Segmenter, stt: Stt) -> Self {
        // Only the Receiver (Send) + seg + stt (both Send per sherpa-onnx) move to
        // the worker; the cpal Stream (!Send) stays on this thread.
        let (stream, samples, cap_rate) = capture.into_parts();
        let (tx, rx): (Sender<Event>, Receiver<Event>) = std::sync::mpsc::channel();
        let speaking = Arc::new(AtomicBool::new(false));
        let finish_flag = Arc::new(AtomicBool::new(false));
        let (sp, ff) = (speaking.clone(), finish_flag.clone());
        let worker = thread::spawn(move || {
            loop {
                match samples.recv_timeout(std::time::Duration::from_millis(50)) {
                    Ok(chunk) => {
                        let resampled = resample_to_16k(&chunk, cap_rate);
                        for s in seg.push(&resampled) {
                            let text = stt.transcribe(&s.samples, s.sample_rate);
                            if !text.trim().is_empty() { let _ = tx.send(Event::Commit(text)); }
                        }
                        sp.store(seg.is_speaking(), Ordering::Relaxed);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        sp.store(seg.is_speaking(), Ordering::Relaxed);
                    }
                    Err(_) => { let _ = tx.send(Event::Done); break; } // capture stopped → end cleanly
                }
                if ff.load(Ordering::Relaxed) {
                    for s in seg.flush() {
                        let text = stt.transcribe(&s.samples, s.sample_rate);
                        if !text.trim().is_empty() { let _ = tx.send(Event::Commit(text)); }
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
    use super::resample_to_16k;
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
}
