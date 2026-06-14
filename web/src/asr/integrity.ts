// Per-file SHA-256 integrity for the on-device model files (R7).
//
// The talk CLI verifies the SEVEN extracted files the session actually loads —
// the archive hash alone doesn't cover post-extraction tampering. The web port
// serves those individual extracted files directly from the CDN, so the same
// verify-extracted-files contract applies here: every file is hashed after fetch
// AND on every cache load, and a mismatch refuses the model rather than running
// it on private audio.

const SHA_256 = 'SHA-256';

/** A single model file the WASM module reads, pinned by its SHA-256. */
export interface ModelFile {
  /** Path relative to the model base host, e.g. `sherpa-onnx-whisper-base.en/base.en-tokens.txt`. */
  readonly path: string;
  /** Lowercase hex SHA-256 of the file's bytes. */
  readonly sha256: string;
}

/**
 * The authoritative per-file pin set, ported verbatim from the CLI manifest in
 * `src/download/models.rs` (the `EXTRACTED` constant): Whisper base.en (int8
 * encoder/decoder + tokens) and the streaming Zipformer-20M transducer
 * (int8 encoder/joiner, fp32 decoder, tokens). Combined ≈ 327 MB.
 *
 * IMPORTANT: these hashes are the CLI's extracted-file pins, corroborated against
 * the k2-fsa release and the Hugging Face mirror. They are the upstream
 * corroborator. At hosting time they MUST be re-pinned against the bytes actually
 * served from the WASM artifacts on `cdn.pilgrimapp.org` — pin upstream BEFORE
 * upload, then assert the served R2 copy matches that pin (Key Technical
 * Decisions, "Model integrity"). Until that live re-pin, treat these as the
 * upstream truth the served files are checked against.
 */
export const MODEL_FILES: readonly ModelFile[] = [
  {
    path: 'sherpa-onnx-whisper-base.en/base.en-encoder.int8.onnx',
    sha256: 'ef6b936f4c9b1d90a3b68634b60c4ed8576b26172b33c2535ec0e933c9edb823',
  },
  {
    path: 'sherpa-onnx-whisper-base.en/base.en-decoder.int8.onnx',
    sha256: 'f7162ad6db2dbef16cfaeaa7f945b9d7dd9c1b8d472f6aca82f2273d185e4d41',
  },
  {
    path: 'sherpa-onnx-whisper-base.en/base.en-tokens.txt',
    sha256: '306cd27f03c1a714eca7108e03d66b7dc042abe8c258b44c199a7ed9838dd930',
  },
  {
    path: 'sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/encoder-epoch-99-avg-1.int8.onnx',
    sha256: '3810755ce7c3ab26b42a8bcf39d191308fa27fb0f53358823ba46141d03b7eb3',
  },
  {
    path: 'sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/decoder-epoch-99-avg-1.onnx',
    sha256: '45a7f940ecfb53d89fa270ad11b88b961e53a317203eb24b1c8e95ed208b0f30',
  },
  {
    path: 'sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/joiner-epoch-99-avg-1.int8.onnx',
    sha256: 'e085d73b593cf9b0707f370dbd656d58327d3fe36d80d849202ef81df02cb01e',
  },
  {
    path: 'sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/tokens.txt',
    sha256: '49e3c2646595fd907228b3c6787069658f67b17377c60aeb8619c4551b2316fb',
  },
];

/** Default model host — the pilgrim CDN R2 bucket, one CSP-allowed origin. */
export const DEFAULT_MODEL_HOST = 'https://cdn.pilgrimapp.org';

/** The manifest a download/verify pass operates over: the files + their host. */
export interface ModelManifest {
  readonly host: string;
  readonly files: readonly ModelFile[];
}

/**
 * Build the manifest. Host is overridable (preview deploys, local hosting, tests)
 * but defaults to the production CDN.
 */
export function modelManifest(host: string = DEFAULT_MODEL_HOST): ModelManifest {
  return { host, files: MODEL_FILES };
}

/** The full absolute URL a file is fetched from, given a manifest host. */
export function modelUrl(host: string, path: string): string {
  const base = host.endsWith('/') ? host.slice(0, -1) : host;
  return `${base}/${path}`;
}

function toUint8(bytes: ArrayBuffer | Uint8Array): Uint8Array {
  return bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
}

function toHex(digest: ArrayBuffer): string {
  const view = new Uint8Array(digest);
  let hex = '';
  for (const byte of view) hex += byte.toString(16).padStart(2, '0');
  return hex;
}

/**
 * SHA-256 of a whole buffer, hex-encoded, via Web Crypto. Use this for the small
 * model files (tokens, decoders). For the large encoders, prefer
 * `sha256HexStream` so the whole file need not sit decoded in one digest call.
 */
export async function sha256Hex(bytes: ArrayBuffer | Uint8Array): Promise<string> {
  const src = toUint8(bytes);
  // Copy into a fresh ArrayBuffer so a Buffer/typed-array view with a nonzero
  // byteOffset (e.g. node's Buffer) hashes its own bytes, not the pool's.
  const copy = src.slice();
  const digest = await crypto.subtle.digest(SHA_256, copy);
  return toHex(digest);
}

/** A source of bytes that can be hashed incrementally without full buffering. */
export interface ByteStream {
  /** Async-iterate chunks of the file's bytes in order. */
  [Symbol.asyncIterator](): AsyncIterator<Uint8Array>;
}

/**
 * Streaming/chunked SHA-256 for large files. `crypto.subtle.digest` is one-shot
 * (no incremental update), so chunk hashing is done with an incremental
 * implementation when streaming a ~199 MB encoder that should not be held in one
 * buffer. Returns the same hex digest as `sha256Hex` over the concatenation.
 *
 * U1 determines whether the one-shot path suffices on mobile; this is the seam
 * for the chunked path if it doesn't.
 */
