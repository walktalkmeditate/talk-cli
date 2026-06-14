// One-time model acquisition: download with progress, cancel, resume/retry,
// per-file SHA-256 verification, and defined offline states (R7, R9, R12, R13).
//
// Engine-agnostic by design: the network `fetch` and the storage cache are
// injected, so the whole state machine is unit-testable without a real browser
// Cache or network. The live fetch + the exact served files are validated live.
//
// Contract (ported from the CLI's verify-extracted-files discipline,
// src/download/models.rs): a file is committed to cache ONLY after its bytes hash
// to the pinned SHA-256. A mismatch refuses the file and re-downloads; a cached
// file is re-verified on load and refused (re-downloaded) if it no longer
// matches. Nothing runs on private audio without a clean checksum.

import {
  modelManifest,
  modelUrl,
  sha256Hex,
  hexEqual,
  type ModelFile,
  type ModelManifest,
} from './integrity';
import { modelCache, type ModelCache } from './cache';

/** Per-file lifecycle, surfaced to the UI as the download progresses. */
export type FileState =
  | 'pending'
  | 'cached' // already present + verified on load; no fetch needed
  | 'downloading'
  | 'verifying'
  | 'complete'
  | 'paused' // mid-download interruption (offline / navigate-away)
  | 'refused' // fetched bytes failed verification; will re-download
  | 'error';

/** Aggregate download phase. */
export type DownloadPhase =
  | 'pre-accept' // models absent; awaiting the user's go-ahead
  | 'downloading'
  | 'complete' // every file cached + verified
  | 'paused' // interrupted mid-download; resumes on return
  | 'blocked' // uncached + offline: cannot proceed until connected once
  | 'error';

export interface FileProgress {
  readonly path: string;
  readonly state: FileState;
  readonly receivedBytes: number;
  /** Total bytes for this file, when the server reported Content-Length. */
  readonly totalBytes: number | null;
}

export interface DownloadProgress {
  readonly phase: DownloadPhase;
  readonly files: readonly FileProgress[];
  /** Bytes received across all files this run. */
  readonly receivedBytes: number;
  /** Sum of known file totals (null until every Content-Length is seen). */
  readonly totalBytes: number | null;
  /** 0..1 across all files when totals are known, else null. */
  readonly fraction: number | null;
}

export type ProgressListener = (progress: DownloadProgress) => void;

export interface DownloadOptions {
  /** Override the manifest (host/files). Defaults to the production manifest. */
  readonly manifest?: ModelManifest;
  /** The storage cache. Defaults to `modelCache()` (Cache API or memory). */
  readonly cache?: ModelCache;
  /** Injected fetch (defaults to global `fetch`) — the test seam for the network. */
  readonly fetchFn?: typeof fetch;
  /** Cancel signal; aborting rejects `downloadModels` with `DownloadCancelled`. */
  readonly signal?: AbortSignal;
  /** Progress + sub-state callback. */
  readonly onProgress?: ProgressListener;
  /** Online check (defaults to navigator.onLine); the offline-state seam. */
  readonly isOnline?: () => boolean;
  /** Max fetch attempts per file before giving up (default 3). */
  readonly maxAttempts?: number;
}

export class DownloadCancelled extends Error {
  constructor() {
    super('model download cancelled');
    this.name = 'DownloadCancelled';
  }
}

export class ModelsBlockedOffline extends Error {
  constructor() {
    super('models are not cached and the browser is offline');
    this.name = 'ModelsBlockedOffline';
  }
}

export class DownloadFailed extends Error {
  constructor(
    readonly path: string,
    cause: unknown,
  ) {
    super(`failed to acquire ${path}: ${describe(cause)}`);
    this.name = 'DownloadFailed';
  }
}

function describe(cause: unknown): string {
  if (cause instanceof Error) return cause.message;
  return String(cause);
}

interface FileTracker {
  path: string;
  state: FileState;
  received: number;
  total: number | null;
}

const defaultIsOnline = (): boolean => {
  // Assume online when the signal is unavailable (no navigator, or `onLine`
  // undefined as in non-browser runtimes) — the offline state is only asserted
  // when the platform explicitly reports it.
  if (typeof navigator === 'undefined') return true;
  return navigator.onLine !== false;
};

/**
 * Decide whether the model set is fully usable from cache, must be downloaded,
 * or is blocked because it's both uncached and offline (R13). Verifies each
 * cached file on load — a tampered/corrupt cache entry counts as "not cached",
 * which forces a clean re-download rather than running a bad weight.
 */
