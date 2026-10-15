import { describe, expect, it, vi } from 'vitest';

// The i18n module reads locale JSON and touches document-level state at load in the node
// environment; mock it so only the pure selection logic is under test.
vi.mock('../i18n', () => ({ t: (key: string) => key }));

import { selectAutoStartIds } from './mcp';
import type { McpServerConfig, McpServerStatusInfo } from '../types/mcp';

function cfg(id: string, autoStart: boolean): McpServerConfig {
  return {
    id,
    displayName: id,
    transport: { transport: 'http', url: 'https://x.example/sse', headers: {} },
    enabledTools: [],
    autoStart,
    hasStoredSecret: false,
  };
}

function status(s: string): McpServerStatusInfo {
  return {
    id: 'x',
    status: s as McpServerStatusInfo['status'],
    toolCount: 0,
    resourceCount: 0,
    promptCount: 0,
  };
}

describe('selectAutoStartIds', () => {
  it('excludes untrusted project servers and already-connected ones', () => {
    const map = { a: cfg('a', true), b: cfg('b', true), c: cfg('c', true), d: cfg('d', false) };
    const statusMap = { c: status('connected') };
    const untrusted = new Set(['b']);
    expect(selectAutoStartIds(map, statusMap, untrusted)).toEqual(['a']);
  });

  it('blocks untrusted autoStart servers entirely until trusted', () => {
    const map = { b: cfg('b', true) };
    expect(selectAutoStartIds(map, {}, new Set(['b']))).toEqual([]);
    expect(selectAutoStartIds(map, {}, new Set())).toEqual(['b']);
  });

  it('skips servers marked without autoStart', () => {
    const map = { a: cfg('a', false), b: cfg('b', true) };
    expect(selectAutoStartIds(map, {}, new Set())).toEqual(['b']);
  });
});