export async function sha256HexStream(stream: ByteStream): Promise<string> {
  const hasher = new Sha256();
  for await (const chunk of stream) {
    hasher.update(toUint8(chunk));
  }
  return hasher.hexDigest();
}

/** Hash an array of chunks (test/convenience wrapper over `sha256HexStream`). */
export async function sha256HexChunks(chunks: readonly (ArrayBuffer | Uint8Array)[]): Promise<string> {
  const hasher = new Sha256();
  for (const chunk of chunks) hasher.update(toUint8(chunk));
  return hasher.hexDigest();
}

/**
 * Constant-time-ish hex comparison. These are public hashes (no secrecy needed),
 * but a length-independent compare avoids early-exit surprises and is cheap.
 */
export function hexEqual(a: string, b: string): boolean {
  const x = a.toLowerCase();
  const y = b.toLowerCase();
  if (x.length !== y.length) return false;
  let diff = 0;
  for (let i = 0; i < x.length; i++) diff |= x.charCodeAt(i) ^ y.charCodeAt(i);
  return diff === 0;
}

/** Verify bytes against an expected hex digest. */
export async function verify(bytes: ArrayBuffer | Uint8Array, expectedHex: string): Promise<boolean> {
  const actual = await sha256Hex(bytes);
  return hexEqual(actual, expectedHex);
}

/**
 * Pure-TS incremental SHA-256. Used only by the streaming path; the whole-buffer
 * path uses the native Web Crypto one-shot digest. This exists because
 * `crypto.subtle.digest` has no update()/streaming API, and U1 may require
 * chunked hashing for the large encoder on memory-constrained mobile.
 */
class Sha256 {
  private static readonly K = new Uint32Array([
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
  ]);

  private h = new Uint32Array([
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
  ]);

  private readonly block = new Uint8Array(64);
  private blockLen = 0;
  private totalLen = 0;
  private readonly w = new Uint32Array(64);

  update(bytes: Uint8Array): void {
    this.totalLen += bytes.length;
    let offset = 0;
    if (this.blockLen > 0) {
      const need = 64 - this.blockLen;
      const take = Math.min(need, bytes.length);
      this.block.set(bytes.subarray(0, take), this.blockLen);
      this.blockLen += take;
      offset = take;
      if (this.blockLen === 64) {
        this.process(this.block, 0);
        this.blockLen = 0;
      }
    }
    while (offset + 64 <= bytes.length) {
      this.process(bytes, offset);
      offset += 64;
    }
    if (offset < bytes.length) {
      const rest = bytes.subarray(offset);
      this.block.set(rest, 0);
      this.blockLen = rest.length;
    }
  }

  hexDigest(): string {
    const bitLen = this.totalLen * 8;
    const pad = new Uint8Array(this.blockLen < 56 ? 64 : 128);
    pad.set(this.block.subarray(0, this.blockLen), 0);
    pad[this.blockLen] = 0x80;
    const lenOffset = pad.length - 8;
    // 64-bit big-endian length; high 32 bits cover files > 4 GiB (we never reach
    // them, but the math stays correct).
    const hi = Math.floor(bitLen / 0x100000000);
    const lo = bitLen >>> 0;
    pad[lenOffset] = (hi >>> 24) & 0xff;
    pad[lenOffset + 1] = (hi >>> 16) & 0xff;
    pad[lenOffset + 2] = (hi >>> 8) & 0xff;
    pad[lenOffset + 3] = hi & 0xff;
    pad[lenOffset + 4] = (lo >>> 24) & 0xff;
    pad[lenOffset + 5] = (lo >>> 16) & 0xff;
    pad[lenOffset + 6] = (lo >>> 8) & 0xff;
    pad[lenOffset + 7] = lo & 0xff;
    for (let off = 0; off < pad.length; off += 64) this.process(pad, off);

    let hex = '';
    for (const word of this.h) hex += (word >>> 0).toString(16).padStart(8, '0');
    return hex;
  }

  private process(data: Uint8Array, offset: number): void {
    const w = this.w;
    for (let i = 0; i < 16; i++) {
      const j = offset + i * 4;
      w[i] = (data[j] << 24) | (data[j + 1] << 16) | (data[j + 2] << 8) | data[j + 3];
    }
    for (let i = 16; i < 64; i++) {
      const a = w[i - 15];
      const b = w[i - 2];
      const s0 = rotr(a, 7) ^ rotr(a, 18) ^ (a >>> 3);
      const s1 = rotr(b, 17) ^ rotr(b, 19) ^ (b >>> 10);
      w[i] = (w[i - 16] + s0 + w[i - 7] + s1) | 0;
    }

    let [a, b, c, d, e, f, g, h] = this.h;
    for (let i = 0; i < 64; i++) {
      const s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
      const ch = (e & f) ^ (~e & g);
      const t1 = (h + s1 + ch + Sha256.K[i] + w[i]) | 0;
      const s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const t2 = (s0 + maj) | 0;
      h = g;
      g = f;
      f = e;
      e = (d + t1) | 0;
      d = c;
      c = b;
      b = a;
      a = (t1 + t2) | 0;
    }

    this.h[0] = (this.h[0] + a) | 0;
    this.h[1] = (this.h[1] + b) | 0;
    this.h[2] = (this.h[2] + c) | 0;
    this.h[3] = (this.h[3] + d) | 0;
    this.h[4] = (this.h[4] + e) | 0;
    this.h[5] = (this.h[5] + f) | 0;
    this.h[6] = (this.h[6] + g) | 0;
    this.h[7] = (this.h[7] + h) | 0;
  }
}

function rotr(x: number, n: number): number {
  return (x >>> n) | (x << (32 - n));
}
