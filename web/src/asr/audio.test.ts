import { describe, it, expect } from 'vitest';
import {
  AudioCapture,
  classifyMicError,
  micStateDetail,
  type Int16Frame,
  type MicState,
} from './audio';

// The mic-permission state machine (R8) is the testable seam here; the real
// audio graph (AudioWorklet/ScriptProcessor) is validated live in a browser.
// `getUserMedia` is injected so denied / denied-suppressed / device-unavailable /
// granted each map to a distinct state without a real mic.

function domError(name: string): DOMException {
  return new DOMException(name, name);
}

describe('classifyMicError — rejection → mic state (R8)', () => {
  it('a slow NotAllowedError is a real denial (the user clicked Block)', () => {
    // #given a NotAllowedError that took longer than a human dialog interaction
    // #then it is a plain `denied` (a dialog was shown and refused)
    expect(classifyMicError(domError('NotAllowedError'), 1500)).toBe('denied');
  });

  it('an immediate NotAllowedError is denied-suppressed (no dialog appeared)', () => {
    // #given a NotAllowedError that arrived faster than a human could click
    // #then it is `denied-suppressed` (the browser auto-rejected without prompting)
    expect(classifyMicError(domError('NotAllowedError'), 5)).toBe('denied-suppressed');
  });

  it('SecurityError maps the same way as NotAllowedError (suppressed when instant)', () => {
    expect(classifyMicError(domError('SecurityError'), 5)).toBe('denied-suppressed');
    expect(classifyMicError(domError('SecurityError'), 1500)).toBe('denied');
  });

  it('NotReadableError is device-unavailable (mic in use / hardware)', () => {
    expect(classifyMicError(domError('NotReadableError'), 800)).toBe('device-unavailable');
  });

  it('NotFoundError / AbortError / OverconstrainedError are device-unavailable', () => {
    expect(classifyMicError(domError('NotFoundError'), 100)).toBe('device-unavailable');
    expect(classifyMicError(domError('AbortError'), 100)).toBe('device-unavailable');
    expect(classifyMicError(domError('OverconstrainedError'), 100)).toBe('device-unavailable');
  });

  it('an unknown rejection degrades to a retryable denial, never a hang', () => {
    expect(classifyMicError(new Error('weird'), 100)).toBe('denied');
  });
});

describe('micStateDetail — each non-granted state carries distinct guidance', () => {
  it('denied is retryable with re-enable guidance', () => {
    const d = micStateDetail('denied');
    expect(d.retryable).toBe(true);
    expect(d.message.length).toBeGreaterThan(0);
  });

  it('denied-suppressed has its own guidance distinct from denied', () => {
    expect(micStateDetail('denied-suppressed').message).not.toBe(micStateDetail('denied').message);
    expect(micStateDetail('denied-suppressed').retryable).toBe(true);
  });

  it('device-unavailable names the mic-in-use / hardware case', () => {
    const d = micStateDetail('device-unavailable');
    expect(d.retryable).toBe(true);
    expect(/use|hardware/i.test(d.message)).toBe(true);
  });

  it('pending and granted are not retryable', () => {
    expect(micStateDetail('pending').retryable).toBe(false);
    expect(micStateDetail('granted').retryable).toBe(false);
  });
});

describe('AudioCapture.start — surfaces each permission outcome via onState', () => {
  const noopFrame = (_f: Int16Frame): void => undefined;

  function captureWith(getUserMedia: (c: MediaStreamConstraints) => Promise<MediaStream>, now?: () => number) {
    const states: MicState[] = [];
    const capture = new AudioCapture({
      onFrame: noopFrame,
      onState: (d) => states.push(d.state),
      getUserMedia,
      now,
    });
    return { capture, states };
  }

  it('a slow denial emits pending → denied', async () => {
    // #given a getUserMedia that rejects slowly (a real Block click)
    let t = 0;
    const { capture, states } = captureWith(
      () => {
        t = 2000;
        return Promise.reject(domError('NotAllowedError'));
      },
      () => t,
    );
    // #when start runs
    const result = await capture.start();
    // #then the state walked pending → denied (never threw)
    expect(states).toEqual(['pending', 'denied']);
    expect(result).toBe('denied');
  });

  it('an instant denial emits pending → denied-suppressed', async () => {
    let t = 0;
    const { capture, states } = captureWith(
      () => {
        t = 10; // far under the suppressed-reject threshold
        return Promise.reject(domError('NotAllowedError'));
      },
      () => t,
    );
    const result = await capture.start();
    expect(states).toEqual(['pending', 'denied-suppressed']);
    expect(result).toBe('denied-suppressed');
  });

  it('a busy device emits pending → device-unavailable', async () => {
    const { capture, states } = captureWith(() => Promise.reject(domError('NotReadableError')));
    const result = await capture.start();
    expect(states).toEqual(['pending', 'device-unavailable']);
    expect(result).toBe('device-unavailable');
  });

  it('a grant whose audio graph fails to wire degrades to device-unavailable', async () => {
    // #given the mic is granted but no AudioContext factory is available
    // (the default factory throws under vitest — no real AudioContext)
    const fakeStream = { getTracks: () => [] } as unknown as MediaStream;
    const { capture, states } = captureWith(() => Promise.resolve(fakeStream));
    const result = await capture.start();
    // #then it does not leave the session hung — it reports device-unavailable
    expect(states).toEqual(['pending', 'device-unavailable']);
    expect(result).toBe('device-unavailable');
  });
});