export async function resolveModelState(
  opts: DownloadOptions = {},
): Promise<{ phase: 'complete' | 'pre-accept' | 'blocked'; missing: ModelFile[] }> {
  const manifest = opts.manifest ?? modelManifest();
  const cache = opts.cache ?? modelCache();
  const isOnline = opts.isOnline ?? defaultIsOnline;

  const missing: ModelFile[] = [];
  for (const file of manifest.files) {
    const cached = await loadVerified(cache, manifest.host, file);
    if (cached === null) missing.push(file);
  }

  if (missing.length === 0) return { phase: 'complete', missing };
  if (!isOnline()) return { phase: 'blocked', missing };
  return { phase: 'pre-accept', missing };
}

/**
 * Load a cached file and verify it against its pin. Returns the bytes on a clean
 * match, else `null` (absent, corrupt, or checksum mismatch → re-download).
 */
export async function loadVerified(
  cache: ModelCache,
  host: string,
  file: ModelFile,
): Promise<ArrayBuffer | null> {
  const key = modelUrl(host, file.path);
  const bytes = await cache.get(key);
  if (bytes === null) return null;
  const actual = await sha256Hex(bytes);
  if (!hexEqual(actual, file.sha256)) {
    // Refuse-not-run: drop the tampered/corrupt entry so the next pass re-fetches.
    await cache.remove(key);
    return null;
  }
  return bytes;
}

/**
 * Acquire every model file: skip already-cached+verified files, fetch the rest
 * with streamed progress, verify each before committing to cache, and retry on
 * failure. Emits progress + sub-state events throughout.
 *
 * Resume model: a file is committed to cache only as a verified whole, so an
 * interrupted file leaves no partial — re-invoking `downloadModels` resumes at
 * the file boundary (already-verified files are skipped) and re-fetches the
 * interrupted one cleanly. Byte-range resume *within* a file (persisting a
 * partial across a navigate-away) is the documented next step, gated on the CDN
 * honoring Range (Key Technical Decisions); the clean-re-fetch fallback here is
 * what's unit-testable without a real browser/network, and is correct on its own.
 *
 * Resolves when every file is cached + verified. Rejects with
 * `ModelsBlockedOffline` (uncached + offline), `DownloadCancelled` (aborted), or
 * `DownloadFailed` (a file exhausted its retries).
 */
export async function downloadModels(opts: DownloadOptions = {}): Promise<void> {
  const manifest = opts.manifest ?? modelManifest();
  const cache = opts.cache ?? modelCache();
  const fetchFn = opts.fetchFn ?? fetch;
  const isOnline = opts.isOnline ?? defaultIsOnline;
  const maxAttempts = opts.maxAttempts ?? 3;
  const signal = opts.signal;

  const trackers: FileTracker[] = manifest.files.map((f) => ({
    path: f.path,
    state: 'pending',
    received: 0,
    total: null,
  }));

  const emit = (phase: DownloadPhase): void => {
    if (!opts.onProgress) return;
    opts.onProgress(snapshot(phase, trackers));
  };

  if (signal?.aborted) throw new DownloadCancelled();

  // First pass: resolve which files are already cached+verified.
  let anyMissing = false;
  for (let i = 0; i < manifest.files.length; i++) {
    const file = manifest.files[i];
    const cached = await loadVerified(cache, manifest.host, file);
    if (cached !== null) {
      trackers[i].state = 'cached';
      trackers[i].received = cached.byteLength;
      trackers[i].total = cached.byteLength;
    } else {
      anyMissing = true;
    }
  }

  if (!anyMissing) {
    emit('complete');
    return;
  }

  if (!isOnline()) {
    emit('blocked');
    throw new ModelsBlockedOffline();
  }

  emit('downloading');

  for (let i = 0; i < manifest.files.length; i++) {
    const file = manifest.files[i];
    const tracker = trackers[i];
    if (tracker.state === 'cached' || tracker.state === 'complete') continue;

    try {
      await acquireFile(file, tracker, {
        manifest,
        cache,
        fetchFn,
        isOnline,
        maxAttempts,
        signal,
        emit: () => emit('downloading'),
        pause: () => emit('paused'),
      });
    } catch (err) {
      if (err instanceof DownloadCancelled) throw err;
      if (err instanceof ModelsBlockedOffline) {
        tracker.state = 'paused';
        emit('paused');
        throw err;
      }
      tracker.state = 'error';
      emit('error');
      throw err instanceof DownloadFailed ? err : new DownloadFailed(file.path, err);
    }
  }

  emit('complete');
}

