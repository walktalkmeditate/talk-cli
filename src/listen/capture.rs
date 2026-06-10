use std::sync::mpsc::{self, Receiver, SyncSender};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub struct Capture {
    _stream: cpal::Stream,
    pub samples: Receiver<Vec<f32>>,
    pub sample_rate: u32,
}

impl Capture {
    /// Open the default input device and stream mono f32 chunks to the channel.
    pub fn start() -> Result<Capture, String> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or("no input device")?;
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        let sample_rate = supported.sample_rate().0;
        let channels = supported.config().channels as usize;
        // Bounded: prevents an unbounded backlog if STT lags. Full → drop the chunk.
        // 1024 chunks ≈ ~10s headroom at ~10ms macOS callbacks — enough to absorb a
        // worst-case pass-2 stall (~0.3–1.5s after a rule3 endpoint) without dropping
        // mid-speech chunks (64 ≈ 0.6s was less than one pass-2, so words were lost).
        let (tx, rx): (SyncSender<Vec<f32>>, Receiver<Vec<f32>>) = mpsc::sync_channel(1024);

        let err_fn = |e| eprintln!("audio stream error: {e}");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &supported.config(),
                move |data: &[f32], _: &_| { let _ = tx.try_send(downmix(data, channels)); },
                err_fn, None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &supported.config(),
                move |data: &[i16], _: &_| {
                    let f: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
                    let _ = tx.try_send(downmix(&f, channels));
                },
                err_fn, None,
            ),
            cpal::SampleFormat::I32 => device.build_input_stream(
                &supported.config(),
                move |data: &[i32], _: &_| {
                    let f: Vec<f32> = data.iter().map(|&s| s as f32 / i32::MAX as f32).collect();
                    let _ = tx.try_send(downmix(&f, channels));
                },
                err_fn, None,
            ),
            cpal::SampleFormat::U8 => device.build_input_stream(
                &supported.config(),
                move |data: &[u8], _: &_| {
                    let f: Vec<f32> = data.iter().map(|&s| (s as f32 - 128.0) / 128.0).collect();
                    let _ = tx.try_send(downmix(&f, channels));
                },
                err_fn, None,
            ),
            other => return Err(format!("unsupported audio input format {other:?} — try a different input device")),
        }.map_err(|e| e.to_string())?;

        stream.play().map_err(|e| e.to_string())?;
        Ok(Capture { _stream: stream, samples: rx, sample_rate })
    }

    /// Split into (the live Stream — keep it on the OWNING thread; cpal::Stream
    /// is !Send on macOS), the sample Receiver (Send — safe to move to a worker),
    /// and the sample rate.
    pub fn into_parts(self) -> (cpal::Stream, std::sync::mpsc::Receiver<Vec<f32>>, u32) {
        (self._stream, self.samples, self.sample_rate)
    }
}

/// Average interleaved channels down to mono.
fn downmix(data: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 { return data.to_vec(); }
    data.chunks(channels).map(|f| f.iter().sum::<f32>() / channels as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::downmix;

    #[test]
    fn downmix_averages_stereo_to_mono() {
        assert_eq!(downmix(&[0.0, 1.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(downmix(&[0.2, 0.4], 1), vec![0.2, 0.4]);
    }
}
