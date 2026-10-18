/**
 * Harness workbench stream listeners — registered once from the root Layout,
 * alive for the whole app (harness agents keep running across page navigation).
 *
 * Mirrors the chat listeners' responsibilities, adapted to the harness model:
 * - Text / thinking deltas accumulate into per-topic draft messages (80ms batch
 *   flush, same anti-storm rationale as the chat pipeline);
 * - Tool call events upsert `AgentStep` entries by tool call id so
 *   AgentProcessBlock renders them with the native chat visuals;
 * - Turn end finalizes the draft into a persisted assistant Message;
 * - Approvals / run states / plan entries land in the harness store.
 */
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { EVENTS } from '../../../core/api/events';
import type {
  HarnessApprovalExpiredPayload,
  HarnessApprovalRequestedPayload,
  HarnessEventPayload,
  HarnessStatePayload,
} from '../../../core/api/harnessPayloads';
import { harnessAppendMessage } from '../../../core/api/harness';
import type { AgentStep, Message, ToolCallDisplay } from '../../../core/store/types';
import type { HarnessDraft } from '../../../core/store/harness';
import {
  currentHarnessTopicId,
  harnessApprovals,
  harnessDrafts,
  harnessMessages,
  harnessPlan,
  harnessRunStates,
  harnessTools,
  harnessTopics,
  harnessTurnErrors,
  setHarnessApprovals,
  saveHarnessConfigCacheEntry,
  setHarnessConfig,
  setHarnessDrafts,
  setHarnessMessages,
  setHarnessPlan,
  setHarnessRunStates,
  setHarnessTopics,
  setHarnessTurnErrors,
} from '../../../core/store/harness';
import { finishBackgroundTask, startBackgroundTask } from '../../../core/store/background-tasks';

/** Stream batch per session row id (content + reasoning deltas, 80ms flush). */
interface StreamBatch {
  sessionRowId: string;
  content: string;
  reasoning: string;
  timer: number;
}

const batches = new Map<string, StreamBatch>();
const FLUSH_MS = 80;

const draftOf = (sessionRowId: string) => harnessDrafts()[sessionRowId];

const upsertDraft = (sessionRowId: string, patch: (d: HarnessDraft) => void) => {
  const drafts = { ...harnessDrafts() };
  const draft = drafts[sessionRowId] ?? {
    content: '',
    reasoning: '',
    agentSteps: [],
    usage: {},
    startedAt: Date.now(),
  };
  patch(draft);
  drafts[sessionRowId] = draft;
  setHarnessDrafts(drafts);
};

/** Upsert a tool_call step by tool call id (chat AgentStep shape). */
const upsertToolStep = (
  draft: HarnessDraft,
  toolCallId: string,
  patchStep: (step: AgentStep, toolCall: ToolCallDisplay) => void,
  create?: {
    name?: string;
    title?: string;
    arguments?: string;
  },
) => {
  const idx = draft.agentSteps.findIndex(
    (s) => s.type === 'tool_call' && s.toolCall?.id === toolCallId,
  );
  const now = Date.now();
  if (idx >= 0) {
    const steps = [...draft.agentSteps];
    const step = { ...steps[idx] };
    const toolCall = { ...(step.toolCall as ToolCallDisplay) };
    patchStep(step, toolCall);
    step.toolCall = toolCall;
    steps[idx] = step;
    draft.agentSteps = steps;
    return;
  }
  const toolCall: ToolCallDisplay = {
    id: toolCallId,
    type: 'function',
    function: {
      name: create?.name || create?.title || 'tool',
      arguments: create?.arguments || '{}',
    },
    state: 'calling',
  };
  const step: AgentStep = {
    id: crypto.randomUUID(),
    type: 'tool_call',
    timestamp: now,
    status: 'running',
    toolCall,
  };
  patchStep(step, toolCall);
  draft.agentSteps = [...draft.agentSteps, step];
};

