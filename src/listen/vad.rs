use sherpa_onnx::{VoiceActivityDetector, VadModelConfig};

/// A completed speech segment (mono f32 @ 16 kHz).
pub struct Segment {
    pub samples: Vec<f32>,
    pub sample_rate: i32,
}

const WINDOW: usize = 512; // Silero VAD requires fixed 512-sample windows.

pub struct Segmenter {
    vad: VoiceActivityDetector,
    buf: Vec<f32>,
    sample_rate: i32,
}

impl Segmenter {
    /// `model` = path to silero_vad.onnx. Input must be 16 kHz mono (caller resamples).
    pub fn new(model: &str) -> Result<Segmenter, String> {
        let mut cfg = VadModelConfig::default();
        cfg.silero_vad.model = Some(model.to_string());
        cfg.silero_vad.threshold = 0.5;
        cfg.silero_vad.min_silence_duration = 0.8; // reflection has long mid-thought pauses; tune on-machine
        cfg.silero_vad.min_speech_duration = 0.25;
        cfg.silero_vad.window_size = WINDOW as i32;
        cfg.sample_rate = 16_000;
        let vad = VoiceActivityDetector::create(&cfg, 30.0)
            .ok_or_else(|| "failed to create VAD (check silero_vad.onnx path)".to_string())?;
        Ok(Segmenter { vad, buf: Vec::new(), sample_rate: 16_000 })
    }

    /// Feed a chunk; returns any segments that completed (speaker paused).
    /// Buffers and feeds the VAD ONLY in fixed 512-sample windows.
    pub fn push(&mut self, chunk: &[f32]) -> Vec<Segment> {
        self.buf.extend_from_slice(chunk);
        let mut offset = 0;
        while offset + WINDOW <= self.buf.len() {
            self.vad.accept_waveform(&self.buf[offset..offset + WINDOW]);
            offset += WINDOW;
        }
        self.buf.drain(..offset);
        self.drain()
    }

    fn drain(&mut self) -> Vec<Segment> {
        let mut out = Vec::new();
        while let Some(seg) = self.vad.front() {
            out.push(Segment { samples: seg.samples().to_vec(), sample_rate: self.sample_rate });
            self.vad.pop();
        }
        out
    }

    /// True while the VAD currently hears speech (drives the listening indicator).
    pub fn is_speaking(&self) -> bool { self.vad.detected() }

    /// On finish, flush any in-progress segment.
    pub fn flush(&mut self) -> Vec<Segment> {
        self.vad.flush();
        self.drain()
    }
}
