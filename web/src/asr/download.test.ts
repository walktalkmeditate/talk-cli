import { describe, it, expect, vi } from 'vitest';
import {
  downloadModels,
  resolveModelState,
  loadVerified,
  DownloadCancelled,
  ModelsBlockedOffline,
  DownloadFailed,
  CacheWriteFailed,
  DownloadStalled,
  type DownloadProgress,
} from './download';
import { sha256Hex, modelUrl, type ModelManifest, type ModelFile } from './integrity';
import {
  BackedModelCache,
  MemoryBackend,
  type CacheBackend,
  type ModelCache,
} from './cache';

const HOST = 'https://cdn.test';

/** A no-op sleep so retry backoff costs no wall-clock time in tests. */
const noSleep = (): Promise<void> => Promise.resolve();

/** A DOMException-shaped quota error (what a real cache write throws when full). */
function quotaError(): Error {
  const e = new Error('QuotaExceededError');
  e.name = 'QuotaExceededError';
  (e as { code?: number }).code = 22;
  return e;
}

function bytes(s: string): Uint8Array {
  return new TextEncoder().encode(s);
}

/** A standalone ArrayBuffer (not a view into a pool) for the given string. */
function ab(s: string): ArrayBuffer {
  const u = bytes(s);
  return u.slice().buffer as ArrayBuffer;
}

/** A streamed Response body for the given bytes (with Content-Length). */
function bodyResponse(b: Uint8Array, withLength = true): Response {
  const headers = new Headers();
  if (withLength) headers.set('content-length', String(b.byteLength));
  return new Response(b.slice().buffer as ArrayBuffer, { status: 200, headers });
}

/** Build a manifest of files whose pins are the real SHA-256 of given contents. */
async function manifestOf(entries: Record<string, string>): Promise<{
  manifest: ManifestWithContents;
}> {
  const files: ModelFile[] = [];
  const contents = new Map<string, Uint8Array>();
  for (const [path, body] of Object.entries(entries)) {
    const b = bytes(body);
    files.push({ path, sha256: await sha256Hex(b) });
    contents.set(path, b);
  }
  return { manifest: { host: HOST, files, contents } };
}

interface ManifestWithContents extends ModelManifest {
  contents: Map<string, Uint8Array>;
}

/** A fetch mock that serves a file's real bytes by URL, with a streamed body. */
function serveFetch(manifest: ManifestWithContents, opts: { contentLength?: boolean } = {}): typeof fetch {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input.toString();
    for (const [path, body] of manifest.contents) {
      if (url === modelUrl(manifest.host, path)) {
        return bodyResponse(body, opts.contentLength !== false);
      }
    }
    return new Response(null, { status: 404 });
  }) as unknown as typeof fetch;
}

function freshCache(): ModelCache {
  return new BackedModelCache(new MemoryBackend());
}

describe('downloadModels — happy path', () => {
  it('fetches, verifies, and caches each file', async () => {
    // #given a two-file manifest and an empty cache
    const { manifest } = await manifestOf({ 'a.onnx': 'alpha', 'b.txt': 'beta' });
    const cache = freshCache();
    const phases: string[] = [];

    // #when downloaded
    await downloadModels({
      manifest,
      cache,
      fetchFn: serveFetch(manifest),
      onProgress: (p) => phases.push(p.phase),
    });

    // #then both files are cached and verify on load
    expect(await loadVerified(cache, manifest.host, manifest.files[0])).not.toBeNull();
    expect(await loadVerified(cache, manifest.host, manifest.files[1])).not.toBeNull();
    expect(phases).toContain('downloading');
    expect(phases.at(-1)).toBe('complete');
  });

  it('reports byte + fraction progress as chunks stream', async () => {
    const { manifest } = await manifestOf({ 'a.onnx': 'alpha' });
    const cache = freshCache();
    let last: DownloadProgress | null = null;
    await downloadModels({
      manifest,
      cache,
      fetchFn: serveFetch(manifest),
      onProgress: (p) => {
        last = p;
      },
    });
    expect(last!.receivedBytes).toBe(bytes('alpha').byteLength);
    expect(last!.fraction).toBe(1);
  });

  it('performs zero fetches when every file is already cached (AE5)', async () => {
    const { manifest } = await manifestOf({ 'a.onnx': 'alpha' });
    const cache = freshCache();
    // pre-seed the cache with verified bytes
    await cache.put(modelUrl(manifest.host, 'a.onnx'), ab('alpha'));
    const fetchFn = serveFetch(manifest);

    let lastPhase = '';
    await downloadModels({ manifest, cache, fetchFn, onProgress: (p) => (lastPhase = p.phase) });

    expect(fetchFn).not.toHaveBeenCalled();
    expect(lastPhase).toBe('complete');
  });
});

