// Domain type definitions — pure interface layer split out of store.ts (no runtime dependencies).

import type { MemoryEmbeddingConfig, MemoryRerankConfig } from '../types/memory';

/** Agent step-timeline entry (one per tool call / round in chronological order) */
export interface AgentStep {
  id: string;
  type: 'thinking' | 'tool_call' | 'content' | 'subagent';
  timestamp: number;
  status: 'running' | 'complete' | 'error';
  /** Step duration in ms; computed when the step closes */
  duration?: number;
  /** Thinking step text (only for type=thinking) */
  thinkingText?: string;
  /** Tool call details (only for type=tool_call) */
  toolCall?: ToolCallDisplay;
  /** Interim summary / reply text (only for type=content) */
  contentText?: string;
  // ===== Subagent fields (only for type=subagent) =====
  /** Subagent unique ID */
  subagentId?: string;
  /** Subagent profile identifier (explorer | coder | general) */
  subagentProfile?: string;
  /** Subagent display name */
  subagentName?: string;
  /** Task description assigned to the subagent */
  subagentTask?: string;
  /** Subagent internal step timeline */
  subagentSteps?: SubagentStep[];
  /** Final result produced by the subagent */
  subagentResult?: string;
  /** Full (untruncated) tool result; only valid for type=tool_call */
  fullResult?: string;
}

/** Subagent step (internal timeline; same shape as AgentStep but without nested subagents) */
export interface SubagentStep {
  id: string;
  type: 'thinking' | 'tool_call' | 'content' | 'streaming';
  timestamp: number;
  status: 'running' | 'complete' | 'error';
  duration?: number;
  summary?: string;
  toolName?: string;
  /** Owning LLM round (round field of the backend subagent-step payload; absent in old persisted data) */
  round?: number;
}

/** Message content part (multimodal: text / image URL; aligned with OpenAI content parts) */
export interface ContentPart {
  type: 'text' | 'image_url';
  text?: string;
  image_url?: { url: string };
}

/* Message interface defining the chat message data structure */
export interface Message {
  id?: string; // Unique message ID (needed to persist tool/assistant messages)
  role: 'user' | 'assistant' | 'tool' | 'system'; // Sender role: 'tool' is a tool execution result, 'system' is a system instruction
  /** Message content: plain text string, or multimodal part array (text + images) */
  content: string | ContentPart[];
  modelId?: string; // Model ID that generated the reply; present only on assistant messages
  displayFiles?: AttachmentMeta[]; // Attachment metadata associated with the message
  displayText?: string; // Plain text for UI display (sanitized or parsed)
  reasoning?: string; // Model-native chain of thought (reasoning_content); only assistant messages may carry it
  toolCallId?: string; // tool_call id this role="tool" message answers (OpenAI API requires pairing)
  name?: string; // For role="tool", the invoked function name; for role="assistant" with tool_calls, "assistant"
  toolCalls?: ToolCallDisplay[]; // Tool calls issued by the model on role="assistant" messages (with frontend UI state/result/error)
  agentStartTime?: number; // Agent turn start timestamp (ms), used to compute working time
  interimContent?: string; // Accumulated interim-round summary text across multi-round agent runs
  agentSteps?: AgentStep[]; // Agent step timeline, one entry per step in chronological order
  inputTokens?: number; // Input token usage for this turn/message (accumulated across turns, for cost stats)
  outputTokens?: number; // Output token usage for this turn/message (accumulated across turns, for cost stats)
  contextTokens?: number; // Peak context tokens (input_tokens of the last API call), used for the context window progress bar
  parentMessageId?: string; // Parent message ID (for the conversation branch tree)
  branchIndex?: number; // Branch index (defaults to 0; old data may lack this field)
  cachedInputTokens?: number; // Cache-hit tokens this turn (OpenAI cached_tokens / DeepSeek prompt_cache_hit_tokens)
  images?: GeneratedImageInfo[]; // Model-generated images (assistant messages), for download and history replay
}

