//! Anti-aliasing resampler to 16 kHz (the rate both models want).
//!
//! The mic runs at the device default (48 kHz here). Naive linear downsampling
//! has no low-pass, so every frequency above the 8 kHz output-Nyquist FOLDS back
//! into the speech band — a 12 kHz sibilant lands at 4 kHz, smearing exactly the
//! consonants that distinguish "worked"/"work" and "sometimes"/"times". This is a
//! stateful windowed-sinc resampler: it low-passes at the output Nyquist as part of
//! the same pass, so high frequencies are REJECTED, not aliased. State carries the
//! filter's left context across chunks, so there are no per-chunk boundary clicks.

/// Kernel half-width in INPUT samples. 32 gives a usable transition band with the
/// Blackman window; cost is ~2*32 mul-adds per output sample (~1M/s — negligible).
const HALF: usize = 32;

pub struct Resampler {
    in_rate: u32,
    /// Input samples consumed per output sample (in_rate / 16000).
    step: f64,
    /// Low-pass cutoff in cycles/input-sample: min(input-Nyquist, output-Nyquist).
    cutoff: f64,
    /// Unconsumed input, front-padded with HALF samples of left context.
    buf: Vec<f32>,
    /// Position of the next output sample, in input samples from `buf[0]`.
    phase: f64,
}

impl Resampler {
    pub fn new(in_rate: u32) -> Resampler {
        let out_rate = 16_000.0f64;
        // Cutoff at the lower of the two Nyquists (output-Nyquist when downsampling).
        let cutoff = (out_rate / 2.0).min(in_rate as f64 / 2.0) / in_rate as f64;
        Resampler {
            in_rate,
            step: in_rate as f64 / out_rate,
            cutoff,
            buf: vec![0.0; HALF], // HALF samples of zero left-context to start
            phase: HALF as f64,
            }
    }

    /// Drop all pending state (e.g. on pause, so pre-pause audio never bleeds into
    /// the post-resume output).
    pub fn reset(&mut self) {
        self.buf.clear();
        self.buf.resize(HALF, 0.0);
        self.phase = HALF as f64;
    }

    /// Resample one chunk of input to 16 kHz, carrying filter state forward.
    pub fn process(&mut self, chunk: &[f32]) -> Vec<f32> {
        if self.in_rate == 16_000 {
            return chunk.to_vec();
        }
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        // Emit while we still have HALF samples of right context for the kernel.
        while self.phase + HALF as f64 + 1.0 < self.buf.len() as f64 {
            out.push(self.sample_at(self.phase));
            self.phase += self.step;
        }
        // Drain consumed input, keeping HALF samples of left context.
        let keep_from = (self.phase.floor() as usize).saturating_sub(HALF);
        if keep_from > 0 {
            self.buf.drain(0..keep_from);
            self.phase -= keep_from as f64;
        }
        out
    }

    /// One output sample: normalized windowed-sinc over the surrounding input.
    /// Normalizing by the weight sum pins passband (DC) gain to 1 at every phase.
    fn sample_at(&self, p: f64) -> f32 {
        let center = p.floor() as isize;
        let mut acc = 0.0f64;
        let mut wsum = 0.0f64;
        for i in (center - HALF as isize + 1)..=(center + HALF as isize) {
            if i < 0 || i as usize >= self.buf.len() {
                continue;
            }
            let dt = p - i as f64; // distance in input samples
            let weight = sinc(2.0 * self.cutoff * dt) * blackman(dt / HALF as f64);
            acc += self.buf[i as usize] as f64 * weight;
            wsum += weight;
        }
        if wsum.abs() < 1e-12 { 0.0 } else { (acc / wsum) as f32 }
    }
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        let pix = std::f64::consts::PI * x;
        pix.sin() / pix
    }
}

/// Blackman window over t in [-1, 1]; zero outside.
fn blackman(t: f64) -> f64 {
    if t.abs() > 1.0 {
        return 0.0;
    }
    let x = std::f64::consts::PI * (t + 1.0); // map [-1,1] -> [0, 2pi]
    0.42 - 0.5 * x.cos() + 0.08 * (2.0 * x).cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f64, rate: u32, secs: f64) -> Vec<f32> {
        (0..(secs * rate as f64) as usize)
            .map(|n| (2.0 * std::f64::consts::PI * freq * n as f64 / rate as f64).sin() as f32)
            .collect()
    }

    fn rms(s: &[f32]) -> f64 {
        if s.is_empty() { return 0.0; }
        (s.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / s.len() as f64).sqrt()
    }

    #[test]
    fn identity_at_16k() {
        let mut r = Resampler::new(16_000);
        let s = vec![0.1, -0.2, 0.3];
        assert_eq!(r.process(&s), s);
    }

    #[test]
    fn downsamples_length_proportionally() {
        let mut r = Resampler::new(48_000);
        let out = r.process(&vec![0.0; 48_000]);
        assert!((out.len() as i32 - 16_000).abs() < 64, "got {} samples", out.len());
    }

    #[test]
    fn passband_tone_is_preserved() {
        // 1 kHz is well inside the 8 kHz passband: amplitude must survive.
        let mut r = Resampler::new(48_000);
        let out = r.process(&tone(1_000.0, 48_000, 0.5));
        let r_out = rms(&out);
        // input sine RMS is ~0.707; allow generous tolerance for edge transients.
        assert!(r_out > 0.5, "1 kHz attenuated too much: rms {r_out}");
    }

    #[test]
    fn out_of_band_tone_is_rejected_not_aliased() {
        // 12 kHz is ABOVE the 8 kHz output-Nyquist. A correct anti-aliasing resampler
        // attenuates it heavily; naive linear would alias it to 4 kHz at full
        // amplitude. This is the whole point of the change.
        let mut r = Resampler::new(48_000);
        let out = r.process(&tone(12_000.0, 48_000, 0.5));
        let r_out = rms(&out);
        assert!(r_out < 0.1, "12 kHz not rejected (aliasing!): rms {r_out}");
    }

    #[test]
    fn chunked_matches_whole_within_tolerance() {
        // Streaming in small chunks must produce ~the same output as one big call,
        // proving the cross-chunk state is correct (no boundary discontinuities).
        let sig = tone(2_000.0, 48_000, 0.3);
        let mut whole = Resampler::new(48_000);
        let a = whole.process(&sig);
        let mut chunked = Resampler::new(48_000);
        let mut b = Vec::new();
        for c in sig.chunks(777) {
            b.extend(chunked.process(c));
        }
        let n = a.len().min(b.len());
        assert!((a.len() as i32 - b.len() as i32).abs() <= 1, "lengths {} vs {}", a.len(), b.len());
        let max_diff = (0..n).map(|i| (a[i] - b[i]).abs()).fold(0.0f32, f32::max);
        assert!(max_diff < 1e-4, "chunk boundary mismatch: max diff {max_diff}");
    }

    #[test]
    fn reset_clears_pending_state() {
        let mut r = Resampler::new(48_000);
        let _ = r.process(&tone(1_000.0, 48_000, 0.1));
        r.reset();
        // After reset the next output starts clean (no panic, sane length).
        let out = r.process(&tone(1_000.0, 48_000, 0.1));
        assert!(out.len() > 1_000);
    }
}