describe('downloadModels — integrity refuse + retry (AE7)', () => {
  it('refuses a hash mismatch, re-downloads, never caches the bad bytes', async () => {
    // #given a manifest whose pin will NOT match the served bytes on the 1st try
    const { manifest } = await manifestOf({ 'a.onnx': 'good' });
    const cache = freshCache();

    let call = 0;
    const url = modelUrl(manifest.host, 'a.onnx');
    const fetchFn = vi.fn(async () => {
      call += 1;
      // first response is tampered; second is the real bytes
      return bodyResponse(call === 1 ? bytes('TAMPERED') : bytes('good'));
    }) as unknown as typeof fetch;

    const states: string[] = [];
    await downloadModels({
      manifest,
      cache,
      fetchFn,
      sleep: noSleep, // skip the retry backoff so the test stays fast
      onProgress: (p) => p.files.forEach((f) => states.push(f.state)),
    });

    // #then the bad bytes were refused, the good ones cached
    expect(states).toContain('refused');
    expect(call).toBe(2);
    const cached = await cache.get(url);
    expect(new TextDecoder().decode(cached!)).toBe('good');
  });

  it('fails after exhausting retries on persistent mismatch', async () => {
    const { manifest } = await manifestOf({ 'a.onnx': 'good' });
    const cache = freshCache();
    const fetchFn = vi.fn(async () => bodyResponse(bytes('always wrong'))) as unknown as typeof fetch;

    await expect(
      downloadModels({ manifest, cache, fetchFn, maxAttempts: 2, sleep: noSleep }),
    ).rejects.toBeInstanceOf(DownloadFailed);
    // nothing bad got committed
    expect(await cache.get(modelUrl(manifest.host, 'a.onnx'))).toBeNull();
  });
});

describe('offline states (R13, AE8)', () => {
  it('resolveModelState: cached + offline → complete', async () => {
    const { manifest } = await manifestOf({ 'a.onnx': 'alpha' });
    const cache = freshCache();
    await cache.put(modelUrl(manifest.host, 'a.onnx'), ab('alpha'));
    const state = await resolveModelState({ manifest, cache, isOnline: () => false });
    expect(state.phase).toBe('complete');
  });

  it('resolveModelState: uncached + offline → blocked', async () => {
    const { manifest } = await manifestOf({ 'a.onnx': 'alpha' });
    const state = await resolveModelState({ manifest, cache: freshCache(), isOnline: () => false });
    expect(state.phase).toBe('blocked');
    expect(state.missing).toHaveLength(1);
  });

  it('resolveModelState: uncached + online → pre-accept', async () => {
    const { manifest } = await manifestOf({ 'a.onnx': 'alpha' });
    const state = await resolveModelState({ manifest, cache: freshCache(), isOnline: () => true });
    expect(state.phase).toBe('pre-accept');
  });

  it('downloadModels: uncached + offline rejects ModelsBlockedOffline + emits blocked', async () => {
    const { manifest } = await manifestOf({ 'a.onnx': 'alpha' });
    let blocked = false;
    await expect(
      downloadModels({
        manifest,
        cache: freshCache(),
        fetchFn: serveFetch(manifest),
        isOnline: () => false,
        onProgress: (p) => {
          if (p.phase === 'blocked') blocked = true;
        },
      }),
    ).rejects.toBeInstanceOf(ModelsBlockedOffline);
    expect(blocked).toBe(true);
  });
});

