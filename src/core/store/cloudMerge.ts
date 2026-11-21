// Store merge for pulled cloud changes.
//
// Pulled payloads are applied to the `datas` store by id instead of reloading every
// assistant: the sync feed already carries the authoritative payload, and a full reload
// would drop in-progress local edits and remount the chat list.
import {
  asNumber,
  asRecord,
  asString,
  asStringList,
  messageFromPayload,
} from './cloudMergeHelpers';
import { setDatas } from './chat';
import type { Assistant, AttachmentMeta, Topic } from './types';
import type { CloudRemoteChange } from '../api/cloud';

// ---------- store merge ----------

export function mergeRemoteChanges(changes: CloudRemoteChange[]): void {
  for (const change of changes) {
    const payload = asRecord(change.payload);
    switch (change.type) {
      case 'assistant.upsert':
        if (payload) upsertAssistant(payload);
        break;
      case 'thread.upsert':
        if (payload) upsertTopic(payload);
        break;
      case 'message.upsert':
        if (payload) upsertMessage(payload);
        break;
      case 'assistant.delete':
        setDatas('assistants', (list) => list.filter((assistant) => assistant.id !== change.id));
        break;
      case 'thread.delete':
        setDatas('assistants', (list) =>
          list.map((assistant) => ({
            ...assistant,
            topics: assistant.topics.filter((topic) => topic.id !== change.id),
          })),
        );
        break;
      case 'message.delete':
        setDatas('assistants', (list) =>
          list.map((assistant) => ({
            ...assistant,
            topics: assistant.topics.map((topic) => ({
              ...topic,
              history: topic.history.filter((message) => message.id !== change.id),
            })),
          })),
        );
        break;
      default:
        break;
    }
  }
}

function upsertAssistant(payload: Record<string, unknown>): void {
  const id = asString(payload.id);
  if (!id) return;
  const name = asString(payload.name) ?? 'Assistant';
  const core = {
    name,
    prompt: asString(payload.prompt),
    modelId: asString(payload.modelId),
    mcpServerIds: asStringList(payload.mcpServerIds),
    skillIds: asStringList(payload.skillIds),
    projectId: asString(payload.projectId),
    agentMode: asString(payload.agentMode),
    assistantType: asString(payload.assistantType),
  };
  setDatas('assistants', (list) => {
    const index = list.findIndex((assistant) => assistant.id === id);
    if (index < 0) {
      const created = { id, ...core, name, topics: [] } as unknown as Assistant;
      return [...list, created];
    }
    const next = list.slice();
    next[index] = { ...next[index], ...core, name } as Assistant;
    return next;
  });
}

function upsertTopic(payload: Record<string, unknown>): void {
  const id = asString(payload.id);
  const assistantId = asString(payload.assistantId);
  if (!id || !assistantId) return;
  const core = {
    name: asString(payload.title) ?? asString(payload.summary) ?? 'Topic',
    summary: asString(payload.summary),
    branchedFromMessageId: asString(payload.branchedFromMessageId),
    parentTopicId: asString(payload.parentThreadId),
    contextTrimCount: asNumber(payload.contextTrimCount) ?? 0,
  };
  setDatas('assistants', (list) =>
    list.map((assistant) => {
      if (assistant.id !== assistantId) return assistant;
      const topics = assistant.topics.slice();
      const index = topics.findIndex((topic) => topic.id === id);
      if (index < 0) {
        topics.push({ id, history: [], ...core } as Topic);
      } else {
        topics[index] = { ...topics[index], ...core } as Topic;
      }
      return { ...assistant, topics };
    }),
  );
}

function upsertMessage(payload: Record<string, unknown>): void {
  const message = messageFromPayload(payload);
  const threadId = asString(payload.threadId);
  if (!message || !threadId) return;
  setDatas('assistants', (list) =>
    list.map((assistant) => {
      const topic = assistant.topics.find((candidate) => candidate.id === threadId);
      if (!topic) return assistant;
      const history = topic.history.slice();
      const index = history.findIndex((candidate) => candidate.id === message.id);
      if (index >= 0) {
        // Local-only fields the sync DTO does not carry stay untouched.
        const previous = history[index];
        history[index] = {
          ...message,
          agentSteps: previous.agentSteps,
          images: previous.images,
          displayFiles: mergeDisplayFiles(previous.displayFiles, message.displayFiles),
        };
      } else {
        // Pull returns changes in server order, so appending keeps per-thread order;
        // the next reload re-orders by the server timestamp stored in SQLite.
        history.push(message);
      }
      const topics = assistant.topics.map((candidate) =>
        candidate.id === threadId ? { ...candidate, history } : candidate,
      );
      return { ...assistant, topics };
    }),
  );
}

/** Keeps a locally downloaded file path when the server ref only has metadata. */
function mergeDisplayFiles(
  previous: AttachmentMeta[] | undefined,
  incoming: AttachmentMeta[] | undefined,
): AttachmentMeta[] | undefined {
  if (!incoming) return previous;
  if (!previous) return incoming;
  return incoming.map((file) => {
    const match = previous.find((candidate) => candidate.id === file.id);
    return match?.storagePath ? { ...file, storagePath: match.storagePath } : file;
  });
}
