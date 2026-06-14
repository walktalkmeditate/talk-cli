// THROWAWAY U1 spike — AudioWorklet capture processor.
//
// Runs on the audio render thread. Forwards mono Float32 frames to the main
// thread via the message port. The AudioContext is created at 16 kHz, so the
// frames are already at the rate both sherpa-onnx models want — no resampling
// here (the Rust path resamples; the browser gives us 16 kHz directly when the
// context sampleRate is set, with the browser doing the device→16k conversion).
//
// Loaded as a classic worklet module (NOT an ES module) via
// audioWorklet.addModule(). Keep it dependency-free.

class MicCaptureProcessor extends AudioWorkletProcessor {
  process(inputs) {
    const input = inputs[0];
    if (input && input[0] && input[0].length) {
      // Copy: the input buffer is reused by the engine after process() returns.
      this.port.postMessage(input[0].slice(0));
    }
    return true;
  }
}

registerProcessor("mic-capture", MicCaptureProcessor);
