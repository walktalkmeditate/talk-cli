use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};

pub struct Stt {
    recognizer: OfflineRecognizer,
}

impl Stt {
    /// Paths from the unpacked Whisper base.en (int8) model dir:
    /// `base.en-encoder.int8.onnx`, `base.en-decoder.int8.onnx`, `base.en-tokens.txt`.
    pub fn new(encoder: &str, decoder: &str, tokens: &str) -> Result<Stt, String> {
        let mut cfg = OfflineRecognizerConfig::default();
        cfg.model_config.whisper.encoder = Some(encoder.to_string());
        cfg.model_config.whisper.decoder = Some(decoder.to_string());
        cfg.model_config.whisper.language = Some("en".to_string());
        cfg.model_config.whisper.task = Some("transcribe".to_string());
        cfg.model_config.tokens = Some(tokens.to_string());
        cfg.model_config.provider = Some("cpu".to_string());
        cfg.model_config.num_threads = 2;
        let recognizer = OfflineRecognizer::create(&cfg)
            .ok_or_else(|| "failed to create Whisper recognizer (check model paths)".to_string())?;
        Ok(Stt { recognizer })
    }

    /// Transcribe one 16 kHz mono segment. Whisper takes one ≤30s window; use
    /// `transcribe_chunked` for anything that could exceed it.
    pub fn transcribe(&self, samples: &[f32], sample_rate: i32) -> String {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(sample_rate, samples);
        self.recognizer.decode(&stream);
        stream.get_result().map(|r| r.text.trim().to_string()).unwrap_or_default()
    }
}

/// Whisper decodes a fixed 30 s mel window; segments are ≤20 s (rule3 cap), so a
/// single call covers them. The chunker only fires defensively past 30 s.
const MAX_DECODE_SECS: f32 = 30.0;
/// Preferred cut point, searched ±CUT_SLACK_SECS for the quietest gap.
const CUT_TARGET_SECS: f32 = 28.0;
const CUT_SLACK_SECS: f32 = 1.5;

/// Transcribe a segment of ANY length: segments inside Whisper's 30 s window go
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

/// A Whisper transcript is a suspected hallucination over near-silence when it is
/// a single token repeated or implausibly dense for the audio length. Used only on
/// the quiet-speech rescue path (pass-1 found nothing), where Whisper is most prone
/// to inventing text from silence.
pub fn suspect_hallucination(text: &str, audio_secs: f32) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 4 {
        return false;
    }
    if words.len() as f32 > audio_secs * 4.0 + 4.0 {
        return true;
    }
    let distinct: std::collections::HashSet<String> =
        words.iter().map(|w| w.to_lowercase()).collect();
    distinct.len() == 1
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

    #[test]
    fn suspect_hallucination_flags_repetition_and_density() {
        assert!(suspect_hallucination("you you you you", 3.0));
        assert!(suspect_hallucination("a b c d e f g h i j k l", 1.0));
        assert!(!suspect_hallucination("oh well that worked", 1.3));
        assert!(!suspect_hallucination("", 2.0));
    }
}
