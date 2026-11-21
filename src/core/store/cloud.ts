// Cloud sync state (session, workspaces, sync loop) for the team backend integration.
//
// The loop lives here, not in Rust, so it can cooperate with the UI: it pauses while the
// model is streaming (`isThinking`) to avoid clobbering in-flight rows, and it merges
// pulled changes straight into the `datas` store instead of reloading every assistant.
// The Rust side owns SQLite writes; this module only orchestrates and mirrors the DTOs
// needed for rendering.
import { createSignal } from 'solid-js';
import { listen } from '@tauri-apps/api/event';
import {
  cloudAttachmentDownload,
  cloudCancelRun,
  cloudGetMessagePayload,
  cloudResolveRunApproval,
  cloudStartRun,
  cloudStartRunEvents,
  cloudStopRunEvents,
  type CloudRunEvent,
  cloudBindWorkspace,
  cloudDevicePoll,
  cloudDeviceStart,
  cloudAcceptInvitation,
  cloudListAnnouncements,
  cloudListInvitations,
  cloudListWorkspaces,
  cloudLogout,
  cloudRejectInvitation,
  cloudSessionStatus,
  cloudStartEvents,
  cloudStopEvents,
  cloudSyncApply,
  cloudSyncPull,
  cloudSyncPush,
  cloudSyncSnapshot,
  type CloudAnnouncement,
  type CloudAttachmentRef,
  type CloudInvitation,
  type CloudDevicePollStatus,
  type CloudMe,
  type CloudRemoteChange,
  type CloudSessionStatus,
  type CloudWorkspace,
} from '../api/cloud';
import { appendMessage, loadAssistants } from '../api/chat';
import { EVENTS } from '../api/events';
import { datas, setDatas } from './chat';
import { dismissedAnnouncementIds, dismissAnnouncement } from './cloudAnnouncements';
import { invitationExpiryTime } from './cloudInvitations';
import { mergeRemoteChanges } from './cloudMerge';
import { asRecord, asString, attachmentRefs, patchMessage } from './cloudMergeHelpers';
import { isThinking } from './stream';
import type { Message } from './types';

export type CloudStatus = 'signed_out' | 'syncing' | 'idle' | 'offline' | 'error';

/** Link state; `signedIn` is only meaningful together with a bound workspace. */
export const [cloudSession, setCloudSession] = createSignal<CloudSessionStatus>({
  signedIn: false,
  serverUrl: null,
  workspaceId: null,
  userId: null,
  userLogin: null,
  deviceId: null,
  lastCursor: null,
  pending: 0,
});

export const [cloudWorkspaces, setCloudWorkspaces] = createSignal<CloudWorkspace[]>([]);
export const [cloudStatus, setCloudStatus] = createSignal<CloudStatus>('signed_out');
export const [cloudPending, setCloudPending] = createSignal(0);
export const [cloudLastError, setCloudLastError] = createSignal<string | null>(null);

/** Active device-authorization grant shown in the settings card. */
export interface CloudDevicePrompt {
  userCode: string;
  verificationUri: string;
  expiresAt: number;
}
export const [cloudDevicePrompt, setCloudDevicePrompt] = createSignal<CloudDevicePrompt | null>(
  null,
);

/** Active announcements plus the ids this installation already dismissed. */
export const [cloudAnnouncements, setCloudAnnouncements] = createSignal<CloudAnnouncement[]>([]);
export const [cloudAnnouncementDismissed, setCloudAnnouncementDismissed] = createSignal<string[]>(
  dismissedAnnouncementIds(),
);

/** Pending workspace invitations for the signed-in account. */
export const [cloudInvitations, setCloudInvitations] = createSignal<CloudInvitation[]>([]);

/** Interval between announcement refreshes; notices are not latency-sensitive. */
const ANNOUNCEMENT_REFRESH_MS = 15 * 60 * 1000;

/** Interval between sync ticks (the SSE feed triggers extra ticks immediately). */
const SYNC_INTERVAL_MS = 3000;
/** Upper bound on pages applied per tick so one sync never blocks the UI for long. */
const MAX_PULL_PAGES = 10;
/** Upper bound on lazy attachment downloads per tick. */
const MAX_ATTACHMENT_DOWNLOADS = 4;

let listenersInstalled = false;
let announcementTimer: number | null = null;
let loopTimer: number | undefined;
let tickRunning = false;
/** Bumped to cancel the device poll loop. */
let devicePollGeneration = 0;
/** Mirror of `cloudDevicePrompt` read by the poll timer (which runs outside reactivity). */
let activeDevicePrompt: CloudDevicePrompt | null = null;
const downloadedAttachments = new Set<string>();

