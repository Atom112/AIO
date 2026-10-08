// 后端 → 前端事件名常量（与 src-tauri/src/core/events.rs 一一对应，kebab-case 由后端守门测试保证）。
// 约定：新增事件两端同步登记；监听器一律使用本模块常量，禁止裸字符串。
export const EVENTS = {
  /** 引擎启动进度（通用） */
  engineProgress: 'engine-progress',
  /** llama.cpp 历史事件名（向后兼容） */
  llamaProgress: 'llama-progress',
  /** 模型下载进度 */
  modelDownloadProgress: 'model-download-progress',
  /** 嵌入模型索引重建进度 */
  reindexProgress: 'reindex-progress',
  /** 代码库索引进度 */
  codeIndexProgress: 'code-index-progress',
  /** 应用更新下载进度 */
  appUpdateProgress: 'app-update-progress',

  /** 一轮 LLM 调用开始 */
  llmRoundStart: 'llm-round-start',
  /** 流式增量（文本） */
  llmChunk: 'llm-chunk',
  /** 流式增量（推理/思考内容） */
  llmReasoning: 'llm-reasoning',
  /** 工具调用开始 */
  llmToolCall: 'llm-tool-call',
  /** 工具调用结果 */
  llmToolResult: 'llm-tool-result',
  /** 上下文压缩完成 */
  llmCompression: 'llm-compression',
  /** 上下文压缩失败 */
  llmCompressionFailed: 'llm-compression-failed',
  /** 未完成任务已生成交接文档 */
  llmHandoff: 'llm-handoff',

  /** 子智能体启动 / 单步 / 完成 / 错误 */
  subagentStart: 'subagent-start',
  subagentStep: 'subagent-step',
  subagentDone: 'subagent-done',
  subagentError: 'subagent-error',

  /** 工作流开始 / 步骤开始 / 步骤完成 / 完成 */
  workflowStart: 'workflow-start',
  workflowStepStart: 'workflow-step-start',
  workflowStepComplete: 'workflow-step-complete',
  workflowComplete: 'workflow-complete',

  /** MCP server 状态变化 / stderr / 工具列表变化 */
  mcpServerStatus: 'mcp-server-status',
  mcpServerStderr: 'mcp-server-stderr',
  mcpToolsChanged: 'mcp-tools-changed',

  /** 工具审批请求 */
  toolApprovalRequested: 'tool-approval-requested',

  /** BTW 旁路问答流式增量 */
  btwChunk: 'btw-chunk',
} as const;

export type EventName = (typeof EVENTS)[keyof typeof EVENTS];
