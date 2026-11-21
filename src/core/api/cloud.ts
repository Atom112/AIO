// Cloud sync domain Tauri command wrappers (mirrors src-tauri/src/commands/cloud/mod.rs).
// Convention: components must not invoke these command names directly; always go through this module.
import { invoke } from '@tauri-apps/api/core';

/** Link state of this installation to a team server. */
export interface CloudSessionStatus {
  signedIn: boolean;
  serverUrl: string | null;
  workspaceId: string | null;
  userId: string | null;
  userLogin: string | null;
  deviceId: string | null;
  lastCursor: string | null;
  pending: number;
}

/** Device-authorization grant returned by `cloud_device_start`. */
export interface CloudDeviceStart {
  deviceCode: string;
  userCode: string;
  verificationUri: string;
  verificationUriComplete: string | null;
  expiresIn: number;
  interval: number;
}

export type CloudDevicePollStatus = 'pending' | 'approved' | 'denied' | 'expired' | 'error';

export interface CloudDevicePoll {
  status: CloudDevicePollStatus;
  message: string | null;
}

export interface CloudWorkspace {
  id: string;
  name: string;
  slug: string;
  role: string;
}

export interface CloudMe {
  user: { id: string; email: string; name: string };
  workspaces: CloudWorkspace[];
  serverVersion: string;
}

/** One server change; `payload` carries the entity for upserts. */
export interface CloudRemoteChange {
  cursor?: number | null;
  type: string;
  id: string;
  op?: string;
  version?: number;
  payload?: Record<string, unknown> | null;
}

export interface CloudPushOutcome {
  applied: number;
  conflicts: number;
  uploaded: number;
  pending: number;
  retryable: boolean;
  status: number | null;
  message: string | null;
  signedOut: boolean;
}

export interface CloudPullOutcome {
  changes: CloudRemoteChange[];
  /** Server `sync_events` id: numeric on the wire, opaque to callers. */
  nextCursor: number | null;
  hasMore: boolean;
  cursorExpired: boolean;
  retryable: boolean;
  status: number | null;
  message: string | null;
  signedOut: boolean;
}

export interface CloudSnapshotOutcome {
  applied: number;
  changedTopicIds: string[];
  nextPage: string | null;
  hasMore: boolean;
  snapshotCursor: number | null;
  retryable: boolean;
  status: number | null;
  message: string | null;
  signedOut: boolean;
}

export interface CloudApplyOutcome {
  changedTopicIds: string[];
  applied: number;
  skipped: number;
}

export interface CloudAttachmentRef {
  id: string;
  fileName: string;
  mimeType: string;
  sha256: string;
  sizeBytes: number;
}

export interface CloudStoredAttachment {
  id: string;
  name: string;
  mimeType: string;
  size: number;
  storagePath: string;
}

/** Starts GitHub device sign-in; returns the user code to show the user. */
export const cloudDeviceStart = (serverUrl: string) =>
  invoke<CloudDeviceStart>('cloud_device_start', { serverUrl });

/** Polls the device grant once (`approved` stores the session token in the OS keychain). */
export const cloudDevicePoll = (serverUrl: string, deviceCode: string) =>
  invoke<CloudDevicePoll>('cloud_device_poll', { serverUrl, deviceCode });

/** Lists workspaces the signed-in account can bind to. */
export const cloudListWorkspaces = (serverUrl: string) =>
  invoke<CloudMe>('cloud_list_workspaces', { serverUrl });

/** Binds this installation to one workspace (one workspace per profile). */
export const cloudBindWorkspace = (serverUrl: string, workspaceId: string) =>
  invoke<CloudSessionStatus>('cloud_bind_workspace', { serverUrl, workspaceId });

export const cloudSessionStatus = () => invoke<CloudSessionStatus>('cloud_session_status');

export type CloudAnnouncementSeverity = 'info' | 'warning' | 'critical';