async function installListeners(): Promise<void> {
  if (listenersInstalled) return;
  listenersInstalled = true;
  // Process-level singleton for the app lifetime; no unlisten needed.
  await listen<{ workspaceId: string; cursor: number }>(EVENTS.cloudSyncEvent, () => {
    // A cursor notification means new server data: pull right away instead of waiting
    // for the next tick. Never apply the notification itself (it carries no payload).
    void tick();
  });
}

/** Loads the persisted link state and starts the loop when a workspace is bound. */
export async function initCloud(): Promise<void> {
  await installListeners();
  await refreshCloudSession();
  const session = cloudSession();
  if (!session.signedIn) return;
  // Server-wide notices are worth showing before a workspace is bound; the
  // invitation list only needs a signed-in account.
  void refreshCloudAnnouncements();
  void refreshCloudInvitations();
  startAnnouncementRefresh();
  if (session.workspaceId) {
    await cloudStartEvents().catch(() => undefined);
    startLoop();
  }
}

/** Re-reads the link state from the backend. */
export async function refreshCloudSession(): Promise<void> {
  try {
    const session = await cloudSessionStatus();
    setCloudSession(session);
    setCloudPending(session.pending);
    if (!session.signedIn) {
      setCloudStatus('signed_out');
    } else if (cloudStatus() === 'signed_out') {
      setCloudStatus('idle');
    }
  } catch (error) {
    setCloudStatus('error');
    setCloudLastError(String(error));
  }
}

/** Starts GitHub device sign-in and returns the prompt to display. */
export async function startCloudSignIn(serverUrl: string): Promise<CloudDevicePrompt> {
  const start = await cloudDeviceStart(serverUrl.trim());
  const prompt: CloudDevicePrompt = {
    userCode: start.userCode,
    verificationUri: start.verificationUriComplete ?? start.verificationUri,
    expiresAt: Date.now() + Math.max(start.expiresIn, 60) * 1000,
  };
  activeDevicePrompt = prompt;
  setCloudDevicePrompt(prompt);
  setCloudLastError(null);
  pollDeviceGrant(serverUrl.trim(), start.deviceCode, Math.max(start.interval, 5) * 1000);
  return prompt;
}

/**
 * Polls the device grant until it settles.
 *
 * A cancelled-generation loop instead of `setInterval`: the poll body writes signals,
 * and keeping those writes in a plain async function (not a timer callback) keeps the
 * reactive graph honest and lets a sign-out stop the loop immediately.
 */
function pollDeviceGrant(serverUrl: string, deviceCode: string, intervalMs: number): void {
  stopDevicePoll();
  const generation = devicePollGeneration;
  void (async () => {
    while (devicePollGeneration === generation) {
      await new Promise((resolve) => setTimeout(resolve, intervalMs));
      if (devicePollGeneration !== generation) return;
      const prompt = activeDevicePrompt;
      if (!prompt || prompt.expiresAt < Date.now()) {
        clearDevicePrompt();
        setCloudLastError('expired');
        return;
      }
      try {
        const result = await cloudDevicePoll(serverUrl, deviceCode);
        const status: CloudDevicePollStatus = result.status;
        if (status === 'pending') continue;
        clearDevicePrompt();
        if (status === 'approved') {
          await refreshCloudSession();
          await loadCloudWorkspaces(serverUrl);
          setCloudStatus('idle');
          setCloudLastError(null);
          return;
        }
        setCloudLastError(result.message ?? status);
        return;
      } catch (error) {
        clearDevicePrompt();
        setCloudLastError(String(error));
        return;
      }
    }
  })();
}

/** Clears the visible grant and the poll loop's mirror together. */
function clearDevicePrompt(): void {
  activeDevicePrompt = null;
  setCloudDevicePrompt(null);
}

/** Cancels the device poll loop (generation bump; the loop exits on its next check). */
function stopDevicePoll(): void {
  devicePollGeneration += 1;
}

/** Fetches the workspace list for a signed-in account. */
export async function loadCloudWorkspaces(serverUrl: string): Promise<CloudMe | null> {
  try {
    const me = await cloudListWorkspaces(serverUrl.trim());
    setCloudWorkspaces(me.workspaces);
    return me;
  } catch (error) {
    setCloudLastError(String(error));
    return null;
  }
}

