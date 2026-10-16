/**
 * Harness workbench IPC wrappers (components must import from here, never
 * invoke directly — layering gate).
 */
import { invoke } from '@tauri-apps/api/core';
import type { Message } from '../store/types';
import type {
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

/** Send a user prompt (persisted + submitted to the harness agent); returns the stored user message.
 * `permissionMode` applies to a freshly spawned Claude Code native agent (ignored otherwise). */
export const harnessSendMessage = (
  sessionRowId: string,
  text: string,
  permissionMode?: string,
): Promise<Message> =>
  invoke<Message>('harness_send_message', { sessionRowId, text, permissionMode });

/** Stop the running turn of a topic. */
export const harnessStopSession = (sessionRowId: string): Promise<void> =>
  invoke<void>('harness_stop_session', { sessionRowId });

/** Answer a pending harness approval. */
export const harnessRespondApproval = (requestId: string, optionId: string): Promise<void> =>
  invoke<void>('harness_respond_approval', { requestId, optionId });

/** Which harness topics still have a live runtime (stop-button reconciliation). */
export const harnessListRunning = (): Promise<string[]> => invoke<string[]>('harness_list_running');

/** Switch the permission mode of a running session (Claude Code native; no-op otherwise). */
export const harnessSetPermissionMode = (sessionRowId: string, mode: string): Promise<void> =>
  invoke<void>('harness_set_permission_mode', { sessionRowId, mode });

/** List user-registered custom ACP agents. */
export const harnessListCustomAgents = (): Promise<HarnessCustomAgent[]> =>
  invoke<HarnessCustomAgent[]>('harness_list_custom_agents');

/** Insert or update a custom ACP agent (id generated when empty). */
export const harnessSaveCustomAgent = (agent: HarnessCustomAgent): Promise<HarnessCustomAgent> =>
  invoke<HarnessCustomAgent>('harness_save_custom_agent', { agent });

/** Delete a custom ACP agent by id. */
export const harnessDeleteCustomAgent = (id: string): Promise<void> =>
  invoke<void>('harness_delete_custom_agent', { id });
