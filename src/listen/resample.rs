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
/// Below this, sinc's argument is treated as its removable singularity (value 1).
const SINC_NEAR_ZERO: f64 = 1e-9;
/// Weight-sum floor: below this the normalized output is taken as 0 (never reached
/// in practice — the center tap alone keeps the sum well clear of zero).
const WSUM_MIN: f64 = 1e-12;

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
        // A 0 rate would make step 0 (the emit loop never advances → hang) and
        // cutoff NaN. cpal never reports 0, but fail loud rather than wedge.
        assert!(in_rate > 0, "Resampler: in_rate must be > 0");
        let out_rate = 16_000.0f64;
        // Low-pass at the lower of the two Nyquists — the output Nyquist when
        // downsampling, which is what rejects the aliasing.
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
        // A single non-finite sample would smear across the whole kernel into the
        // ONNX models; clamp it to silence at the boundary instead.
        self.buf.extend(chunk.iter().map(|&s| if s.is_finite() { s } else { 0.0 }));
        let mut out = Vec::with_capacity((chunk.len() as f64 / self.step) as usize + 1);
        // An output needs HALF input samples of right context for the kernel; the
        // +1 keeps the top index strictly inside buf (sample_at reads up to floor+HALF).
        while self.phase + HALF as f64 + 1.0 < self.buf.len() as f64 {
            out.push(self.sample_at(self.phase));
            self.phase += self.step;
        }
        // Drop consumed input but keep HALF samples of left context for the kernel.
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
        if wsum.abs() < WSUM_MIN { 0.0 } else { (acc / wsum) as f32 }
    }
}

fn sinc(x: f64) -> f64 {
    if x.abs() < SINC_NEAR_ZERO {
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
    fn reset_discards_pre_reset_audio() {
        // Load loud audio, reset, then feed SILENCE: the output must be silent —
        // proving the pre-reset left context is gone, not just that output exists.
        let mut r = Resampler::new(48_000);
        let _ = r.process(&tone(1_000.0, 48_000, 0.2));
        r.reset();
        let out = r.process(&vec![0.0f32; 48_000]);
        assert!(out.len() > 1_000);
        assert!(rms(&out) < 0.01, "pre-reset audio bled through: rms {}", rms(&out));
    }

    #[test]
    fn upsamples_from_8k() {
        // 8 kHz in → 16 kHz out: ratio 0.5, cutoff = input Nyquist. 1 kHz survives.
        // The single call holds back HALF input samples of right-context latency,
        // which is HALF/step = 64 output samples here — so allow for that tail.
        let mut r = Resampler::new(8_000);
        let out = r.process(&tone(1_000.0, 8_000, 0.5));
        assert!((8_000 - out.len() as i32).abs() < 80, "got {} samples", out.len());
        assert!(rms(&out) > 0.3, "1 kHz lost on upsample: rms {}", rms(&out));
    }

    #[test]
    fn non_integer_ratio_44100_length_stays_proportional() {
        // 44.1 kHz is the other common device rate (non-integer ratio): length must
        // track 16000/44100 with no drift accumulating across chunk boundaries.
        let mut r = Resampler::new(44_100);
        let sig = tone(440.0, 44_100, 3.0);
        let mut total = 0usize;
        for c in sig.chunks(441) {
            total += r.process(c).len();
        }
        assert!((total as i32 - 48_000).abs() < 64, "3s → {} samples (expected ~48000)", total);
    }

    #[test]
    fn empty_chunk_yields_no_output() {
        let mut r = Resampler::new(48_000);
        assert!(r.process(&[]).is_empty());
    }

    #[test]
    fn non_finite_input_is_clamped_to_silence() {
        let mut r = Resampler::new(48_000);
        let mut sig = tone(440.0, 48_000, 0.2);
        sig[1000] = f32::NAN;
        sig[2000] = f32::INFINITY;
        let out = r.process(&sig);
        assert!(out.iter().all(|s| s.is_finite()), "non-finite leaked into output");
    }
}