/** Binds this installation to a workspace and starts syncing. */
export async function bindCloudWorkspace(serverUrl: string, workspaceId: string): Promise<void> {
  const session = await cloudBindWorkspace(serverUrl.trim(), workspaceId);
  setCloudSession(session);
  setCloudStatus('idle');
  void refreshCloudAnnouncements();
  void refreshCloudInvitations();
  await cloudStartEvents().catch(() => undefined);
  startLoop();
  void tick();
}

/** Signs out: stops the loop, clears sync bookkeeping, keeps local conversations. */
export async function signOutCloud(): Promise<void> {
  stopLoop();
  stopDevicePoll();
  stopAnnouncementRefresh();
  setCloudAnnouncements([]);
  setCloudInvitations([]);
  clearDevicePrompt();
  await cloudStopEvents().catch(() => undefined);
  await cloudLogout();
  setCloudWorkspaces([]);
  setCloudSession({
    signedIn: false,
    serverUrl: null,
    workspaceId: null,
    userId: null,
    userLogin: null,
    deviceId: null,
    lastCursor: null,
    pending: 0,
  });
  setCloudPending(0);
  setCloudStatus('signed_out');
  setCloudLastError(null);
}

/** Reloads active announcements; a failure keeps the previous list (they are advisory). */
export async function refreshCloudAnnouncements(): Promise<void> {
  const session = cloudSession();
  if (!session.signedIn) {
    setCloudAnnouncements([]);
    return;
  }
  try {
    setCloudAnnouncements(await cloudListAnnouncements());
  } catch {
    // A transient failure must not surface as a sync error.
  }
}

/** Timer callback kept outside `startAnnouncementRefresh` so the reactivity lint
 * rule sees the signal reads only inside `refreshCloudAnnouncements`. */
function announcementRefreshTick(): void {
  void refreshCloudAnnouncements();
}

function startAnnouncementRefresh(): void {
  if (announcementTimer !== null) return;
  announcementTimer = window.setInterval(announcementRefreshTick, ANNOUNCEMENT_REFRESH_MS);
}

function stopAnnouncementRefresh(): void {
  if (announcementTimer === null) return;
  window.clearInterval(announcementTimer);
  announcementTimer = null;
}

/** Reloads pending invitations; a failure keeps the previous list. */
export async function refreshCloudInvitations(): Promise<void> {
  if (!cloudSession().signedIn) {
    setCloudInvitations([]);
    return;
  }
  try {
    setCloudInvitations(await cloudListInvitations());
  } catch {
    // Invitations are advisory until accepted; the settings card stays usable.
  }
}

/** Joins the workspace behind an invitation and refreshes the workspace list. */
export async function acceptCloudInvitation(invitationId: string): Promise<void> {
  await cloudAcceptInvitation(invitationId);
  await refreshCloudInvitations();
  const serverUrl = cloudSession().serverUrl;
  if (serverUrl) await loadCloudWorkspaces(serverUrl);
}

/** Declines an invitation; the row disappears from the pending list. */
export async function rejectCloudInvitation(invitationId: string): Promise<void> {
  await cloudRejectInvitation(invitationId);
  await refreshCloudInvitations();
}

/** Expiry label for the settings card. */
export function invitationExpiresLabel(invitation: CloudInvitation): string {
  return invitationExpiryTime(invitation.expiresAt);
}

/** Hides one announcement for this installation and re-reads the dismissal list. */
export function dismissCloudAnnouncement(id: string): void {
  dismissAnnouncement(id);
  setCloudAnnouncementDismissed(dismissedAnnouncementIds());
}

/** Runs one sync tick immediately (the "Sync now" button). */
export async function syncNow(): Promise<void> {
  await tick();
}

export function startLoop(): void {
  if (loopTimer !== undefined) return;
  loopTimer = window.setInterval(() => {
    void tick();
  }, SYNC_INTERVAL_MS);
}

export function stopLoop(): void {
  if (loopTimer !== undefined) {
    window.clearInterval(loopTimer);
    loopTimer = undefined;
  }
}

/**
 * One sync pass: push local changes, then pull and apply server changes.
 *
 * Pushes are retried by the loop on retryable failures (the backend reports 429/5xx as
 * `retryable`); permanent failures surface through `cloudLastError`.
 */
