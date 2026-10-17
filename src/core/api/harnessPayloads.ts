/**
 * Harness workbench backend payload types.
 *
 * Mirrors the serde structs in `src-tauri/src/plugins/harness/model.rs`
 * (HarnessEvent / HarnessApprovalRequest / usage / plan / file change) and the
 * event envelopes (snake_case wrapper fields + camelCase model fields, exactly
 * as the backend serializes them). Types-only — no runtime behavior.
 */

/** One file change reported by a harness tool call (camelCase, mirrors FileChangeInfo). */
export interface HarnessFileChange {
  filePath: string;
  action: string;
  diff: string;
}

/** One plan/task-board entry pushed by the harness agent. */
export interface HarnessPlanEntry {
  content: string;
  priority: string;
  status: string;
}

/** Usage / cost snapshot (all fields optional). */
export interface HarnessUsage {
  inputTokens?: number;
  outputTokens?: number;
  contextTokens?: number;
  contextWindow?: number;
  costUsd?: number;
}

/** One selectable value of a session-config option (model / thought level ...). */
export interface HarnessConfigValue {
  id: string;
  name?: string;
}

/** A session-config option exposed for the GUI (model selector, thought-level
 * selector, or boolean toggle). ACP agents report these in session/new;
 * native adapters synthesize them from their own surfaces. */
export interface HarnessConfigOption {
  optionId: string;
  name?: string;
  /** select | boolean */
  kind: string;
  /** Semantic category hint (model | thought_level | model_config | mode | other). */
  category?: string;
  /** select: currently selected value id */
  current?: string;
  /** select: available values */
  values: HarnessConfigValue[];
  /** boolean: current on/off state */
  booleanValue?: boolean;
}

/** Normalized harness event (`harness-event` payload `event` field). */
export interface HarnessEvent {
  kind:
    | 'session_info'
    | 'config_options'
    | 'turn_started'
    | 'text_delta'
    | 'thinking_delta'
    | 'tool_call_start'
    | 'tool_call_update'
    | 'tool_call_end'
    | 'plan_update'
    | 'subagent_delta'
    | 'usage_update'
    | 'turn_ended'
    | 'notice'
    | 'error';
  harnessSessionId?: string;
  model?: string;
  text?: string;
  toolCallId?: string;
  name?: string;
  title?: string;
  toolKind?: string;
  arguments?: string;
  status?: string;
  content?: string;
  rawOutput?: string;
  ok?: boolean;
  error?: string;
  fileChanges?: HarnessFileChange[];
  entries?: HarnessPlanEntry[];
  usage?: HarnessUsage;
  stopReason?: string;
  message?: string;
  /** config_options: the agent's full current option set */
  options?: HarnessConfigOption[];
}

/** `harness-event` payload. */
export interface HarnessEventPayload {
  tool_id: string;
  session_row_id: string;
  event: HarnessEvent;
}

/** `harness-state` payload (state: connecting | running | idle | error). */
export interface HarnessStatePayload {
  tool_id: string;
  session_row_id: string;
  state: 'connecting' | 'running' | 'idle' | 'error';
  error?: string;
}

/** One selectable option of a harness approval request. */
export interface HarnessApprovalOption {
  optionId: string;
  name: string;
  kind: string;
}

/** Harness approval request body. */
export interface HarnessApprovalRequest {
  requestId: string;
  kind: string;
  title?: string;
  toolName?: string;
  arguments?: unknown;
  command?: string;
  filePath?: string;
  previewDiff?: string;
  options: HarnessApprovalOption[];
}

/** `harness-approval-requested` payload. */
export interface HarnessApprovalRequestedPayload {
  tool_id: string;
  session_row_id: string;
  request: HarnessApprovalRequest;
}

/** `harness-approval-expired` payload. */
export interface HarnessApprovalExpiredPayload {
  request_id: string;
}

/** Tool availability detection row (`harness_detect_tools`). */
export interface HarnessToolStatus {
  id: string;
  /** Display name (brand names for built-ins; user-chosen for customs). */
  name: string;
  state: 'ready' | 'installable' | 'missing';
  /** Which adapter serves this tool: claude-native | acp | acp-fallback. */
  adapter: string;
  /** True for user-registered custom ACP agents (drives the delete action). */
  custom?: boolean;
}

/** User-registered custom ACP agent. */
export interface HarnessCustomAgent {
  id: string;
  name: string;
  command: string;
  args: string[];
}

/** Registered workspace row. */
export interface HarnessWorkspace {
  id: string;
  toolId: string;
  path: string;
  name: string;
  createdAt: string;
}

/** Topic ("session") row. */
export interface HarnessTopic {
  id: string;
  toolId: string;
  workspacePath: string;
  harnessSessionId?: string;
  title: string;
  modelId?: string;
  createdAt: string;
  updatedAt: string;
}