/** Tool call for frontend display (OpenAI tool_calls plus UI state fields) */
export interface ToolCallDisplay {
  id: string;
  type: 'function';
  function: { name: string; arguments: string };
  /** UI state: calling / success / error */
  state?: 'calling' | 'success' | 'error';
  /** Plain-text tool result (display + message rebuild; truncated view of fullContent) */
  content?: string;
  /** Structured result returned by the tool */
  result?: any;
  /** Error message (when state=error) */
  error?: string;
  /** File change details (provided by the backend on write/replace/delete) */
  fileChanges?: FileChangeInfo[];
  /** Full (untruncated) tool result. content is the truncated version (10000-char cap). */
  fullContent?: string;
}

/** Single file modification record */
export interface FileChangeInfo {
  /** File path (relative to the project root) */
  filePath: string;
  /** Action type: create | modify | delete */
  action: 'create' | 'modify' | 'delete';
  /** Unified diff text */
  diff: string;
  /** Line change overview "+N -M" */
  summary: string;
}

export interface AttachmentMeta {
  id?: string;
  name: string;
  mimeType?: string;
  size?: number;
  /** Absolute on-disk path of the attachment, used to render image thumbnails (convertFileSrc asset URL) */
  storagePath?: string;
}

/** Model-generated image metadata (assistant messages), for the download button and history replay */
export interface GeneratedImageInfo {
  name: string;
  mimeType: string;
  size: number;
  storagePath: string;
}

export interface StoredAttachment extends AttachmentMeta {
  id: string;
  mimeType: string;
  size: number;
  storagePath: string;
}

export interface PendingAttachment extends StoredAttachment {
  type: 'text' | 'image';
  previewUrl?: string;
}

/* Topic interface defining the conversation topic data structure */
export interface Topic {
  id: string; // Topic unique identifier
  name: string; // Topic display name
  history: Message[]; // Message history of this topic
  summary: string; // Long-term memory summary of the topic; maps to the summary column stored in the database
  /**
   * Whether the topic has been auto-renamed.
   * - false: new topic or pre-migration old topic; after the first conversation it is auto-renamed to the AI-generated title and set to true
   * - true: already auto-renamed or manually renamed; auto-rename never triggers again
   * - absent: tolerated for old data, equivalent to false
   */
  renamed?: boolean;
  /** Source message ID this topic was branched from */
  branchedFromMessageId?: string;
  /** Parent topic ID (for the topic tree structure) */
  parentTopicId?: string;
  /**
   * Compaction context boundary: the first N messages of `history` are condensed into
   * `summary` and excluded from LLM requests (context = summary + history.slice(N)),
   * while the full transcript stays visible in the message record (mainstream harness
   * behavior: DSH shadows surface nodes, pi keeps raw entries). Absent = 0.
   */
  contextTrimCount?: number;
}

/**
 * Agent execution mode (wire values stable — persisted per assistant; the ladder):
 * off = 只读 (read-only) · normal = 工作区内修改 (workspace edits) ·
 * auto = 完全权限 (full autonomy inherent) · plan = 计划
 */
export type AgentMode = 'off' | 'normal' | 'auto' | 'plan';

/**
 * Context compaction outcome (/compact or auto-triggered), so the UI can report accurately.
 * - compressed: summary persisted (overwrites topic.summary and trims history)
 * - archived: summary failed, fell back to local archive (readable via read_artifact)
 * - skipped: compaction not run (below auto threshold / preserve zone covers all history / nothing to compact /
 *   CHAT-2: topic switched mid-summarize or the trim would empty the history — persist aborted to protect data)
 * - failed: summary and archive both failed, history unchanged
 */
export type CompactOutcome =
  | { kind: 'compressed'; summarizedCount: number; keptCount: number }
  | { kind: 'archived'; count: number }
  | {
      kind: 'skipped';
      reason:
        | 'below-threshold'
        | 'short-history'
        | 'nothing-to-compact'
        | 'topic-switched'
        | 'empty-result';
    }
  | { kind: 'failed'; error: string };

