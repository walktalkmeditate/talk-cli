// AudioWorklet capture processor (the shipped one — distinct from the throwaway
// spike copy in web/spike/).
//
// Runs on the audio render thread. Converts mono Float32 frames to Int16 PCM —
// the form both sherpa-onnx recognizers want — and forwards them to the main
// thread via the message port, so the conversion never blocks the UI thread.
//
// The AudioContext is created at 16 kHz, so frames already arrive at the rate
// both models want; the browser does the device→16k conversion. No resampling
// here.
//
// Loaded as a CLASSIC worklet module (NOT an ES module) via
// audioWorklet.addModule('/mic-worklet.js'). It lives in web/public/ so Vite
// serves it verbatim at the site root rather than bundling it. Keep it
// dependency-free.

/** Clamp a Float32 sample [-1, 1] to a signed 16-bit integer. */
function floatToInt16(sample) {
  const clamped = sample < -1 ? -1 : sample > 1 ? 1 : sample;
  // Asymmetric full-scale: negative range is one step larger in two's complement.
  return clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff;
}

class MicCaptureProcessor extends AudioWorkletProcessor {
  process(inputs) {
    const input = inputs[0];
    const channel = input && input[0];
    if (channel && channel.length) {
      const pcm = new Int16Array(channel.length);
      for (let i = 0; i < channel.length; i++) {
        pcm[i] = floatToInt16(channel[i]);
      }
      // Transfer the buffer so no copy is made crossing the thread boundary.
      this.port.postMessage(pcm, [pcm.buffer]);
    }
    return true;
  }
}

registerProcessor('mic-capture', MicCaptureProcessor);