async function tick(): Promise<void> {
  if (tickRunning) return;
  const session = cloudSession();
  if (!session.signedIn || !session.workspaceId) return;
  // Never touch rows while the model is streaming into them.
  if (isThinking()) return;

  tickRunning = true;
  setCloudStatus('syncing');
  try {
    const push = await cloudSyncPush();
    if (push.signedOut) {
      await handleSignedOut();
      return;
    }
    setCloudPending(push.pending);
    if (push.retryable) {
      setCloudStatus('offline');
      setCloudLastError(push.message);
      return;
    }
    await pullPages();
    setCloudStatus('idle');
    setCloudLastError(null);
  } catch (error) {
    setCloudStatus('error');
    setCloudLastError(String(error));
  } finally {
    tickRunning = false;
  }
}

async function pullPages(): Promise<void> {
  for (let page = 0; page < MAX_PULL_PAGES; page += 1) {
    const pull = await cloudSyncPull();
    if (pull.signedOut) {
      await handleSignedOut();
      return;
    }
    if (pull.cursorExpired) {
      await resyncSnapshot();
      return;
    }
    if (pull.status !== null) {
      setCloudLastError(pull.message);
      setCloudStatus(pull.retryable ? 'offline' : 'error');
      return;
    }
    if (pull.changes.length === 0) {
      if (pull.nextCursor) await cloudSyncApply([], pull.nextCursor);
      return;
    }
    await cloudSyncApply(pull.changes, pull.nextCursor);
    mergeRemoteChanges(pull.changes);
    await hydrateAttachments(pull.changes);
    if (!pull.hasMore) {
      await refreshCloudSession();
      return;
    }
  }
}

/** Full-resync recovery after an expired cursor: page through the snapshot, then reload. */
async function resyncSnapshot(): Promise<void> {
  let nextPage: string | null = null;
  for (let page = 0; page < MAX_PULL_PAGES * 20; page += 1) {
    const snapshot = await cloudSyncSnapshot(nextPage);
    if (snapshot.signedOut) {
      await handleSignedOut();
      return;
    }
    if (snapshot.status !== null) {
      setCloudLastError(snapshot.message);
      setCloudStatus(snapshot.retryable ? 'offline' : 'error');
      return;
    }
    nextPage = snapshot.nextPage;
    if (!snapshot.hasMore) {
      if (snapshot.snapshotCursor) await cloudSyncApply([], snapshot.snapshotCursor);
      // The snapshot may have removed remote-origin rows, so re-read local state once.
      const assistants = await loadAssistants();
      setDatas('assistants', assistants);
      await refreshCloudSession();
      return;
    }
  }
}

async function handleSignedOut(): Promise<void> {
  stopLoop();
  await cloudStopEvents().catch(() => undefined);
  setCloudStatus('signed_out');
  // Clear the binding too: a signed-out session must never keep ticking against it.
  setCloudSession({
    signedIn: false,
    serverUrl: null,
    workspaceId: null,
    userId: null,
    userLogin: null,
    deviceId: null,
    lastCursor: null,
    pending: 0,
  });
}

/**
 * Downloads attachments referenced by pulled messages that are not on this device yet.
 *
 * Runs after the store merge and patches `displayFiles` with the local path so images
 * render. A failure keeps the server reference; the next sync retries it.
 */
async function hydrateAttachments(changes: CloudRemoteChange[]): Promise<void> {
  let downloads = 0;
  for (const change of changes) {
    if (change.type !== 'message.upsert') continue;
    const payload = asRecord(change.payload);
    if (!payload) continue;
    const messageId = asString(payload.id);
    const threadId = asString(payload.threadId);
    if (!messageId || !threadId) continue;
    for (const ref of attachmentRefs(payload)) {
      if (downloads >= MAX_ATTACHMENT_DOWNLOADS) return;
      if (downloadedAttachments.has(ref.id) || hasLocalCopy(threadId, messageId, ref)) continue;
      try {
        const stored = await cloudAttachmentDownload(ref);
        downloadedAttachments.add(ref.id);
        downloads += 1;
        patchDisplayFile(threadId, messageId, stored.id, stored.storagePath);
      } catch {
        return;
      }
    }
  }
}

function hasLocalCopy(threadId: string, messageId: string, ref: CloudAttachmentRef): boolean {
  const message = findMessage(threadId, messageId);
  const file = message?.displayFiles?.find((candidate) => candidate.id === ref.id);
  return Boolean(file?.storagePath);
}

function findMessage(threadId: string, messageId: string): Message | undefined {
  for (const assistant of datas.assistants) {
    const topic = assistant.topics.find((candidate) => candidate.id === threadId);
    if (!topic) continue;
    return topic.history.find((message) => message.id === messageId);
  }
  return undefined;
}