/** One active announcement served by the team backend. */
export interface CloudAnnouncement {
  id: string;
  severity: CloudAnnouncementSeverity;
  scope: 'server' | 'workspace';
  workspaceId: string | null;
  title: string;
  body: string;
  startsAt: string;
  endsAt: string | null;
}

/** Active announcements for the linked account (empty array when signed out). */
export const cloudListAnnouncements = (): Promise<CloudAnnouncement[]> =>
  invoke<CloudAnnouncement[]>('cloud_list_announcements');

export const cloudLogout = () => invoke<void>('cloud_logout');

/** Flushes the local outbox to the server (uploads due attachments first). */
export const cloudSyncPush = () => invoke<CloudPushOutcome>('cloud_sync_push');

/** Pulls one page of server changes. */
export const cloudSyncPull = (cursor?: string | null, limit?: number) =>
  invoke<CloudPullOutcome>('cloud_sync_pull', { cursor: cursor ?? null, limit: limit ?? null });

/** Applies pulled changes locally and advances the cursor atomically. */
export const cloudSyncApply = (changes: CloudRemoteChange[], cursor?: number | null) =>
  invoke<CloudApplyOutcome>('cloud_sync_apply', { changes, cursor: cursor ?? null });

/** Fetches and applies one snapshot page (recovery after an expired cursor). */
export const cloudSyncSnapshot = (page?: string | null, limit?: number) =>
  invoke<CloudSnapshotOutcome>('cloud_sync_snapshot', { page: page ?? null, limit: limit ?? null });

export const cloudStartEvents = () => invoke<void>('cloud_start_events');

export const cloudStopEvents = () => invoke<void>('cloud_stop_events');

/** Uploads queued attachments now and returns how many were transferred. */
export const cloudAttachmentUpload = () => invoke<number>('cloud_attachment_upload');

/** Downloads a remote attachment into the local store. */
export const cloudAttachmentDownload = (attachment: CloudAttachmentRef) =>
  invoke<CloudStoredAttachment>('cloud_attachment_download', { attachment });

/** Uploads the agent-steps or full-tool-result payload for one message (gzip JSON). */
export const cloudUploadMessagePayload = (
  messageId: string,
  kind: 'agent_steps' | 'tool_full_result',
) => invoke<void>('cloud_upload_message_payload', { messageId, kind });

/** Fetches a payload from the server; resolves to the JSON text that was merged locally. */
export const cloudGetMessagePayload = (
  messageId: string,
  kind: 'agent_steps' | 'tool_full_result',
) => invoke<string>('cloud_get_message_payload', { messageId, kind });

/** One event from a cloud run (AG-UI naming: `message.delta`, `tool.call`, ...). */
export interface CloudRunEvent {
  runId: string;
  seq: number;
  type: string;
  payload: Record<string, unknown>;
}

export interface CloudStartedRun {
  runId: string;
  status: string;
}

export interface CloudArtifact {
  id: string;
  kind: string;
  sizeBytes: number;
  sha256: string;
  downloadUrl: string;
}

/** Starts a cloud agent run for a thread and returns its id. */
export const cloudStartRun = (
  threadId: string,
  assistantId: string,
  message: string,
  model?: string | null,
) =>
  invoke<CloudStartedRun>('cloud_start_run', {
    threadId,
    assistantId,
    message,
    model: model ?? null,
  });

/** Streams run events as `cloud-run-event` Tauri events until the run terminates. */
export const cloudStartRunEvents = (runId: string) =>
  invoke<void>('cloud_start_run_events', { runId });

export const cloudStopRunEvents = () => invoke<void>('cloud_stop_run_events');

export const cloudCancelRun = (runId: string) => invoke<void>('cloud_cancel_run', { runId });

export const cloudResolveRunApproval = (
  runId: string,
  approvalId: string,
  decision: 'approve' | 'deny',
) => invoke<void>('cloud_resolve_run_approval', { runId, approvalId, decision });

/** Lists a run's artifacts with presigned download URLs. */
export const cloudRunArtifacts = (runId: string) =>
  invoke<CloudArtifact[]>('cloud_run_artifacts', { runId });