describe('mid-download drop → pause/resume', () => {
  it('pauses when the network drops mid-run, then resumes on a second call', async () => {
    // #given a two-file manifest; the network goes offline after the first file
    const { manifest } = await manifestOf({ 'a.onnx': 'alpha', 'b.onnx': 'beta' });
    const cache = freshCache();
    const fetchFn = serveFetch(manifest);

    let online = true;
    let fileACount = 0;
    const isOnline = (): boolean => online;
    const wrappedFetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString();
      if (url === modelUrl(manifest.host, 'a.onnx')) {
        fileACount += 1;
        const res = await fetchFn(input, init);
        online = false; // drop the network right after file A is served
        return res;
      }
      return fetchFn(input, init);
    }) as unknown as typeof fetch;

    // #when downloaded while the net drops mid-run
    let sawPaused = false;
    await expect(
      downloadModels({
        manifest,
        cache,
        fetchFn: wrappedFetch,
        isOnline,
        onProgress: (p) => {
          if (p.phase === 'paused') sawPaused = true;
        },
      }),
    ).rejects.toBeInstanceOf(ModelsBlockedOffline);
    expect(sawPaused).toBe(true);

    // file A committed; file B not
    expect(await cache.get(modelUrl(manifest.host, 'a.onnx'))).not.toBeNull();
    expect(await cache.get(modelUrl(manifest.host, 'b.onnx'))).toBeNull();

    // #when the network is back and we resume (second call)
    online = true;
    await downloadModels({ manifest, cache, fetchFn, isOnline });

    // #then file A is NOT re-fetched (already verified), file B completes
    expect(fileACount).toBe(1);
    expect(await cache.get(modelUrl(manifest.host, 'b.onnx'))).not.toBeNull();
  });
});

describe('cancellation', () => {
  it('rejects with DownloadCancelled when aborted before start', async () => {
    const { manifest } = await manifestOf({ 'a.onnx': 'alpha' });
    const controller = new AbortController();
    controller.abort();
    await expect(
      downloadModels({ manifest, cache: freshCache(), fetchFn: serveFetch(manifest), signal: controller.signal }),
    ).rejects.toBeInstanceOf(DownloadCancelled);
  });
});

describe('retry backoff (exponential + jitter)', () => {
  it('waits between retries with an exponential, jittered backoff', async () => {
    // #given a manifest whose first two fetches are tampered, the third is good
    const { manifest } = await manifestOf({ 'a.onnx': 'good' });
    const cache = freshCache();
    let call = 0;
    const fetchFn = vi.fn(async () => {
      call += 1;
      return bodyResponse(call < 3 ? bytes('TAMPERED') : bytes('good'));
    }) as unknown as typeof fetch;

    // #when downloaded with an injected sleep that records the waited durations
    const waits: number[] = [];
    await downloadModels({
      manifest,
      cache,
      fetchFn,
      maxAttempts: 3,
      sleep: (ms) => {
        waits.push(ms);
        return Promise.resolve();
      },
      random: () => 0.5, // deterministic jitter (0.5 * 200 = 100ms)
    });

    // #then a backoff was waited before attempt 1 and attempt 2 (not before the
    // first try), and it grew exponentially: 500*2^0+100, then 500*2^1+100.
    expect(waits).toEqual([600, 1100]);
    expect(call).toBe(3);
  });
});

