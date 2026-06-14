// Browser-local storage for the on-device model files (R11, R12).
//
// The model files (~327 MB) are fetched once and cached so subsequent visits are
// instant and offline-capable. The backend is abstracted behind `CacheBackend`
// so the U1 Cache-API-vs-OPFS decision is a one-line swap (the default export
// `modelCache()` picks the backend), and so tests inject an in-memory backend
// without a real browser Cache.
//
// Corrupt or missing entries resolve to `null` (never throw) — the download
// layer treats a `null`/mismatched load as "re-download", so a half-evicted or
// tampered cache degrades to a clean re-fetch rather than a crash.

/**
 * The low-level byte store. The model URL/path is the key. Implementations may
 * back onto the Cache API, OPFS, or (in tests) a Map.
 */
export interface CacheBackend {
  read(key: string): Promise<ArrayBuffer | null>;
  write(key: string, bytes: ArrayBuffer): Promise<void>;
  delete(key: string): Promise<void>;
  keys(): Promise<string[]>;
  clear(): Promise<void>;
}

/**
 * The outcome of a cache `put`. A write failure (e.g. the origin's quota is
 * exhausted) is NOT a download error — the bytes were fetched + verified fine, so
 * the caller must treat a put failure distinctly (do not re-fetch the whole file).
 */
export interface PutResult {
  /** True when the bytes were committed to the backend. */
  readonly ok: boolean;
  /** Set when `ok` is false — the underlying write error (for classification). */
  readonly error?: unknown;
}

/** The cache surface the download/load layers use. */
export interface ModelCache {
  get(path: string): Promise<ArrayBuffer | null>;
  /** Commit verified bytes. Returns a result rather than throwing, so a quota/
   *  write failure on put is distinguishable from a network download failure. */
  put(path: string, bytes: ArrayBuffer): Promise<PutResult>;
  has(path: string): Promise<boolean>;
  remove(path: string): Promise<void>;
  clear(): Promise<void>;
  /** Ask the browser to make storage persistent (best-effort eviction guard). */
  requestPersist(): Promise<boolean>;
}

/**
 * A `ModelCache` over any `CacheBackend`. The persist call is injected so it can
 * be the real `navigator.storage.persist` in the browser and a stub in tests.
 */
export class BackedModelCache implements ModelCache {
  constructor(
    private readonly backend: CacheBackend,
    private readonly persist: () => Promise<boolean> = defaultPersist,
  ) {}

  async get(path: string): Promise<ArrayBuffer | null> {
    try {
      return await this.backend.read(path);
    } catch {
      // A corrupt/partial entry must not throw — the caller re-downloads on null.
      return null;
    }
  }

  async put(path: string, bytes: ArrayBuffer): Promise<PutResult> {
    try {
      await this.backend.write(path, bytes);
      return { ok: true };
    } catch (error) {
      // A write failure (commonly a quota error) is NOT a network failure — the
      // bytes were already fetched + verified. Report it so the caller can react
      // distinctly (surface "storage full", do NOT re-fetch the whole file).
      return { ok: false, error };
    }
  }

  async has(path: string): Promise<boolean> {
    try {
      return (await this.backend.read(path)) !== null;
    } catch {
      return false;
    }
  }

  async remove(path: string): Promise<void> {
    try {
      await this.backend.delete(path);
    } catch {
      // Removing an absent/corrupt entry is a no-op from the caller's view.
    }
  }

  async clear(): Promise<void> {
    await this.backend.clear();
  }

  async requestPersist(): Promise<boolean> {
    try {
      return await this.persist();
    } catch {
      return false;
    }
  }
}

async function defaultPersist(): Promise<boolean> {
  if (typeof navigator !== 'undefined' && navigator.storage?.persist) {
    return navigator.storage.persist();
  }
  return false;
}

const CACHE_NAME = 'talk-models-v1';

/**
 * `CacheBackend` over the browser Cache API. Stores each file as a synthetic
 * `Response` keyed by its path. Reading an absent or unreadable entry yields
 * `null`. This is the U1-default backend; swapping to OPFS is a one-line change
 * in `modelCache()`.
 */
export class CacheStorageBackend implements CacheBackend {
  constructor(private readonly cacheName: string = CACHE_NAME) {}

  private open(): Promise<Cache> {
    return caches.open(this.cacheName);
  }

  async read(key: string): Promise<ArrayBuffer | null> {
    const cache = await this.open();
    const res = await cache.match(key);
    if (!res) return null;
    return res.arrayBuffer();
  }

  async write(key: string, bytes: ArrayBuffer): Promise<void> {
    const cache = await this.open();
    await cache.put(key, new Response(bytes));
  }

  async delete(key: string): Promise<void> {
    const cache = await this.open();
    await cache.delete(key);
  }

  async keys(): Promise<string[]> {
    const cache = await this.open();
    const reqs = await cache.keys();
    return reqs.map((r) => r.url);
  }

  async clear(): Promise<void> {
    await caches.delete(this.cacheName);
  }
}

/** An in-memory `CacheBackend` for tests (and a fallback when no Cache API). */
export class MemoryBackend implements CacheBackend {
  private readonly store = new Map<string, ArrayBuffer>();

  async read(key: string): Promise<ArrayBuffer | null> {
    return this.store.get(key) ?? null;
  }

  async write(key: string, bytes: ArrayBuffer): Promise<void> {
    this.store.set(key, bytes.slice(0));
  }

  async delete(key: string): Promise<void> {
    this.store.delete(key);
  }

  async keys(): Promise<string[]> {
    return [...this.store.keys()];
  }

  async clear(): Promise<void> {
    this.store.clear();
  }
}

/** True when the browser exposes the Cache API. */
export function hasCacheStorage(): boolean {
  return typeof caches !== 'undefined';
}

/**
 * The default model cache. Picks the Cache API backend when available, else an
 * in-memory backend (degraded — survives only the tab session). The U1 OPFS
 * decision swaps the backend here, on one line.
 */
export function modelCache(): ModelCache {
  const backend: CacheBackend = hasCacheStorage() ? new CacheStorageBackend() : new MemoryBackend();
  return new BackedModelCache(backend);
}
