import { describe, expect, it } from 'vitest';
import { EVENTS } from './events';
// ?raw inlines the file contents at transform time (vite/client types) — no node:fs
// needed, so this gate test type-checks under the app tsconfig.
import eventsRs from '../../../src-tauri/src/core/events.rs?raw';
import eventsTs from './events.ts?raw';

// Frontend event names must follow the same convention as the kebab-case gate test in backend core/events.rs
describe('EVENTS', () => {
  it('事件名全部为 kebab-case', () => {
    for (const [key, value] of Object.entries(EVENTS)) {
      expect(value, key).toMatch(/^[a-z][a-z0-9]*(-[a-z0-9]+)*$/);
    }
  });
  it('包含主循环事件与 MCP 事件', () => {
    expect(EVENTS.llmChunk).toBe('llm-chunk');
    expect(EVENTS.subagentStart).toBe('subagent-start');
    expect(EVENTS.mcpServerStatus).toBe('mcp-server-status');
    expect(EVENTS.btwChunk).toBe('btw-chunk');
  });
});

/**
 * Event registry parity gate (阶段5): every backend event constant must exist in
 * the frontend EVENTS map and vice versa — a drift on either side means a listener
 * subscribing to an event the backend never emits (dead listener) or a backend
 * emitting an event nothing consumes (silent loss).
 */

/** All `pub const NAME: &str = "value";` event values from the backend registry. */
function backendEventNames(): Set<string> {
  const names = new Set<string>();
  const re = /pub const [A-Z0-9_]+: &str = "([^"]+)";/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(eventsRs)) !== null) {
    names.add(m[1]);
  }
  return names;
}

/** All `'value'` property values from the frontend EVENTS map. */
function frontendEventNames(): Set<string> {
  const body = eventsTs.slice(eventsTs.indexOf('export const EVENTS'));
  const names = new Set<string>();
  const re = /:\s*'([^']+)'/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(body)) !== null) {
    names.add(m[1]);
  }
  return names;
}

describe('event registry parity (events.rs ↔ events.ts)', () => {
  it('both registries parse to a non-trivial event set', () => {
    expect(backendEventNames().size).toBeGreaterThan(20);
    expect(frontendEventNames().size).toBeGreaterThan(20);
  });

  it('every backend event has a frontend constant', () => {
    const backend = backendEventNames();
    const frontend = frontendEventNames();
    const missing = [...backend].filter((n) => !frontend.has(n)).sort();
    expect(
      missing,
      '后端已登记但前端缺常量的事件（请在 events.ts 补充）: ' + missing.join(', '),
    ).toEqual([]);
  });

  it('every frontend constant maps to a real backend event', () => {
    const backend = backendEventNames();
    const frontend = frontendEventNames();
    const ghosts = [...frontend].filter((n) => !backend.has(n)).sort();
    expect(
      ghosts,
      '前端常量指向后端不存在的事件（请在 events.rs 登记或删除常量）: ' + ghosts.join(', '),
    ).toEqual([]);
  });

  it('frontend EVENTS module matches the parsed registry exactly (no dead exports)', () => {
    const fromModule = new Set<string>(Object.values(EVENTS));
    const parsed = frontendEventNames();
    // The parsed set derives from the same file, but assert the module shape too:
    // a duplicate value or an accidentally exported extra would drift silently.
    expect(fromModule.size, 'EVENTS 存在重复值').toBe(parsed.size);
    for (const name of parsed) expect(fromModule).toContain(name);
  });
});
