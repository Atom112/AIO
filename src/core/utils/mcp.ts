// MCP 工具函数

import type { McpServerConfig, McpTransport, McpStatus } from '../types/mcp';

/** keyring 占位符正则：${KEYRING:account_id} */
const KEYRING_PLACEHOLDER = /\$\{KEYRING:([^}]+)\}/g;

/** 解析 env 值中的 keyring 占位符（前端用，仅显示） */
export function parseEnvValue(value: string): string {
    if (!value.includes('${KEYRING:')) return value;
    return value.replace(KEYRING_PLACEHOLDER, (_m, account) => {
        return `[keyring:${account}]`;
    });
}

/** 构造 keyring 占位符 */
export function buildKeyringPlaceholder(serverId: string, envKey: string): string {
    return `\${KEYRING:mcp-server-${serverId}-env-${envKey}}`;
}

/** 检查 env 字段是否包含密钥占位符 */
export function hasSecretInEnv(env: Record<string, string>): boolean {
    return Object.values(env).some(v => v.includes('${KEYRING:'));
}

/** 获取 transport 的人类可读标签 */
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

/** 状态徽章颜色（用于 UI） */
export function statusColor(s: McpStatus): string {
    switch (s) {
        case 'connected': return '#7cd9a0';
        case 'connecting': return '#e0c060';
        case 'error': return '#ff6b6b';
        default: return 'rgba(255,255,255,0.4)';
    }
}

export function statusLabel(s: McpStatus): string {
    switch (s) {
        case 'connected': return '已连接';
        case 'connecting': return '连接中…';
        case 'error': return '错误';
        default: return '未连接';
    }
}

/** 构造一个空配置（新增用） */
export function emptyMcpServerConfig(): McpServerConfig {
    return {
        id: `mcp-${Date.now().toString(36)}`,
        displayName: '',
        transport: {
            transport: 'stdio',
            command: 'npx',
            args: [],
            env: {},
        },
        enabledTools: [],
        autoStart: true,
        hasStoredSecret: false,
    };
}
