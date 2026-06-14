import { describe, it, expect } from 'vitest';
import {
  BackedModelCache,
  MemoryBackend,
  type CacheBackend,
} from './cache';

function buf(s: string): ArrayBuffer {
  return new TextEncoder().encode(s).buffer;
}

function text(b: ArrayBuffer | null): string | null {
  return b === null ? null : new TextDecoder().decode(b);
}

describe('BackedModelCache over MemoryBackend', () => {
  it('round-trips get/put/has', async () => {
    // #given an empty cache
    const cache = new BackedModelCache(new MemoryBackend());
    // #when a file is put
    await cache.put('models/a.onnx', buf('weights'));
    // #then it is present and reads back identically
    expect(await cache.has('models/a.onnx')).toBe(true);
    expect(text(await cache.get('models/a.onnx'))).toBe('weights');
  });

  it('returns null + has=false for an absent key', async () => {
    const cache = new BackedModelCache(new MemoryBackend());
    expect(await cache.get('missing')).toBeNull();
    expect(await cache.has('missing')).toBe(false);
  });

  it('remove deletes an entry; removing an absent entry is a no-op', async () => {
    const cache = new BackedModelCache(new MemoryBackend());
    await cache.put('k', buf('v'));
    await cache.remove('k');
    expect(await cache.has('k')).toBe(false);
    await expect(cache.remove('k')).resolves.toBeUndefined();
  });

  it('clear empties the cache', async () => {
    const cache = new BackedModelCache(new MemoryBackend());
    await cache.put('a', buf('1'));
    await cache.put('b', buf('2'));
    await cache.clear();
    expect(await cache.has('a')).toBe(false);
    expect(await cache.has('b')).toBe(false);
  });

  it('isolates stored bytes from later mutation of the source', async () => {
    // #given a put of a typed array, then a mutation of the original
    const cache = new BackedModelCache(new MemoryBackend());
    const src = new Uint8Array([1, 2, 3]);
    await cache.put('k', src.buffer);
    src[0] = 99;
    // #then the cached copy is unchanged
    const got = new Uint8Array((await cache.get('k'))!);
    expect([...got]).toEqual([1, 2, 3]);
  });
});

describe('corrupt backend → null (no throw)', () => {
  // A backend whose read throws (e.g. a corrupt Cache entry / OPFS read error).
  const throwingBackend: CacheBackend = {
    read: () => Promise.reject(new Error('corrupt')),
    write: () => Promise.resolve(),
    delete: () => Promise.reject(new Error('corrupt')),
    keys: () => Promise.resolve([]),
    clear: () => Promise.resolve(),
  };

  it('get returns null instead of throwing', async () => {
    const cache = new BackedModelCache(throwingBackend);
    await expect(cache.get('k')).resolves.toBeNull();
  });

  it('has returns false instead of throwing', async () => {
    const cache = new BackedModelCache(throwingBackend);
    await expect(cache.has('k')).resolves.toBe(false);
  });

  it('remove swallows a backend delete error', async () => {
    const cache = new BackedModelCache(throwingBackend);
    await expect(cache.remove('k')).resolves.toBeUndefined();
  });
});

describe('requestPersist', () => {
  it('returns the injected persist result', async () => {
    const cache = new BackedModelCache(new MemoryBackend(), () => Promise.resolve(true));
    expect(await cache.requestPersist()).toBe(true);
  });

  it('returns false (never throws) when persist rejects', async () => {
    const cache = new BackedModelCache(new MemoryBackend(), () => Promise.reject(new Error('no')));
    await expect(cache.requestPersist()).resolves.toBe(false);
  });

  it('defaults to false when no persist is available', async () => {
    const cache = new BackedModelCache(new MemoryBackend());
    expect(await cache.requestPersist()).toBe(false);
  });
});