/* Assistant interface defining the AI assistant data structure */
export interface Assistant {
  id: string; // Assistant unique identifier
  name: string; // Assistant display name
  prompt: string; // System prompt for the assistant
  modelId?: string; // Preferred model ID for the assistant; falls back to the global default model when unset
  mcpServerIds?: string[]; // MCP server ids enabled for the assistant; empty/unset = no MCP tools (opt-in)
  skillIds?: string[]; // Skill ids enabled for the assistant; empty/unset = no skill instructions injected
  projectId?: string; // Project the assistant belongs to; unset = global assistant
  agentMode?: AgentMode; // Agent execution mode; 'off' = plain conversation
  assistantType?: 'chat' | 'project'; // Assistant type: chat = conversation, project = project assistant
  topics: Topic[]; // Topics belonging to the assistant
}

/* Base app config interface holding global settings such as API connection */
/** Enable/disable setting entry for a memory embedding model. */
export interface EmbeddingModelSetting {
  /** Model id (matches a model registry entry) */
  id: string;
  /** Whether enabled (missing entries are treated as enabled) */
  enabled: boolean;
}

/**
 * App-wide configuration (mirrors the serialized shape of AppConfig in backend core/models/config.rs).
 *
 * Note: the backend serde-renames only common fields to camelCase; autoRetry* and
 * similar stay snake_case for backward compatibility — declare the ACTUAL serialized names here; never assume camelCase.
 */
export interface AppConfig {
  /** Base URL of the API service provider */
  apiUrl: string;
  /** API key for authentication */
  apiKey: string;
  /** User's preferred default model ID */
  defaultModel: string;
  /** Local model directory path */
  localModelPath: string;
  /** Whether to auto-retry failed tool calls (default true; serialized name stays snake_case) */
  auto_retry_enabled: boolean;
  /** Max auto-retry attempts (default 2) */
  auto_retry_count: number;
  /** Delay between retries in ms (default 500) */
  auto_retry_delay_ms: number;
  /** Master switch for project-level memory (semantic RAG) */
  memoryEnabled: boolean;
  /** Whether retrieved memories are auto-injected into the system prompt (default true) */
  memoryAutoInject: boolean;
  /** Whether facts are auto-extracted in the background (default true) */
  memoryAutoExtract: boolean;
  /** Max active facts per project (default 2000) */
  memoryMaxFacts: number;
  /** Token budget for injecting memories into the system prompt (default 3000) */
  memoryInjectionBudgetTokens: number;
  /** Debounce in seconds for background auto-extraction (default 60) */
  memoryExtractDebounceSecs: number;
  /** Memory embedding config (legacy global migration source for project-level embeddings; local only) */
  memoryEmbedding: MemoryEmbeddingConfig;
  /** Global embedding model install directory (empty = default app_data_dir/models/embed) */
  embeddingModelsDir: string;
  /** Global embedding model enable/disable registry */
  embeddingModels: EmbeddingModelSetting[];
  /** Memory rerank config (second-stage rerank after RRF; default off) */
  memoryRerank: MemoryRerankConfig;
  /** Cosine similarity threshold that triggers dedup/arbitration (default 0.92) */
  memoryDedupThreshold: number;
  /** Whether queries are rewritten by an LLM at retrieval time (default off) */
  memoryQueryRewrite: boolean;
  /** Max LLM arbitration calls for daily background consolidation (default 20) */
  memoryConsolidateBudget: number;
  /** Launch the app at system startup (default off) */
  autoStartEnabled: boolean;
  /** Max concurrent subagents (-1 = unlimited; 0 forbidden; undefined = default 5) */
  maxConcurrentSubagents?: number;
  /** Max tool-call rounds per agent run (-1 = unlimited; 0 forbidden; undefined = default) */
  maxToolRounds?: number;
}

/* Activated model config interface holding connection info for available models */
export interface ActivatedModel {
  api_url: string; // Model API endpoint URL
  api_key: string; // API key required to access the model
  model_id: string; // Model unique identifier, e.g. 'gpt-4o', 'llama3'
  owned_by: string; // Model provider or vendor name
  local_path?: string; // Absolute filesystem path of the local model; valid only for local models
  engine_type?: string; // Local inference engine type identifier, e.g. "llama_cpp", "vllm"
}

/** Project interface */
export interface Project {
  id: string;
  name: string;
  path: string;
  createdAt: string;
  updatedAt: string;
  /** Assistant id linked to the project (returned alongside by the backend list_projects command) */
  assistantId: string;
}