/** Append a completed content step (notices / interim info). */
const pushContentStep = (draft: HarnessDraft, text: string) => {
  draft.agentSteps = [
    ...draft.agentSteps,
    {
      id: crypto.randomUUID(),
      type: 'content',
      timestamp: Date.now(),
      status: 'complete',
      contentText: text,
    },
  ];
};

const flushBatch = (key: string) => {
  const batch = batches.get(key);
  if (!batch) return;
  batches.delete(key);
  window.clearTimeout(batch.timer);
  const draft = draftOf(batch.sessionRowId);
  if (!draft) return;
  const next = {
    ...draft,
    content: draft.content + batch.content,
    reasoning: draft.reasoning + batch.reasoning,
  };
  setHarnessDrafts({ ...harnessDrafts(), [batch.sessionRowId]: next });
};

const pushDelta = (sessionRowId: string, piece: 'content' | 'reasoning', text: string) => {
  let batch = batches.get(sessionRowId);
  if (!batch) {
    batch = { sessionRowId, content: '', reasoning: '', timer: 0 };
    batches.set(sessionRowId, batch);
  }
  if (piece === 'content') batch.content += text;
  else batch.reasoning += text;
  window.clearTimeout(batch.timer);
  batch.timer = window.setTimeout(() => flushBatch(sessionRowId), FLUSH_MS);
};

// ===== M-5: tool_call_update coalescing =====
//
// Progressive tool-output updates arrive in bursts per tool call (each carrying
// the FULL current content, so the last one wins). Unbatched, every update paid
// a full draft-object clone + steps-array copy + a store write — the same
// per-token amplification the text batches above already solved. Coalesce to
// the LATEST update per (session, toolCallId) and apply once per 80ms window.
// tool_call_end (final) and turn finalize flush synchronously first, so the
// end's own content always lands after (and wins over) any pending update.

interface ToolUpdateBatch {
  text: string;
  name?: string;
  title?: string;
  timer: number;
}

const toolUpdates = new Map<string, ToolUpdateBatch>();

const toolUpdateKey = (sid: string, toolCallId: string) => `${sid}::${toolCallId}`;

const flushToolUpdate = (key: string) => {
  const entry = toolUpdates.get(key);
  if (!entry) return;
  toolUpdates.delete(key);
  window.clearTimeout(entry.timer);
  const sep = key.indexOf('::');
  const sid = key.slice(0, sep);
  const toolCallId = key.slice(sep + 2);
  upsertDraft(sid, (draft) =>
    upsertToolStep(
      draft,
      toolCallId,
      (_step, toolCall) => {
        if (entry.text) toolCall.content = entry.text;
      },
      { name: entry.name, title: entry.title },
    ),
  );
};

/** Flush every pending update of one session (turn finalize / error path). */
const flushToolUpdatesFor = (sid: string) => {
  for (const key of [...toolUpdates.keys()]) {
    if (key.startsWith(`${sid}::`)) flushToolUpdate(key);
  }
};

/** Diff text → "+N -M" summary for FileChangeInfo. */
const diffSummary = (diff: string): string => {
  let added = 0;
  let removed = 0;
  for (const line of diff.split('\n')) {
    if (line.startsWith('+') && !line.startsWith('+++')) added++;
    else if (line.startsWith('-') && !line.startsWith('---')) removed++;
  }
  return `+${added} -${removed}`;
};

/** Ensure a draft exists for a new turn (resets any stale draft). */
const startDraft = (sessionRowId: string) => {
  setHarnessTurnErrors({ ...harnessTurnErrors(), [sessionRowId]: undefined });
  setHarnessDrafts({
    ...harnessDrafts(),
    [sessionRowId]: {
      content: '',
      reasoning: '',
      agentSteps: [],
      usage: {},
      startedAt: Date.now(),
    },
  });
};

