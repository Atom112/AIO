// MCP utility functions

import type { McpTransport, McpStatus } from '../types/mcp';
import { t } from '../i18n';

/** Builds a keyring placeholder */
export function buildKeyringPlaceholder(serverId: string, envKey: string): string {
  return `\${KEYRING:mcp-server-${serverId}-env-${envKey}}`;
}

/** Checks whether any env value contains a secret placeholder */
export function hasSecretInEnv(env: Record<string, string>): boolean {
  return Object.values(env).some((v) => v.includes('${KEYRING:'));
}

/** Returns a human-readable label for the transport */
export function transportLabel(t: McpTransport): string {
  switch (t.transport) {
    case 'stdio':
      return `stdio · ${t.command}`;
    case 'http':
      return `HTTP · ${shortUrl(t.url)}`;
    case 'streamable_http':
      return `Streamable HTTP · ${shortUrl(t.url)}`;
  }
}

function shortUrl(url: string): string {
  try {
    const u = new URL(url);
    return u.host + u.pathname;
  } catch {
    return url;
  }
}

/** Status badge color (for the UI) */
export function statusColor(s: McpStatus): string {
  switch (s) {
    case 'connected':
      return 'var(--color-success)';
    case 'connecting':
      return 'var(--color-warn)';
    case 'error':
      return 'var(--color-danger)';
    default:
      return 'rgba(var(--text-base-rgb),0.4)';
  }
}

export function statusLabel(s: McpStatus): string {
  switch (s) {
    case 'connected':
      return t('mcp.connected');
    case 'connecting':
      return t('mcp.connecting');
    case 'error':
      return t('mcp.connectionError');
    default:
      return t('mcp.disconnected');
  }
}
