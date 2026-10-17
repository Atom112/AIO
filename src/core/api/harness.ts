/**
 * Harness workbench IPC wrappers (components must import from here, never
 * invoke directly — layering gate).
 */
import { invoke } from '@tauri-apps/api/core';
import type { Message } from '../store/types';
import type {
  HarnessConfigOption,
  HarnessCustomAgent,
  HarnessToolStatus,
  HarnessTopic,
  HarnessWorkspace,
} from './harnessPayloads';

/** Detect every built-in harness tool's availability. */
export const harnessDetectTools = (): Promise<HarnessToolStatus[]> =>
  invoke<HarnessToolStatus[]>('harness_detect_tools');

/** List every registered harness workspace. */
export const harnessListWorkspaces = (): Promise<HarnessWorkspace[]> =>
  invoke<HarnessWorkspace[]>('harness_list_workspaces');

/** Register a workspace for a tool (path must be an existing directory). */
export const harnessAddWorkspace = (
  toolId: string,
  path: string,
  name?: string,
): Promise<HarnessWorkspace> =>
  invoke<HarnessWorkspace>('harness_add_workspace', { toolId, path, name });

/** Remove a workspace and all topics recorded under it. */
export const harnessRemoveWorkspace = (workspaceId: string): Promise<void> =>
  invoke<void>('harness_remove_workspace', { workspaceId });

/** List the topics of one (tool, workspace). */
export const harnessListSessions = (
  toolId: string,
  workspacePath: string,
): Promise<HarnessTopic[]> =>
  invoke<HarnessTopic[]>('harness_list_sessions', { toolId, workspacePath });

/** Create a new topic under (tool, workspace). */
export const harnessNewSession = (
  toolId: string,
  workspacePath: string,
  title: string,
): Promise<HarnessTopic> =>
  invoke<HarnessTopic>('harness_new_session', { toolId, workspacePath, title });

/** Rename a topic. */
export const harnessRenameSession = (sessionRowId: string, title: string): Promise<void> =>
  invoke<void>('harness_rename_session', { sessionRowId, title });

/** Delete a topic. */
export const harnessDeleteSession = (sessionRowId: string): Promise<void> =>
  invoke<void>('harness_delete_session', { sessionRowId });

/** Load a topic's message history. */
export const harnessLoadMessages = (sessionRowId: string): Promise<Message[]> =>
  invoke<Message[]>('harness_load_messages', { sessionRowId });

/** Persist an assistant message the frontend assembled from the event stream. */
export const harnessAppendMessage = (sessionRowId: string, message: Message): Promise<void> =>
  invoke<void>('harness_append_message', { sessionRowId, message });

/** Frontend-stored native-adapter preferences applied to a freshly spawned
 * harness runtime (permission mode / model / thinking level); ACP agents own
 * their state and ignore them. */
export interface HarnessSpawnPrefs {
  permissionMode?: string;
  model?: string;
  thinkingLevel?: string;
  /** Registry-declared spawn-option values keyed by option id (e.g. omp's approval-mode). */
  spawnOptionValues?: Record<string, string>;
}

/** Send a user prompt (persisted + submitted to the harness agent); returns the stored user message. */
export const harnessSendMessage = (
  sessionRowId: string,
  text: string,
  prefs?: HarnessSpawnPrefs,
): Promise<Message> => invoke<Message>('harness_send_message', { sessionRowId, text, prefs });

/** Stop the running turn of a topic. */
export const harnessStopSession = (sessionRowId: string): Promise<void> =>
  invoke<void>('harness_stop_session', { sessionRowId });

/** Eagerly connect a topic: spawn the agent runtime without a prompt so the
 * live state and config controls appear on selection. Idempotent. */
export const harnessConnectTopic = (
  sessionRowId: string,
  prefs?: HarnessSpawnPrefs,
): Promise<void> => invoke<void>('harness_connect_topic', { sessionRowId, prefs });

/** Answer a pending harness approval. */
export const harnessRespondApproval = (requestId: string, optionId: string): Promise<void> =>
  invoke<void>('harness_respond_approval', { requestId, optionId });

/** Which harness topics still have a live runtime (stop-button reconciliation). */
export const harnessListRunning = (): Promise<string[]> => invoke<string[]>('harness_list_running');

