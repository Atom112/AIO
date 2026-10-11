import { beforeEach, describe, expect, it } from 'vitest';
import { createJsonCache } from './cache';

/**
 * Node environment has no localStorage — inject a minimal in-memory stub
 * (the factory touches storage lazily, inside read/write).
 * FE-17 / 阶段4 regression: the factory is the single cache implementation
 * shared by the three market pages.
 */
class MemStorage {
  private map = new Map<string, string>();
  getItem(k: string): string | null {
    return this.map.get(k) ?? null;
  }
  setItem(k: string, v: string): void {
    this.map.set(k, v);
  }
  removeItem(k: string): void {
    this.map.delete(k);
  }
}

interface Shape {
  updatedAt: number;
  items: string[];
}

const validate = (raw: unknown): Shape | null => {
  if (!raw || typeof raw !== 'object') return null;
  const v = raw as Record<string, unknown>;
  if (typeof v.updatedAt !== 'number' || !Array.isArray(v.items)) return null;
  return raw as Shape;
};

const KEY = 'test-cache';

beforeEach(() => {
  (globalThis as Record<string, unknown>).localStorage = new MemStorage();
});

describe('createJsonCache', () => {
  it('write→read roundtrip; absent key reads null', () => {
    const cache = createJsonCache<Shape>(KEY, validate);
    expect(cache.read()).toBeNull(); // absent → null, no throw
    const value = { updatedAt: 1, items: ['a'] };
    cache.write(value);
    expect(cache.read()).toEqual(value);
  });

  it('corrupt JSON and rejected shapes degrade to null (never throw)', () => {
    localStorage.setItem(KEY, '{not json');
    expect(createJsonCache<Shape>(KEY, validate).read()).toBeNull();
    localStorage.setItem(KEY, JSON.stringify({ updatedAt: 'x', items: 5 }));
    expect(createJsonCache<Shape>(KEY, validate).read()).toBeNull();
  });

  it('read is memoized: a later external storage change is not re-parsed', () => {
    const cache = createJsonCache<Shape>(KEY, validate);
    cache.write({ updatedAt: 1, items: ['a'] });
    // Another writer (e.g. another code path with its own instance) changes storage
    localStorage.setItem(KEY, JSON.stringify({ updatedAt: 2, items: ['b'] }));
    // The memo keeps this cache's own view (single-load semantics, same as FE-17)
    expect(cache.read()).toEqual({ updatedAt: 1, items: ['a'] });
  });

  it('write failure (quota exceeded) is swallowed and the memo still serves reads', () => {
    const cache = createJsonCache<Shape>(KEY, validate);
    const failing = new MemStorage();
    failing.setItem = () => {
      throw new Error('QuotaExceededError');
    };
    (globalThis as Record<string, unknown>).localStorage = failing;
    expect(() => cache.write({ updatedAt: 3, items: ['c'] })).not.toThrow();
    expect(cache.read()).toEqual({ updatedAt: 3, items: ['c'] });
  });

  it('isFresh honors the TTL window', () => {
    const cache = createJsonCache<Shape>(KEY, validate);
    const ttl = 1000;
    expect(cache.isFresh(Date.now(), ttl)).toBe(true);
    expect(cache.isFresh(Date.now() - ttl, ttl)).toBe(false); // boundary: at TTL age = stale
    expect(cache.isFresh(Date.now() - ttl - 1, ttl)).toBe(false);
    // No timestamp (0 = never written) is always stale
    expect(cache.isFresh(0, ttl)).toBe(false);
  });
});
