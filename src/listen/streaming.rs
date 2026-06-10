use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};

/// Streaming first-pass recognizer (Zipformer-20M transducer) with built-in
/// endpointing. Emits revising partials while speech is ongoing; `endpoint()`
/// fires on a pause (rule2: 0.8s trailing silence — the settle-on-pause feel)
/// or a 20s utterance cap (rule3 — bounds the pass-2 segment length).
pub struct Streaming {
    // Field order is load-bearing: fields drop in declaration order, and the C API
    // requires streams destroyed BEFORE their recognizer (the recognizer owns the ORT env).
    stream: OnlineStream,
    recognizer: OnlineRecognizer,
}

impl Streaming {
    pub fn new(encoder: &str, decoder: &str, joiner: &str, tokens: &str) -> Result<Streaming, String> {
        let mut cfg = OnlineRecognizerConfig::default();
        cfg.model_config.transducer.encoder = Some(encoder.to_string());
        cfg.model_config.transducer.decoder = Some(decoder.to_string());
        cfg.model_config.transducer.joiner = Some(joiner.to_string());
        cfg.model_config.tokens = Some(tokens.to_string());
        cfg.model_config.num_threads = 2;
        cfg.model_config.provider = Some("cpu".to_string());
        cfg.enable_endpoint = true;
        cfg.rule1_min_trailing_silence = 2.4;
        cfg.rule2_min_trailing_silence = 0.8;
        cfg.rule3_min_utterance_length = 20.0;
        let recognizer = OnlineRecognizer::create(&cfg)
            .ok_or_else(|| "failed to create streaming recognizer (check zipformer paths)".to_string())?;
        let stream = recognizer.create_stream();
        Ok(Streaming { recognizer, stream })
    }

    /// Feed a 16 kHz mono chunk and decode whatever is ready.
    pub fn push(&mut self, samples: &[f32]) {
        self.stream.accept_waveform(16_000, samples);
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
    }

    /// The current partial hypothesis, lowercased (the 20M export shouts).
    pub fn partial(&self) -> String {
        self.recognizer
            .get_result(&self.stream)
            .map(|r| r.text.to_lowercase())
            .unwrap_or_default()
    }

    pub fn endpoint(&self) -> bool {
        self.recognizer.is_endpoint(&self.stream)
    }

    /// Reset for the next utterance (after taking the endpointed text).
    pub fn reset(&mut self) {
        self.recognizer.reset(&self.stream);
    }
}