function patchDisplayFile(
  threadId: string,
  messageId: string,
  attachmentId: string,
  storagePath: string,
): void {
  setDatas('assistants', (list) =>
    list.map((assistant) => {
      const topic = assistant.topics.find((candidate) => candidate.id === threadId);
      if (!topic) return assistant;
      const history = topic.history.map((message) => {
        if (message.id !== messageId) return message;
        const files = (message.displayFiles ?? []).map((file) =>
          file.id === attachmentId ? { ...file, storagePath } : file,
        );
        return files.length > 0 ? { ...message, displayFiles: files } : message;
      });
      const topics = assistant.topics.map((candidate) =>
        candidate.id === threadId ? { ...candidate, history } : candidate,
      );
      return { ...assistant, topics };
    }),
  );
}

/** Test helper: drop every cloud signal back to the signed-out state. */
export function resetCloudState(): void {
  stopLoop();
  stopDevicePoll();
  downloadedAttachments.clear();
  setCloudSession({
    signedIn: false,
    serverUrl: null,
    workspaceId: null,
    userId: null,
    userLogin: null,
    deviceId: null,
    lastCursor: null,
    pending: 0,
  });
  setCloudWorkspaces([]);
  setCloudStatus('signed_out');
  setCloudPending(0);
  setCloudLastError(null);
  clearDevicePrompt();
}

/** Payload kinds used by the lazy hydration helpers. */
export type CloudPayloadKind = 'agent_steps' | 'tool_full_result';

const payloadRequests = new Set<string>();

/**
 * Lazily hydrates one message payload (agent steps or full tool results).
 *
 * Called when a user expands an agent-process block: the sync feed strips these bulky
 * fields, so a device fetches them on demand. A missing payload (legacy message, another
 * device never uploaded it) is not an error — the legacy rendering stays.
 */
export async function hydrateMessagePayload(
  threadId: string,
  messageId: string,
  kind: CloudPayloadKind,
): Promise<void> {
  if (!cloudSession().signedIn) return;
  const key = `${messageId}:${kind}`;
  if (payloadRequests.has(key)) return;
  payloadRequests.add(key);
  try {
    const json = await cloudGetMessagePayload(messageId, kind);
    const parsed: unknown = JSON.parse(json);
    if (kind === 'agent_steps') {
      if (!Array.isArray(parsed)) return;
      patchMessage(threadId, messageId, (message) => ({
        ...message,
        agentSteps: parsed as Message['agentSteps'],
      }));
      return;
    }
    // Full tool results: merge each entry back into its tool call.
    if (!Array.isArray(parsed)) return;
    patchMessage(threadId, messageId, (message) => {
      const calls = message.toolCalls?.slice();
      if (!calls) return message;
      for (const entry of parsed) {
        const record = asRecord(entry);
        const id = asString(record?.id);
        const full = record?.fullContent;
        if (!id || typeof full !== 'string') continue;
        const index = calls.findIndex((call) => call.id === id);
        if (index >= 0) calls[index] = { ...calls[index], fullContent: full };
      }
      return { ...message, toolCalls: calls };
    });
  } catch {
    // Payload absent or offline: keep the legacy tool-call rendering.
  } finally {
    payloadRequests.delete(key);
  }
}

// ---------- cloud runs ----------

export interface CloudRunApproval {
  approvalId: string;
  toolName: string;
  toolCallId: string;
}

export interface CloudRunState {
  runId: string;
  threadId: string;
  status: 'queued' | 'running' | 'waiting_approval' | 'succeeded' | 'failed' | 'canceled';
  /** Streaming assistant text (deltas appended as they arrive). */
  text: string;
  approvals: CloudRunApproval[];
  error: string | null;
}

/** Active cloud run, or null when none is running in this window. */
export const [cloudRun, setCloudRun] = createSignal<CloudRunState | null>(null);

let runListenerInstalled = false;

async function ensureRunListener(): Promise<void> {
  if (runListenerInstalled) return;
  runListenerInstalled = true;
  await listen<CloudRunEvent>(EVENTS.cloudRunEvent, (event) => {
    applyRunEvent(event.payload);
  });
}