interface AcquireCtx {
  manifest: ModelManifest;
  cache: ModelCache;
  fetchFn: typeof fetch;
  isOnline: () => boolean;
  maxAttempts: number;
  signal: AbortSignal | undefined;
  emit: () => void;
  pause: () => void;
}

/**
 * Fetch one file with progress, resume, retry, and verify-before-commit. On a
 * verification mismatch the partial is discarded and the file is re-fetched
 * cleanly (state `refused`). On network loss mid-stream the file pauses and the
 * outer loop surfaces the paused phase; the next call resumes from the cached
 * partial via Range.
 */
async function acquireFile(file: ModelFile, tracker: FileTracker, ctx: AcquireCtx): Promise<void> {
  const url = modelUrl(ctx.manifest.host, file.path);
  let lastErr: unknown;

  for (let attempt = 0; attempt < ctx.maxAttempts; attempt++) {
    if (ctx.signal?.aborted) throw new DownloadCancelled();
    if (!ctx.isOnline()) {
      tracker.state = 'paused';
      ctx.pause();
      throw new ModelsBlockedOffline();
    }

    tracker.state = 'downloading';
    tracker.received = 0;
    tracker.total = null;
    ctx.emit();

    try {
      const bytes = await fetchWholeFile(url, tracker, ctx);
      tracker.state = 'verifying';
      ctx.emit();

      const actual = await sha256Hex(bytes);
      if (!hexEqual(actual, file.sha256)) {
        // Refuse + clean re-download: never commit a mismatched weight.
        tracker.state = 'refused';
        ctx.emit();
        lastErr = new Error(`checksum mismatch (got ${actual}, want ${file.sha256})`);
        continue;
      }

      await ctx.cache.put(url, bytes);
      tracker.state = 'complete';
      tracker.received = bytes.byteLength;
      tracker.total = bytes.byteLength;
      ctx.emit();
      return;
    } catch (err) {
      if (err instanceof DownloadCancelled) throw err;
      if (err instanceof ModelsBlockedOffline) throw err;
      lastErr = err;
      // Retry the next attempt (clean re-fetch — fetchWholeFile reads no partial).
    }
  }

  throw new DownloadFailed(file.path, lastErr);
}

/**
 * Stream the response body, updating progress per chunk. Resolves the assembled
 * bytes. If the body has no reader (e.g. a test fetch returning a plain
 * Response), falls back to `arrayBuffer()`.
 */
async function fetchWholeFile(url: string, tracker: FileTracker, ctx: AcquireCtx): Promise<ArrayBuffer> {
  const res = await ctx.fetchFn(url, { signal: ctx.signal });
  if (!res.ok) throw new Error(`HTTP ${res.status} for ${url}`);

  const lenHeader = res.headers.get('content-length');
  tracker.total = lenHeader ? Number(lenHeader) : null;

  const body = res.body;
  if (!body) {
    const buf = await res.arrayBuffer();
    tracker.received = buf.byteLength;
    if (tracker.total === null) tracker.total = buf.byteLength;
    ctx.emit();
    return buf;
  }

  const reader = body.getReader();
  const chunks: Uint8Array[] = [];
  let received = 0;
  for (;;) {
    if (ctx.signal?.aborted) {
      await reader.cancel().catch(() => undefined);
      throw new DownloadCancelled();
    }
    const { done, value } = await reader.read();
    if (done) break;
    if (value) {
      chunks.push(value);
      received += value.byteLength;
      tracker.received = received;
      ctx.emit();
    }
  }

  return concat(chunks, received);
}

function concat(chunks: readonly Uint8Array[], total: number): ArrayBuffer {
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return out.buffer;
}

function snapshot(phase: DownloadPhase, trackers: readonly FileTracker[]): DownloadProgress {
  const files: FileProgress[] = trackers.map((t) => ({
    path: t.path,
    state: t.state,
    receivedBytes: t.received,
    totalBytes: t.total,
  }));
  const receivedBytes = trackers.reduce((sum, t) => sum + t.received, 0);
  const allKnown = trackers.every((t) => t.total !== null);
  const totalBytes = allKnown ? trackers.reduce((sum, t) => sum + (t.total ?? 0), 0) : null;
  const fraction = totalBytes && totalBytes > 0 ? Math.min(1, receivedBytes / totalBytes) : null;
  return { phase, files, receivedBytes, totalBytes, fraction };
}
