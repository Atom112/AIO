/**
 * localStorage-backed JSON cache factory — 阶段4 第6项（设置层样板收敛）.
 *
 * The three market pages (SkillList / McpServerList / LocalModelMarket) each
 * carried a hand-rolled readCache/writeCache pair over the same machinery:
 * getItem + JSON.parse + shape validation with a null degrade, and
 * JSON.stringify + setItem swallowing quota errors. The copies had already
 * drifted once: FE-17 fixed LocalModelMarket re-parsing the entire cache on
 * every reactive read (its `currentEntry()` accessor runs per render), while
 * the other two still re-parse per call.
 *
 * This factory is the single implementation:
 * - `read()` memoizes the parse — parsed once per load, and re-validated only
 *   through this factory's own `write()` (read-modify-write cycles stay memo-correct);
 * - `validate` receives the parsed value and returns the typed shape or null
 *   (it can also normalize — e.g. backfill a field after a schema evolution);
 * - `write()` swallows storage failures: the cache is best-effort and must
 *   never block the feature that uses it;
 * - `isFresh()` is the shared TTL check (all three markets use 6h).
 *
 * The storage is touched lazily (inside read/write), so the module unit-tests
 * under a plain Node environment with an injected stub.
 */

export interface JsonCache<T> {
  /** Parsed snapshot (memoized); null when absent, corrupt, or rejected by validate. */
  read(): T | null;
  /** Persists the value and refreshes the memo; storage failures are swallowed. */
  write(value: T): void;
  /** TTL freshness check (`ts` = the cached-at timestamp to test). */
  isFresh(ts: number, ttlMs: number): boolean;
}

export function createJsonCache<T>(
  key: string,
  validate: (parsed: unknown) => T | null,
): JsonCache<T> {
  let memo: T | null = null;
  let loaded = false;

  const read = (): T | null => {
    if (!loaded) {
      loaded = true;
      try {
        const raw = localStorage.getItem(key);
        if (raw != null) {
          memo = validate(JSON.parse(raw));
        }
      } catch {
        memo = null; // corrupt JSON / storage unavailable → empty cache, never a throw
      }
    }
    return memo;
  };

  const write = (value: T): void => {
    memo = value;
    loaded = true;
    try {
      localStorage.setItem(key, JSON.stringify(value));
    } catch {
      /* storage full / disabled: cache is best-effort */
    }
  };

  const isFresh = (ts: number, ttlMs: number): boolean => Date.now() - ts < ttlMs;

  return { read, write, isFresh };
}