/** Finalize the draft into a persisted assistant message. */
const finalizeTurn = async (sessionRowId: string, error?: string) => {
  flushBatch(sessionRowId);
  // M-5: pending tool-output updates must land before the draft is frozen.
  flushToolUpdatesFor(sessionRowId);
  const draft = draftOf(sessionRowId);
  const drafts = { ...harnessDrafts() };
  delete drafts[sessionRowId];
  setHarnessDrafts(drafts);
  if (error) {
    setHarnessTurnErrors({ ...harnessTurnErrors(), [sessionRowId]: error });
  }
  if (!draft || (!draft.content && !draft.reasoning && draft.agentSteps.length === 0)) {
    return; // nothing to persist (e.g. instant failure)
  }
  const message: Message = {
    id: crypto.randomUUID(),
    role: 'assistant',
    content: draft.content,
    reasoning: draft.reasoning || undefined,
    agentSteps: draft.agentSteps,
    inputTokens: draft.usage.inputTokens,
    outputTokens: draft.usage.outputTokens,
    contextTokens: draft.usage.contextTokens,
    agentStartTime: draft.startedAt,
  };
  // Append locally only when the finished topic is on screen (background
  // topics get their history on next selection via harnessLoadMessages).
  if (sessionRowId === currentHarnessTopicId()) {
    setHarnessMessages([...harnessMessages(), message]);
  }
  try {
    await harnessAppendMessage(sessionRowId, message);
  } catch (e) {
    console.error('[harness] persist assistant message failed:', e);
  }
};

const handleHarnessEvent = (payload: HarnessEventPayload) => {
  const { session_row_id: sid, event } = payload;
  switch (event.kind) {
    case 'session_info': {
      // Attach the harness-side session id to the topic row (display + resume).
      setHarnessTopics(
        harnessTopics().map((t) =>
          t.id === sid && event.harnessSessionId && t.harnessSessionId !== event.harnessSessionId
            ? { ...t, harnessSessionId: event.harnessSessionId }
            : t,
        ),
      );
      break;
    }
    case 'config_options': {
      // The agent's full current config-option set (model / thought-level
      // selectors, toggles) — replaces any previous report; also persists a
      // per-tool snapshot so the controls render eagerly after a restart.
      setHarnessConfig((prev) => ({ ...prev, [sid]: event.options ?? [] }));
      saveHarnessConfigCacheEntry(payload.tool_id, event.options ?? []);
      break;
    }
    case 'turn_started':
      startDraft(sid);
      break;
    case 'text_delta':
      pushDelta(sid, 'content', event.text ?? '');
      break;
    case 'thinking_delta':
      pushDelta(sid, 'reasoning', event.text ?? '');
      break;
    case 'tool_call_start':
      upsertDraft(sid, (draft) =>
        upsertToolStep(draft, event.toolCallId ?? '', () => undefined, {
          name: event.name,
          title: event.title ?? event.name,
          arguments: event.arguments,
        }),
      );
      break;
    case 'tool_call_update': {
      // M-5: coalesce progressive updates — keep the latest per (session, tool
      // call); the flush applies it once per 80ms window instead of per event.
      const toolCallId = event.toolCallId ?? '';
      if (!toolCallId) break;
      const key = toolUpdateKey(sid, toolCallId);
      const text = event.content ?? event.rawOutput;
      const prev = toolUpdates.get(key);
      if (text == null && !event.name && !event.title) break;
      if (prev) window.clearTimeout(prev.timer);
      toolUpdates.set(key, {
        text: text ?? prev?.text ?? '',
        name: event.name ?? prev?.name,
        title: event.title ?? prev?.title,
        timer: window.setTimeout(() => flushToolUpdate(key), FLUSH_MS),
      });
      break;
    }
    case 'tool_call_end':
      // Final state — flush any pending update for this tool first so the end's
      // content (the later, authoritative one) lands after it.
      flushToolUpdate(toolUpdateKey(sid, event.toolCallId ?? ''));
      upsertDraft(sid, (draft) =>
        upsertToolStep(
          draft,
          event.toolCallId ?? '',
          (step, toolCall) => {
            step.status = event.ok ? 'complete' : 'error';
            step.duration = Date.now() - step.timestamp;
            toolCall.state = event.ok ? 'success' : 'error';
            const text = event.content ?? event.rawOutput;
            if (text) {
              toolCall.content = text;
              toolCall.fullContent = text;
            }
            if (event.error) toolCall.error = event.error;
            if (event.fileChanges && event.fileChanges.length > 0) {
              toolCall.fileChanges = event.fileChanges.map((fc) => ({
                filePath: fc.filePath,
                action: fc.action as 'create' | 'modify' | 'delete',
                diff: fc.diff,
                summary: diffSummary(fc.diff),
              }));
            }
          },
          { name: event.name, title: event.title },
        ),
      );
      break;
    case 'plan_update':
      setHarnessPlan({ ...harnessPlan(), [sid]: event.entries ?? [] });
      break;
    case 'subagent_delta':
      upsertDraft(sid, (draft) =>
        pushContentStep(draft, `${event.name ? `[${event.name}] ` : ''}${event.text ?? ''}`),
      );
      break;
    case 'usage_update':
      upsertDraft(sid, (draft) => {
        if (event.usage) draft.usage = { ...draft.usage, ...event.usage };
      });
      break;
    case 'notice':
      upsertDraft(sid, (draft) => pushContentStep(draft, event.message ?? ''));
      break;
    case 'error': {
      // Driver-level error: surface as the topic's turn error + idle.
      setHarnessTurnErrors({ ...harnessTurnErrors(), [sid]: event.message });
      break;
    }
    case 'turn_ended':
      void finalizeTurn(sid, event.error);
      break;
  }
};

