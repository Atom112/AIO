// Backend → frontend event-name constants (one-to-one with src-tauri/src/core/events.rs; kebab-case enforced by the backend gatekeeping test).
// Convention: register new events on both sides in sync; listeners must use these constants, never raw strings.
export const EVENTS = {
  /** Engine start progress (generic) */
  engineProgress: 'engine-progress',
  /** Legacy llama.cpp event name (kept for backward compatibility) */
  llamaProgress: 'llama-progress',
  /** Model download progress */
  modelDownloadProgress: 'model-download-progress',
  /** Local GGUF model download progress (market downloader) */
  localModelDownloadProgress: 'local-model-download-progress',
  /** Embedding-model index rebuild progress */
  reindexProgress: 'reindex-progress',
  /** Codebase indexing progress */
  codeIndexProgress: 'code-index-progress',
  /** App update download progress */
  appUpdateProgress: 'app-update-progress',

  /** Start of one LLM call round */
  llmRoundStart: 'llm-round-start',
  /** Streaming delta (text) */
  llmChunk: 'llm-chunk',
  /** Streaming delta (reasoning/thinking content) */
  llmReasoning: 'llm-reasoning',
  /** Tool call started */
  llmToolCall: 'llm-tool-call',
  /** Tool call result */
  llmToolResult: 'llm-tool-result',
  /** Context compression completed */
  llmCompression: 'llm-compression',
  /** Context compression failed */
  llmCompressionFailed: 'llm-compression-failed',
  /** Handoff document generated for an unfinished task */
  llmHandoff: 'llm-handoff',

  /** Subagent started / stepped / done / error */
  subagentStart: 'subagent-start',
  subagentStep: 'subagent-step',
  /** Subagent live streaming deltas (text / reasoning) */
  subagentChunk: 'subagent-chunk',
  subagentReasoning: 'subagent-reasoning',
  subagentDone: 'subagent-done',
  subagentError: 'subagent-error',

  /** Workflow started / step started / step completed / completed */
  workflowStart: 'workflow-start',
  workflowStepStart: 'workflow-step-start',
  workflowStepComplete: 'workflow-step-complete',
  workflowComplete: 'workflow-complete',

  /** MCP server status change / stderr / tool-list change */
  mcpServerStatus: 'mcp-server-status',
  mcpServerStderr: 'mcp-server-stderr',
  mcpToolsChanged: 'mcp-tools-changed',

  /** Tool approval request */
  toolApprovalRequested: 'tool-approval-requested',

  /** Tool approval expired (backend timed out; the bubble removes itself) */
  toolApprovalExpired: 'tool-approval-expired',

  /** Agent task board changed (create_tasks / update_task) */
  agentTasksChanged: 'agent-tasks-changed',

  /** Agent asks the user clarification questions (options + custom input) */
  agentQuestionRequested: 'agent-question-requested',

  /** BTW side-question streaming delta */
  btwChunk: 'btw-chunk',

  /** LSP diagnostics updated / server stderr / server status */
  lspDiagnosticsUpdated: 'lsp-diagnostics-updated',
  lspServerStderr: 'lsp-server-stderr',
  lspServerStatus: 'lsp-server-status',

  /** Models catalog updated (fresh fetch persisted) */
  modelsCatalogUpdated: 'models-catalog-updated',
} as const;

export type EventName = (typeof EVENTS)[keyof typeof EVENTS];
