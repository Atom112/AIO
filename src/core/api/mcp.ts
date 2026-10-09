// MCP-domain Tauri command wrappers (counterpart of commands/mcp.rs).
// Convention: components must not invoke these command names directly; always go through this module.
import { invoke } from '@tauri-apps/api/core';
import type {
  McpCatalogInstallRequest,
  McpCatalogPage,
  McpServerConfig,
  McpServerStatusInfo,
  ToolSpec,
} from '../types/mcp';

/** List MCP server configs (global list when projectId is empty). */
export const listMcpServers = (projectId?: string | null) =>
  invoke<McpServerConfig[]>('list_mcp_servers', { projectId: projectId ?? null });

/** Start an MCP server and return its tool list. */
export const startMcpServer = (id: string, projectId?: string | null) =>
  invoke<ToolSpec[]>('start_mcp_server', { id, projectId: projectId ?? null });

/** Stop an MCP server. */
export const stopMcpServer = (id: string) => invoke<void>('stop_mcp_server', { id });

/** List runtime status of all MCP servers. */
export const listMcpServerStatuses = () =>
  invoke<Record<string, McpServerStatusInfo>>('list_mcp_server_status');

/** List currently available MCP tools (global when projectId is empty). */
export const listMcpTools = (projectId?: string | null) =>
  invoke<ToolSpec[]>('list_mcp_tools', { projectId: projectId ?? null });

/** Add an MCP server config. */
export const addMcpServer = (config: McpServerConfig, projectId?: string | null) =>
  invoke<void>('add_mcp_server', { config, projectId: projectId ?? null });

/** Test connectivity of an MCP server config (nothing is persisted) and return its tool list. */
export const testMcpServerConnection = (config: McpServerConfig) =>
  invoke<ToolSpec[]>('test_mcp_server_connection', { config });

/** Paged query params for the MCP marketplace catalog. */
export interface McpCatalogQuery {
  search?: string | null;
  cursor?: string | null;
  limit?: number;
  forceRefresh?: boolean;
}

/** Browse the MCP marketplace catalog (paginated). */
export const listMcpCatalog = (q: McpCatalogQuery) =>
  invoke<McpCatalogPage>('list_mcp_catalog', q as unknown as Record<string, unknown>);

/** Remove an MCP server. */
export const removeMcpServer = (id: string, projectId?: string | null) =>
  invoke<void>('remove_mcp_server', { id, projectId: projectId ?? null });

/** Save an MCP server secret to the system keyring; returns the placeholder to write into the config. */
export const saveMcpServerSecret = (
  serverId: string,
  target: 'env' | 'header',
  key: string,
  value: string,
) => invoke<string>('save_mcp_server_secret', { serverId, target, key, value });

/** Respond to a tool approval (approved = allow once, false = deny; always='session' = session-level allow). */
export const respondToolApproval = (approvalId: string, approved: boolean, always?: 'session') =>
  invoke<void>('respond_tool_approval', { approvalId, approved, always });

/** List available MCP transport types. */
export const listMcpTransports = () => invoke<string[]>('list_mcp_transports');

/** Check whether the runtime required by a package type is available (e.g. npm/http). */
export const checkMcpCatalogRuntime = (packageType: string) =>
  invoke<boolean>('check_mcp_catalog_runtime', { packageType });

/** Install an MCP server from the marketplace catalog (per package type + user-supplied values); returns its config. */
export const installMcpCatalogServer = (request: McpCatalogInstallRequest) =>
  invoke<McpServerConfig>('install_mcp_catalog_server', {
    request: request as unknown as Record<string, unknown>,
  });