/** Switch the permission mode of a running session (Claude Code native; no-op otherwise). */
export const harnessSetPermissionMode = (sessionRowId: string, mode: string): Promise<void> =>
  invoke<void>('harness_set_permission_mode', { sessionRowId, mode });

/** Set a session config option (model / thought level / boolean) on a running session. */
export const harnessSetConfigOption = (
  sessionRowId: string,
  configId: string,
  value: string | boolean,
): Promise<void> => invoke<void>('harness_set_config_option', { sessionRowId, configId, value });

/** Synthesized config options for a NATIVE adapter without a live session
 * (first-use dropdown data; ACP agents own their lists → empty). */
export const harnessToolConfigOptions = (toolId: string): Promise<HarnessConfigOption[]> =>
  invoke<HarnessConfigOption[]>('harness_tool_config_options', { toolId });

/** Setup/auth surface of one tool (settings modal). */
export interface HarnessToolSetupInfo {
  /** The tool's own auth/config probe files exist. */
  configured: boolean;
  /** Env var receiving an AIO-stored API key at spawn (null = no injection). */
  apiKeyEnv: string | null;
  /** A key is stored in AIO's secure store for this tool. */
  hasAioKey: boolean;
  authDocsUrl: string;
  /** Workspace instruction file convention (CLAUDE.md / AGENTS.md / ...). */
  instructionFile: string;
}

/** Setup/auth status of one tool. */
export const harnessToolSetupInfo = (toolId: string): Promise<HarnessToolSetupInfo> =>
  invoke<HarnessToolSetupInfo>('harness_tool_setup_info', { toolId });

/** Store (or clear with an empty string) a tool's API key in the OS keychain. */
export const harnessSetToolApiKey = (toolId: string, apiKey: string): Promise<void> =>
  invoke<void>('harness_set_tool_api_key', { toolId, apiKey });

/** Workspace instruction file payload. */
export interface HarnessInstruction {
  path: string;
  exists: boolean;
  content: string;
}

/** Read the workspace's instruction file for the tool (empty when absent). */
export const harnessInstructionRead = (
  toolId: string,
  workspacePath: string,
): Promise<HarnessInstruction> =>
  invoke<HarnessInstruction>('harness_instruction_read', { toolId, workspacePath });

/** Write (create or overwrite) the workspace's instruction file for the tool. */
export const harnessInstructionWrite = (
  toolId: string,
  workspacePath: string,
  content: string,
): Promise<void> => invoke<void>('harness_instruction_write', { toolId, workspacePath, content });

// ===== Tool config files (settings pages) =====

/** One user-level config file slot with live file state. */
export interface HarnessToolConfigSlot {
  slotId: string;
  label: string;
  /** json | jsonc | toml | yaml */
  format: string;
  path: string;
  exists: boolean;
  sizeBytes?: number;
  /** Unix epoch seconds of the last modification (staleness token). */
  modified?: number;
  hasBackup: boolean;
  /** YAML: whole-file rewrites lose comments (the UI warns). */
  commentsLostOnWrite: boolean;
}

/** One config file's content + staleness token. */
export interface HarnessToolConfigContent {
  content: string;
  modified?: number;
}

/** List the tool's config file slots with live state. */
export const harnessToolConfigSlots = (toolId: string): Promise<HarnessToolConfigSlot[]> =>
  invoke<HarnessToolConfigSlot[]>('harness_tool_config_slots', { toolId });

/** Read one config slot (content + mtime staleness token). */
export const harnessToolConfigRead = (
  toolId: string,
  slotId: string,
): Promise<HarnessToolConfigContent> =>
  invoke<HarnessToolConfigContent>('harness_tool_config_read', { toolId, slotId });

/** Write one config slot (validated; backup; stale-mtime guard; atomic). */
export const harnessToolConfigWrite = (
  toolId: string,
  slotId: string,
  content: string,
  expectedModified?: number,
): Promise<void> =>
  invoke<void>('harness_tool_config_write', {
    toolId,
    slotId,
    content,
    expectedModified,
  });

/** Restore the pre-AIO backup over the current config file. */
export const harnessToolConfigRestore = (toolId: string, slotId: string): Promise<void> =>
  invoke<void>('harness_tool_config_restore', { toolId, slotId });

/** DSH profile stack entry (read-only overview). */
export interface DshProfileInfo {
  name: string;
  path: string;
}

