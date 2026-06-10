use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};

pub struct Stt {
    recognizer: OfflineRecognizer,
}

impl Stt {
    /// Paths from the unpacked quantized merged-decoder Moonshine tiny model dir:
    /// `encoder_model.ort`, `decoder_model_merged.ort`, `tokens.txt`.
    pub fn new(encoder: &str, decoder: &str, tokens: &str) -> Result<Stt, String> {
        let mut cfg = OfflineRecognizerConfig::default();
        cfg.model_config.moonshine.encoder = Some(encoder.to_string());
        cfg.model_config.moonshine.merged_decoder = Some(decoder.to_string());
        cfg.model_config.tokens = Some(tokens.to_string());
        cfg.model_config.provider = Some("cpu".to_string());
        cfg.model_config.num_threads = 2;
        let recognizer = OfflineRecognizer::create(&cfg)
            .ok_or_else(|| "failed to create Moonshine recognizer (check model paths)".to_string())?;
        Ok(Stt { recognizer })
    }

    /// Transcribe one VAD segment (16 kHz mono).
    pub fn transcribe(&self, samples: &[f32], sample_rate: i32) -> String {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(sample_rate, samples);
        self.recognizer.decode(&stream);
        stream.get_result().map(|r| r.text).unwrap_or_default()
    }
}

/// The quantized Moonshine export decodes only short segments reliably — its
/// decoder errors past the envelope and sherpa returns EMPTY text (measured on
/// the pinned 2026-02-27 export: 8s ok, 10s fails), which once silently erased
/// an entire pause-free monologue.
const MAX_DECODE_SECS: f32 = 8.0;
/// Preferred chunk cut point, searched ±CUT_SLACK_SECS for the quietest gap.
const CUT_TARGET_SECS: f32 = 7.0;
const CUT_SLACK_SECS: f32 = 0.75;

/// Transcribe a segment of ANY length: segments inside the model envelope go
/// straight through; longer ones are split at low-energy points (breath gaps,
/// not mid-word, when possible) and the per-chunk transcripts joined — so no
/// segment can ever vanish into an empty result.
pub fn transcribe_chunked(stt: &Stt, samples: &[f32], sample_rate: i32) -> String {
    let max = (MAX_DECODE_SECS * sample_rate as f32) as usize;
    let mut out: Vec<String> = Vec::new();
    let mut rest = samples;
    while rest.len() > max {
        let cut = quietest_cut(rest, sample_rate);
        let (head, tail) = rest.split_at(cut);
        let t = stt.transcribe(head, sample_rate);
        if !t.trim().is_empty() {
            out.push(t.trim().to_string());
        }
        rest = tail;
    }
    let t = stt.transcribe(rest, sample_rate);
    if !t.trim().is_empty() {
        out.push(t.trim().to_string());
    }
    out.join(" ")
}

/// The index (in samples) of the quietest 100ms window centered around
/// CUT_TARGET_SECS, searched within ±CUT_SLACK_SECS — a likely word gap.
fn quietest_cut(samples: &[f32], sample_rate: i32) -> usize {
    let rate = sample_rate as f32;
    let lo = ((CUT_TARGET_SECS - CUT_SLACK_SECS) * rate) as usize;
    let hi = (((CUT_TARGET_SECS + CUT_SLACK_SECS) * rate) as usize).min(samples.len() - 1);
    let win = (rate * 0.1) as usize;
    let step = (rate * 0.05) as usize;
    let mut best = (f32::MAX, lo);
    let mut i = lo;
    while i + win <= hi {
        let energy: f32 = samples[i..i + win].iter().map(|s| s * s).sum();
        if energy < best.0 {
            best = (energy, i + win / 2);
        }
        i += step;
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quietest_cut_lands_in_the_silent_gap() {
        // 10s of "speech" (constant tone) with one silent 200ms dip at 7.3s.
        let rate = 16_000;
        let mut s = vec![0.5f32; 10 * rate];
        let dip = (7.3 * rate as f32) as usize;
        for x in &mut s[dip..dip + (rate / 5)] {
            *x = 0.0;
        }
        let cut = quietest_cut(&s, rate as i32);
        let cut_secs = cut as f32 / rate as f32;
        assert!((7.3..7.6).contains(&cut_secs), "cut at {cut_secs}s, expected inside the dip");
    }

    #[test]
    fn quietest_cut_stays_inside_the_search_band() {
        let rate = 16_000;
        let s = vec![0.5f32; 12 * rate]; // uniform: any window ties, first wins
        let cut = quietest_cut(&s, rate as i32);
        let cut_secs = cut as f32 / rate as f32;
        assert!((6.2..=7.8).contains(&cut_secs), "cut at {cut_secs}s outside ±slack band");
    }
}
