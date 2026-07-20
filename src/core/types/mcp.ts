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
    version?: string;
    delivery?: string;
}

export interface McpCatalogInput {
    name: string;
    description: string;
    required: boolean;
    secret: boolean;
    defaultValue: string;
    target: 'env' | 'header';
}

export interface McpCatalogDelivery {
    id: string;
    kind: 'http' | 'npm' | 'pypi';
    label: string;
    command: string;
    args: string[];
    url: string;
    inputs: McpCatalogInput[];
}

export interface McpCatalogServer {
    id: string;
    name: string;
    displayName: string;
    description: string;
    version: string;
    repositoryUrl: string;
    websiteUrl: string;
    deliveries: McpCatalogDelivery[];
}

export interface McpCatalogPage {
    servers: McpCatalogServer[];
    nextCursor?: string;
}

export interface McpCatalogInstallRequest {
    server: McpCatalogServer;
    deliveryId: string;
    values: Record<string, string>;
    secrets: Record<string, string>;
}

export type McpStatus = 'disconnected' | 'connecting' | 'connected' | 'error';

export interface McpServerStatusInfo {
    id: string;
    status: McpStatus;
    message?: string;
    toolCount: number;
    resourceCount: number;
    promptCount: number;
}

// ===== MCP Resources =====

export interface McpResource {
    uri: string;
    name: string;
    description?: string;
    mimeType?: string;
}

export interface ReadResourceResult {
    contents: ResourceContent[];
}

export interface ResourceContent {
    uri: string;
    mimeType?: string;
    text?: string;
    blob?: string;
}

// ===== MCP Prompts =====

export interface McpPrompt {
    name: string;
    description?: string;
    arguments?: PromptArgument[];
}

export interface PromptArgument {
    name: string;
    description?: string;
    required?: boolean;
}

export interface GetPromptResult {
    description?: string;
    messages: PromptMessage[];
}

export interface PromptMessage {
    role: string;
    content: any;
}

// ===== 流式事件 =====
// 注意：后端流式事件 payload 使用 snake_case（与 llm-chunk/llm-reasoning/llm-tool-call 一致）。

/** LLM 文本/思维链条目负载（llm-chunk / llm-reasoning） */
export interface LlmStreamPayload {
    assistant_id: string;
    topic_id: string;
    content: string;
    done: boolean;
    /** done=true 时的错误信息 */
    error?: string;
    /** 本轮输入 tokens（done=true 时由服务端返回） */
    input_tokens?: number;
    /** 本轮输出 tokens（done=true 时由服务端返回） */
    output_tokens?: number;
}

/** 新一轮开始负载（llm-round-start） */
export interface LlmRoundStartPayload {
    assistant_id: string;
    topic_id: string;
    round: number;
}

/** LLM 工具调用通知负载（llm-tool-call，仅通知前端展示"调用中"气泡） */
export interface LlmToolCallPayload {
    assistant_id: string;
    topic_id: string;
    tool_call_id: string;
    name: string;
    arguments: string;
}

/** 工具执行结果负载（llm-tool-result） */
export interface LlmToolResultPayload {
    assistant_id: string;
    topic_id: string;
    tool_call_id: string;
    name: string;
    content: string;
    result: any;
    is_error: boolean;
}

/** 工具审批请求负载（tool-approval-requested，snake_case） */
export interface ToolApprovalRequestPayload {
    approval_id: string;
    server_id: string;
    tool_name: string;
    arguments: any;
    reason: string;
}

// ===== 扩展 Message =====

export interface MessageToolFields {
    toolCallId?: string;
    name?: string;
    toolCalls?: ToolCall[];
}