/** DSH profile stack listing. */
export const harnessToolDshProfiles = (): Promise<DshProfileInfo[]> =>
  invoke<DshProfileInfo[]>('harness_tool_dsh_profiles');

// ===== Structured config keys (settings form) =====

/** One curated key's value (None-shaped = key absent → tool default). */
export type HarnessStructuredValue =
  | { kind: 'str'; value?: string }
  | { kind: 'bool'; value?: boolean }
  | { kind: 'num'; value?: number }
  | { kind: 'strList'; value: string[] }
  | { kind: 'kv'; value: [string, string][] };

export interface HarnessStructuredKey {
  /** Dotted path ("permissions.defaultMode"). */
  key: string;
  label: string;
  /** string | enum | bool | number | string-list | kv */
  kind: string;
  options: string[];
  value: HarnessStructuredValue;
  /** Tool-provided description (omp's catalog carries one; label tooltip). */
  description?: string;
}

/** Structured view of the tool's curated config keys. */
export const harnessToolStructuredGet = (toolId: string): Promise<HarnessStructuredKey[]> =>
  invoke<HarnessStructuredKey[]>('harness_tool_structured_get', { toolId });

/** Set one curated key (null removes the key → tool default). */
export const harnessToolStructuredSet = (
  toolId: string,
  key: string,
  value: unknown,
): Promise<void> => invoke<void>('harness_tool_structured_set', { toolId, key, value });

// ===== Markdown collections (skills / prompts / commands) =====

/** One markdown collection entry (a skill / prompt / command). */
export interface HarnessMarkdownEntry {
  name: string;
  path: string;
  /** First markdown heading — one-line preview. */
  summary: string;
}

/** List a tool's markdown collection entries. */
export const harnessToolMdEntries = (
  toolId: string,
  kind: string,
): Promise<HarnessMarkdownEntry[]> =>
  invoke<HarnessMarkdownEntry[]>('harness_tool_md_entries', { toolId, kind });

/** Read one markdown entry's content (empty when absent). */
export const harnessToolMdRead = (toolId: string, kind: string, name: string): Promise<string> =>
  invoke<string>('harness_tool_md_read', { toolId, kind, name });

/** Write (create or overwrite) one markdown entry. */
export const harnessToolMdWrite = (
  toolId: string,
  kind: string,
  name: string,
  content: string,
): Promise<void> => invoke<void>('harness_tool_md_write', { toolId, kind, name, content });

/** Delete one markdown entry. */
export const harnessToolMdDelete = (toolId: string, kind: string, name: string): Promise<void> =>
  invoke<void>('harness_tool_md_delete', { toolId, kind, name });
// ===== MCP server management =====

/** One MCP server entry (normalized across tools). */
export interface HarnessMcpEntry {
  name: string;
  /** stdio | http */
  type: string;
  command: string;
  url?: string;
  args?: string[];
  env?: [string, string][];
  enabled?: boolean;
}

/** List the tool's MCP servers from its own config file. */
export const harnessToolMcpList = (toolId: string): Promise<HarnessMcpEntry[]> =>
  invoke<HarnessMcpEntry[]>('harness_tool_mcp_list', { toolId });

/** Add/update one MCP server entry (tool-native shape on write). */
export const harnessToolMcpSet = (
  toolId: string,
  name: string,
  entry: HarnessMcpEntry,
): Promise<void> => invoke<void>('harness_tool_mcp_set', { toolId, name, entry });

/** Delete one MCP server entry. */
export const harnessToolMcpDelete = (toolId: string, name: string): Promise<void> =>
  invoke<void>('harness_tool_mcp_delete', { toolId, name });

/** List user-registered custom ACP agents. */
export const harnessListCustomAgents = (): Promise<HarnessCustomAgent[]> =>
  invoke<HarnessCustomAgent[]>('harness_list_custom_agents');

/** Insert or update a custom ACP agent (id generated when empty). */
export const harnessSaveCustomAgent = (agent: HarnessCustomAgent): Promise<HarnessCustomAgent> =>
  invoke<HarnessCustomAgent>('harness_save_custom_agent', { agent });

/** Delete a custom ACP agent by id. */
export const harnessDeleteCustomAgent = (id: string): Promise<void> =>
  invoke<void>('harness_delete_custom_agent', { id });