function applyRunEvent(event: CloudRunEvent): void {
  const current = cloudRun();
  if (!current || current.runId !== event.runId) return;
  const payload = event.payload;
  switch (event.type) {
    case 'message.delta': {
      const delta = typeof payload.delta === 'string' ? payload.delta : '';
      if (delta) setCloudRun({ ...current, status: 'running', text: current.text + delta });
      return;
    }
    case 'approval.requested': {
      const approvalId = typeof payload.approvalId === 'string' ? payload.approvalId : '';
      if (!approvalId) return;
      // The worker names the tool in `name` (AG-UI tool.call), not `toolName`.
      const toolName =
        typeof payload.name === 'string'
          ? payload.name
          : typeof payload.toolName === 'string'
            ? payload.toolName
            : 'tool';
      const toolCallId = typeof payload.toolCallId === 'string' ? payload.toolCallId : '';
      setCloudRun({
        ...current,
        status: 'waiting_approval',
        approvals: [
          ...current.approvals.filter((item) => item.approvalId !== approvalId),
          { approvalId, toolName, toolCallId },
        ],
      });
      return;
    }
    case 'approval.resolved': {
      const approvalId = typeof payload.approvalId === 'string' ? payload.approvalId : '';
      setCloudRun({
        ...current,
        status: 'running',
        approvals: current.approvals.filter((item) => item.approvalId !== approvalId),
      });
      return;
    }
    case 'run.failed': {
      const message = typeof payload.error === 'string' ? payload.error : null;
      setCloudRun({ ...current, status: 'failed', error: message });
      void cloudStopRunEvents().catch(() => undefined);
      return;
    }
    case 'run.canceled':
    case 'run.completed': {
      const status = event.type === 'run.canceled' ? 'canceled' : 'succeeded';
      setCloudRun({ ...current, status });
      void cloudStopRunEvents().catch(() => undefined);
      if (status === 'succeeded' && current.text.trim().length > 0) {
        // The desktop stays the writer of its own conversation: the server streams text,
        // and the finished reply is appended locally so it also syncs to other devices.
        void persistRunReply(current.threadId, current.text);
      }
      return;
    }
    default:
      if (event.type === 'run.started') setCloudRun({ ...current, status: 'running' });
  }
}

/** Starts a cloud run for the current thread and begins streaming its events. */
export async function startCloudRun(input: {
  threadId: string;
  assistantId: string;
  message: string;
  model?: string | null;
}): Promise<void> {
  if (!cloudSession().signedIn || !cloudSession().workspaceId) {
    throw new Error('cloud session required');
  }
  await ensureRunListener();
  const started = await cloudStartRun(
    input.threadId,
    input.assistantId,
    input.message,
    input.model ?? null,
  );
  setCloudRun({
    runId: started.runId,
    threadId: input.threadId,
    status: 'queued',
    text: '',
    approvals: [],
    error: null,
  });
  await cloudStartRunEvents(started.runId);
}

/** Cancels the active run and stops the local stream. */
export async function cancelCloudRun(): Promise<void> {
  const current = cloudRun();
  if (!current) return;
  await cloudCancelRun(current.runId).catch(() => undefined);
  await cloudStopRunEvents().catch(() => undefined);
  setCloudRun({ ...current, status: 'canceled' });
}

/** Approves or denies one pending tool call of the active run. */
export async function resolveCloudApproval(
  approvalId: string,
  decision: 'approve' | 'deny',
): Promise<void> {
  const current = cloudRun();
  if (!current) return;
  await cloudResolveRunApproval(current.runId, approvalId, decision);
  setCloudRun({
    ...current,
    approvals: current.approvals.filter((item) => item.approvalId !== approvalId),
    status: decision === 'approve' ? 'running' : current.status,
  });
}

/** Clears the finished run banner. */
export function dismissCloudRun(): void {
  setCloudRun(null);
}

/**
 * Appends a finished cloud reply to the local thread.
 *
 * Uses the regular `append_message` path so the message gets an id, syncs through the
 * outbox, and renders exactly like a locally produced reply.
 */
async function persistRunReply(threadId: string, text: string): Promise<void> {
  const messageId = crypto.randomUUID();
  try {
    await appendMessage(threadId, {
      id: messageId,
      role: 'assistant',
      content: text,
    } as Message);
    setDatas('assistants', (list) =>
      list.map((assistant) => {
        const topic = assistant.topics.find((candidate) => candidate.id === threadId);
        if (!topic) return assistant;
        const history = [
          ...topic.history,
          { id: messageId, role: 'assistant', content: text } as Message,
        ];
        const topics = assistant.topics.map((candidate) =>
          candidate.id === threadId ? { ...candidate, history } : candidate,
        );
        return { ...assistant, topics };
      }),
    );
  } catch (error) {
    setCloudLastError(String(error));
  }
}
