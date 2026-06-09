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