describe('cache-write failure is distinct from a network failure (R12/R23)', () => {
  it('a put failure rejects CacheWriteFailed and does NOT re-fetch the file', async () => {
    // #given a cache whose backend write always fails (e.g. quota exhausted)
    const { manifest } = await manifestOf({ 'a.onnx': 'good' });
    const failingBackend: CacheBackend = {
      read: async () => null,
      write: async () => {
        throw quotaError();
      },
      delete: async () => undefined,
      keys: async () => [],
      clear: async () => undefined,
    };
    const cache = new BackedModelCache(failingBackend);
    let fetchCount = 0;
    const fetchFn = vi.fn(async () => {
      fetchCount += 1;
      return bodyResponse(bytes('good'));
    }) as unknown as typeof fetch;

    // #when downloaded
    const promise = downloadModels({ manifest, cache, fetchFn, maxAttempts: 3, sleep: noSleep });

    // #then it surfaces as CacheWriteFailed (storage), NOT DownloadFailed (network)
    await expect(promise).rejects.toBeInstanceOf(CacheWriteFailed);
    // #and the verified bytes were fetched ONCE — a put failure must not re-fetch
    expect(fetchCount).toBe(1);
  });
});

describe('per-file stall timeout (R7)', () => {
  it('aborts a fetch whose body never delivers a chunk → DownloadStalled, then gives up', async () => {
    // #given a fetch whose body reader never resolves (a hung connection)
    const { manifest } = await manifestOf({ 'a.onnx': 'good' });
    const cache = freshCache();
    const stallingFetch = vi.fn((_input: RequestInfo | URL, init?: RequestInit) => {
      const body = new ReadableStream<Uint8Array>({
        start(controller) {
          // never enqueue/close — only an abort can end the read
          const signal = init?.signal;
          signal?.addEventListener('abort', () => controller.error(new DOMException('aborted', 'AbortError')));
        },
      });
      return Promise.resolve(new Response(body, { status: 200 }));
    }) as unknown as typeof fetch;

    // #when downloaded with a tiny stall timeout (and no retry backoff)
    await expect(
      downloadModels({
        manifest,
        cache,
        fetchFn: stallingFetch,
        maxAttempts: 2,
        stallTimeoutMs: 10,
        sleep: noSleep,
      }),
    ).rejects.toBeInstanceOf(DownloadFailed); // wraps the final DownloadStalled

    // #then it tried (and stalled) each attempt — never committed bad bytes
    expect(stallingFetch).toHaveBeenCalledTimes(2);
    expect(await cache.get(modelUrl(manifest.host, 'a.onnx'))).toBeNull();
  });

  it('surfaces a DownloadStalled cause through the DownloadFailed message', async () => {
    const { manifest } = await manifestOf({ 'a.onnx': 'good' });
    const stallingFetch = vi.fn((_input: RequestInfo | URL, init?: RequestInit) => {
      const body = new ReadableStream<Uint8Array>({
        start(controller) {
          init?.signal?.addEventListener('abort', () =>
            controller.error(new DOMException('aborted', 'AbortError')),
          );
        },
      });
      return Promise.resolve(new Response(body, { status: 200 }));
    }) as unknown as typeof fetch;

    const err = await downloadModels({
      manifest,
      cache: freshCache(),
      fetchFn: stallingFetch,
      maxAttempts: 1,
      stallTimeoutMs: 10,
      sleep: noSleep,
    }).catch((e: unknown) => e);

    expect(err).toBeInstanceOf(DownloadFailed);
    expect((err as DownloadFailed).message).toContain('stalled');
    // sanity: the stall error type itself is exported + named
    expect(new DownloadStalled('x').name).toBe('DownloadStalled');
  });
});

describe('loadVerified — refuse a tampered cache (AE7 load side)', () => {
  it('returns null and evicts a cached file whose checksum no longer matches', async () => {
    const { manifest } = await manifestOf({ 'a.onnx': 'alpha' });
    const cache = freshCache();
    const url = modelUrl(manifest.host, 'a.onnx');
    // seed a tampered entry under the pinned key
    await cache.put(url, ab('not alpha at all'));

    const got = await loadVerified(cache, manifest.host, manifest.files[0]);
    expect(got).toBeNull();
    // the bad entry is evicted so the next pass re-downloads
    expect(await cache.get(url)).toBeNull();
  });
});
