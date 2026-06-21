// MCP (Model Context Protocol) 前端类型
// 镜像 src-tauri/src/core/models.rs 中的 MCP 相关结构

// ===== 工具（OpenAI 兼容） =====

export interface ToolSpec {
    type: 'function';
    function: ToolFunctionSpec;
}

export interface ToolFunctionSpec {
    name: string;
    description: string;
    /** JSON Schema */
    parameters: any;
}

export interface ToolCall {
    id: string;
    type: 'function';
    function: {
        name: string;
        /** JSON 字符串（OpenAI 规范） */
        arguments: string;
    };
}

export interface ToolResult {
    content: ToolResultContent[];
    isError?: boolean;
}

export interface ToolResultContent {
    type: 'text' | 'image' | 'resource';
    [key: string]: any;
}

/** 按助手视角聚合的 MCP 工具集（list_mcp_tools_for_assistant 返回） */
export interface AssistantTools {
    /** 扁平工具列表，直接喂给 LLM 的 tools 参数 */
    tools: ToolSpec[];
    /** toolName → serverId，供 call_mcp_tool 解析（替代前端启发式查找） */
    toolServerMap: Record<string, string>;
}

// ===== MCP 传输 =====

export type McpTransport =
    | McpStdioTransport
    | McpHttpTransport
    | McpStreamableHttpTransport;

export interface McpStdioTransport {
    transport: 'stdio';
    command: string;
    args: string[];
    /** env 值支持 ${KEYRING:account_id} 占位 */
    env: Record<string, string>;
    cwd?: string;
}

export interface McpHttpTransport {
    transport: 'http';
    url: string;
    headers: Record<string, string>;
}

export interface McpStreamableHttpTransport {
    transport: 'streamable_http';
    url: string;
    headers: Record<string, string>;
}

// ===== MCP 服务器配置 =====

export interface McpServerConfig {
    id: string;
    displayName: string;
    transport: McpTransport;
    /** 工具白名单；空数组 = 全部启用 */
    enabledTools: string[];
    /** 应用启动时是否自动连接（是否被某助手使用由 Assistant.mcpServerIds 决定） */
    autoStart: boolean;
    hasStoredSecret: boolean;
    fromCatalog?: CatalogRef;
}

export interface CatalogRef {
    catalogId: string;
    sourceId: string;
}

export type McpStatus = 'disconnected' | 'connecting' | 'connected' | 'error';

export interface McpServerStatusInfo {
    id: string;
    status: McpStatus;
    message?: string;
    toolCount: number;
}

// ===== 流式事件 =====

/** LLM 工具调用事件负载 */
export interface LlmToolCallPayload {
    assistantId: string;
    topicId: string;
    toolCallId: string;
    name: string;
    arguments: string;
}

// ===== 扩展 Message =====

export interface MessageToolFields {
    toolCallId?: string;
    name?: string;
    toolCalls?: ToolCall[];
}
