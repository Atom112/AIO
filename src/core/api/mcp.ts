// MCP 域 Tauri 命令封装（与 commands/mcp.rs 对应）。
// 约定：组件不得直接 invoke 这些命令名，一律走本模块。
import { invoke } from '@tauri-apps/api/core';
import type {
  McpCatalogInstallRequest,
  McpCatalogPage,
  McpServerConfig,
  McpServerStatusInfo,
  ToolSpec,
} from '../types/mcp';

/** 列出 MCP 服务器配置（projectId 为空时列全局）。 */
export const listMcpServers = (projectId?: string | null) =>
  invoke<McpServerConfig[]>('list_mcp_servers', { projectId: projectId ?? null });

/** 启动一个 MCP server 并返回其工具列表。 */
export const startMcpServer = (id: string, projectId?: string | null) =>
  invoke<ToolSpec[]>('start_mcp_server', { id, projectId: projectId ?? null });

/** 停止一个 MCP server。 */
export const stopMcpServer = (id: string) => invoke<void>('stop_mcp_server', { id });

/** 列出全部 MCP server 的运行时状态。 */
export const listMcpServerStatuses = () =>
  invoke<Record<string, McpServerStatusInfo>>('list_mcp_server_status');

/** 列出当前可用的 MCP 工具（projectId 为空时取全局）。 */
export const listMcpTools = (projectId?: string | null) =>
  invoke<ToolSpec[]>('list_mcp_tools', { projectId: projectId ?? null });

/** 新增一个 MCP server 配置。 */
export const addMcpServer = (config: McpServerConfig, projectId?: string | null) =>
  invoke<void>('add_mcp_server', { config, projectId: projectId ?? null });

/** 测试一个 MCP server 配置的连通性（不落盘），返回其工具列表。 */
export const testMcpServerConnection = (config: McpServerConfig) =>
  invoke<ToolSpec[]>('test_mcp_server_connection', { config });

/** MCP 市场目录分页查询参数。 */
export interface McpCatalogQuery {
  search?: string | null;
  cursor?: string | null;
  limit?: number;
  forceRefresh?: boolean;
}

/** 浏览 MCP 市场目录（分页）。 */
export const listMcpCatalog = (q: McpCatalogQuery) =>
  invoke<McpCatalogPage>('list_mcp_catalog', q as unknown as Record<string, unknown>);

/** 删除一个 MCP server。 */
export const removeMcpServer = (id: string, projectId?: string | null) =>
  invoke<void>('remove_mcp_server', { id, projectId: projectId ?? null });

/** 保存一个 MCP server 的敏感值到系统密钥环，返回可写入配置的占位符。 */
export const saveMcpServerSecret = (
  serverId: string,
  target: 'env' | 'header',
  key: string,
  value: string,
) => invoke<string>('save_mcp_server_secret', { serverId, target, key, value });

/** 响应工具审批（approved = 本次允许，false = 拒绝；always='session' = 会话级放行）。 */
export const respondToolApproval = (approvalId: string, approved: boolean, always?: 'session') =>
  invoke<void>('respond_tool_approval', { approvalId, approved, always });

/** 列出可用的 MCP 传输类型。 */
export const listMcpTransports = () => invoke<string[]>('list_mcp_transports');

/** 检查某交付形态所需的运行时是否可用（如 npm/http）。 */
export const checkMcpCatalogRuntime = (packageType: string) =>
  invoke<boolean>('check_mcp_catalog_runtime', { packageType });

/** 从市场目录安装一个 MCP server（按交付形态 + 用户填值），返回其配置。 */
export const installMcpCatalogServer = (request: McpCatalogInstallRequest) =>
  invoke<McpServerConfig>('install_mcp_catalog_server', {
    request: request as unknown as Record<string, unknown>,
  });
