/**
 * Backend stream-event payload types (FE-21).
 *
 * Mirrors the serde structs in `src-tauri/src/core/models/chat.rs` (StreamPayload /
 * RoundStartPayload / ToolCallPayload / ToolResultPayload) and the subagent /
 * approval event payloads. Fields are snake_case exactly as the backend serializes
 * them (Option fields are skipped when None, hence optional here). These types let
 * the stream listeners drop `listen<any>` — the payload shapes become compiler-
 * checked against the actual emitters.
 */

import type { FileChangeInfo, GeneratedImageInfo } from '../store/types';

/** `llm-round-start`: a new LLM round began (frontend pushes an assistant placeholder). */
export interface LlmRoundStartPayload {
  assistant_id: string;
  topic_id: string;
  /** Round number (1-based). */
  round: number;
}

/** `llm-chunk`: streaming text delta; `done=true` carries the turn-epilogue fields. */
export interface LlmChunkPayload {
  assistant_id: string;
  topic_id: string;
  content: string;
  done: boolean;
  /** Present when done=true and the round ended in error/cancellation. */
  error?: string;
  /** Server-reported usage (meaningful when done=true). */
  input_tokens?: number;
  output_tokens?: number;
  /** Peak context tokens (progress-bar driver; distinct from cumulative input_tokens). */
  context_tokens?: number;
  cached_input_tokens?: number;
  /** Model-generated image metadata (done=true with images). */
  images?: GeneratedImageInfo[];
}

/** `llm-reasoning`: chain-of-thought streaming delta. */
export interface LlmReasoningPayload {
  assistant_id: string;
  topic_id: string;
  content: string;
}

/** `llm-tool-call`: the model issued a tool call (drives the "calling" bubble). */
export interface LlmToolCallPayload {
  assistant_id: string;
  topic_id: string;
  tool_call_id: string;
  name: string;
  /** JSON-encoded arguments string. */
  arguments: string;
}

/** `llm-tool-result`: one tool execution finished. */
export interface LlmToolResultPayload {
  assistant_id: string;
  topic_id: string;
  tool_call_id: string;
  /** Plain-text result for display. */
  content: string;
  /** Raw structured result (ToolResultContent array shape). */
  result: unknown;
  is_error: boolean;
  /** File changes (write_file / replace_in_file / delete_file). */
  file_changes?: FileChangeInfo[];
  /** Untruncated tool result (agentSteps display). */
  full_content?: string;
}

/** `llm-compression`: in-run context compaction finished (main agent only). */
export interface LlmCompressionPayload {
  assistant_id: string;
  topic_id: string;
  /** Compression source; `subagent` compactions are unrelated to the main conversation. */
  source?: string;
  summary?: string;
}

/** `tool-approval-requested`: the backend needs user confirmation before running a tool. */
export interface ToolApprovalRequestedPayload {
  approval_id: string;
  server_id: string;
  tool_name: string;
  /** Raw tool arguments (JSON value). */
  arguments: unknown;
  reason: string;
  preview_diff?: string;
  file_path?: string;
}

/** `subagent-start`: a delegated subagent began. */
export interface SubagentStartPayload {
  parent_assistant_id: string;
  parent_topic_id: string;
  subagent_id: string;
  profile_id: string;
  profile_name: string;
  task_summary: string;
}

/** `subagent-step`: internal step update from a running subagent. */
export interface SubagentStepPayload {
  parent_assistant_id: string;
  parent_topic_id: string;
  subagent_id: string;
  /** Owning LLM round (backend subagent-step payload). */
  round: number;
  /** Step kind; `tool_result:<name>` marks a tool result step. */
  step_type: string;
  summary: string;
}

/** `subagent-done`: a subagent finished with a result. */
export interface SubagentDonePayload {
  parent_assistant_id: string;
  parent_topic_id: string;
  subagent_id: string;
  result: string;
}

/** `subagent-error`: a subagent failed. */
export interface SubagentErrorPayload {
  parent_assistant_id: string;
  parent_topic_id: string;
  subagent_id: string;
  error: string;
}
