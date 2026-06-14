import { describe, it, expect } from 'vitest';
import {
  sha256Hex,
  sha256HexStream,
  sha256HexChunks,
  verify,
  hexEqual,
  modelManifest,
  modelUrl,
  MODEL_FILES,
  DEFAULT_MODEL_HOST,
  type ByteStream,
} from './integrity';

// NIST/FIPS 180-2 SHA-256 test vectors.
const SHA_ABC = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad';
const SHA_EMPTY = 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855';

function bytes(s: string): Uint8Array {
  return new TextEncoder().encode(s);
}

async function* iterChunks(chunks: Uint8Array[]): AsyncGenerator<Uint8Array> {
  for (const c of chunks) yield c;
}

describe('sha256Hex', () => {
  it('matches the known "abc" vector', async () => {
    // #given the canonical 3-byte input
    // #when hashed
    const hex = await sha256Hex(bytes('abc'));
    // #then it equals the NIST vector
    expect(hex).toBe(SHA_ABC);
  });

  it('hashes the empty input to the empty-string vector', async () => {
    expect(await sha256Hex(new Uint8Array(0))).toBe(SHA_EMPTY);
  });

  it('accepts an ArrayBuffer as well as a Uint8Array', async () => {
    const arrayBuffer = bytes('abc').slice().buffer as ArrayBuffer;
    expect(await sha256Hex(arrayBuffer)).toBe(SHA_ABC);
  });

  it('hashes a view with a nonzero byteOffset by its own bytes', async () => {
    // #given a larger buffer with "abc" embedded at offset 2
    const backing = new Uint8Array([0xff, 0xff, 0x61, 0x62, 0x63, 0xff]);
    const view = backing.subarray(2, 5); // "abc"
    // #then the digest covers only the view, not the backing pool
    expect(await sha256Hex(view)).toBe(SHA_ABC);
  });
});

describe('sha256HexStream / chunked', () => {
  it('matches the one-shot digest over the same bytes', async () => {
    const stream: ByteStream = { [Symbol.asyncIterator]: () => iterChunks([bytes('a'), bytes('b'), bytes('c')]) };
    expect(await sha256HexStream(stream)).toBe(SHA_ABC);
  });

  it('matches across chunk boundaries that split a 64-byte block', async () => {
    // #given an input longer than one SHA-256 block, split mid-block
    const big = 'x'.repeat(200);
    const oneShot = await sha256Hex(bytes(big));
    const split = [bytes(big.slice(0, 30)), bytes(big.slice(30, 70)), bytes(big.slice(70))];
    expect(await sha256HexChunks(split)).toBe(oneShot);
  });

  it('hashes the empty stream to the empty vector', async () => {
    const stream: ByteStream = { [Symbol.asyncIterator]: () => iterChunks([]) };
    expect(await sha256HexStream(stream)).toBe(SHA_EMPTY);
  });
});

describe('verify / hexEqual', () => {
  it('returns true when the digest matches the pin', async () => {
    expect(await verify(bytes('abc'), SHA_ABC)).toBe(true);
  });

  it('returns false on mismatch', async () => {
    expect(await verify(bytes('abc'), SHA_EMPTY)).toBe(false);
  });

  it('is case-insensitive on the expected hex', async () => {
    expect(await verify(bytes('abc'), SHA_ABC.toUpperCase())).toBe(true);
  });

  it('hexEqual rejects different lengths and different values', () => {
    expect(hexEqual('abcd', 'abcd')).toBe(true);
    expect(hexEqual('abcd', 'abce')).toBe(false);
    expect(hexEqual('abcd', 'abc')).toBe(false);
  });
});

describe('MODEL_FILES manifest', () => {
  it('carries the seven CLI EXTRACTED pins', () => {
    // #then there are exactly seven files (3 whisper + 4 zipformer)
    expect(MODEL_FILES).toHaveLength(7);
    const paths = MODEL_FILES.map((f) => f.path);
    expect(paths).toContain('sherpa-onnx-whisper-base.en/base.en-encoder.int8.onnx');
    expect(paths).toContain('sherpa-onnx-whisper-base.en/base.en-decoder.int8.onnx');
    expect(paths).toContain('sherpa-onnx-whisper-base.en/base.en-tokens.txt');
    expect(paths).toContain('sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/encoder-epoch-99-avg-1.int8.onnx');
    expect(paths).toContain('sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/decoder-epoch-99-avg-1.onnx');
    expect(paths).toContain('sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/joiner-epoch-99-avg-1.int8.onnx');
    expect(paths).toContain('sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/tokens.txt');
  });

  it('pins each file to a 64-hex-char SHA-256', () => {
    for (const f of MODEL_FILES) {
      expect(f.sha256).toMatch(/^[0-9a-f]{64}$/);
    }
  });

  it('carries the exact whisper-encoder pin from src/download/models.rs', () => {
    const enc = MODEL_FILES.find((f) => f.path.endsWith('base.en-encoder.int8.onnx'));
    expect(enc?.sha256).toBe('ef6b936f4c9b1d90a3b68634b60c4ed8576b26172b33c2535ec0e933c9edb823');
  });

  it('defaults the host to the pilgrim CDN and builds per-file URLs', () => {
    const m = modelManifest();
    expect(m.host).toBe(DEFAULT_MODEL_HOST);
    expect(modelUrl(m.host, 'a/b.onnx')).toBe('https://cdn.pilgrimapp.org/a/b.onnx');
  });

  it('honors an overridden host and trims a trailing slash', () => {
    const m = modelManifest('https://example.test/models/');
    expect(modelUrl(m.host, 'x.txt')).toBe('https://example.test/models/x.txt');
  });
});