/**
 * Register the harness stream listeners. Returns the unlisten promises; the
 * Layout never unmounts so they live for the whole app.
 */
export function registerHarnessEventListeners(): Promise<UnlistenFn>[] {
  const pEvent = listen<HarnessEventPayload>(EVENTS.harnessEvent, (e) =>
    handleHarnessEvent(e.payload),
  );
  const pState = listen<HarnessStatePayload>(EVENTS.harnessState, (e) => {
    const { tool_id: toolId, session_row_id: sid, state, error } = e.payload;
    const next = { ...harnessRunStates() };
    next[sid] = { state, error };
    setHarnessRunStates(next);
    if (state === 'error' && error) {
      setHarnessTurnErrors({ ...harnessTurnErrors(), [sid]: error });
    }
    // Background-task menu linkage: a running CLI agent is an app background
    // job visible from any page (same registry as downloads/index jobs).
    const toolName = harnessTools().find((x) => x.id === toolId)?.name ?? toolId;
    const topicTitle = harnessTopics().find((x) => x.id === sid)?.title ?? sid;
    const scope = `${toolName}/${topicTitle}`;
    if (state === 'running') {
      startBackgroundTask('harness', scope, topicTitle);
    } else if (state === 'error') {
      finishBackgroundTask('harness', scope, 'error', error);
    } else if (state === 'idle') {
      finishBackgroundTask('harness', scope, 'done');
    }
  });
  const pApproval = listen<HarnessApprovalRequestedPayload>(
    EVENTS.harnessApprovalRequested,
    (e) => {
      const { tool_id, session_row_id, request } = e.payload;
      setHarnessApprovals([
        ...harnessApprovals(),
        { ...request, toolId: tool_id, sessionRowId: session_row_id },
      ]);
    },
  );
  const pExpired = listen<HarnessApprovalExpiredPayload>(EVENTS.harnessApprovalExpired, (e) => {
    setHarnessApprovals(harnessApprovals().filter((a) => a.requestId !== e.payload.request_id));
  });
  return [pEvent, pState, pApproval, pExpired];
}
