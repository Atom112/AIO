// Payload narrowing helpers shared by the cloud store and its merge module.
//
// Sync payloads are untyped JSON from the server: every field is narrowed here once, so
// the store and the merge code never guess at shapes.
import { setDatas } from './chat';
import type { CloudAttachmentRef } from '../api/cloud';
import type { AttachmentMeta, Message } from './types';

export function asRecord(value: unknown): Record<string, unknown> | null {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return null;
  // Narrowed to a non-array object above; the payload contract is a plain object record.
  return Object.fromEntries(Object.entries(value));
}

export function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined;
}

export function asNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}

export function asStringList(value: unknown): string[] | undefined {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === 'string')
    : undefined;
}

export function attachmentRefs(payload: Record<string, unknown>): CloudAttachmentRef[] {
  const raw = payload.attachments;
  if (!Array.isArray(raw)) return [];
  const refs: CloudAttachmentRef[] = [];
  for (const item of raw) {
    const record = asRecord(item);
    const id = asString(record?.id);
    if (!record || !id) continue;
    refs.push({
      id,
      fileName: asString(record.fileName) ?? 'attachment',
      mimeType: asString(record.mimeType) ?? 'application/octet-stream',
      sha256: asString(record.sha256) ?? '',
      sizeBytes: asNumber(record.sizeBytes) ?? 0,
    });
  }
  return refs;
}

export function messageFromPayload(payload: Record<string, unknown>): Message | null {
  const id = asString(payload.id);
  const threadId = asString(payload.threadId);
  if (!id || !threadId) return null;
  const refs = attachmentRefs(payload);
  const displayFiles: AttachmentMeta[] | undefined =
    refs.length > 0
      ? refs.map((ref) => ({
          id: ref.id,
          name: ref.fileName,
          mimeType: ref.mimeType,
          size: ref.sizeBytes,
        }))
      : undefined;
  const rawContent = payload.content;
  const content: Message['content'] =
    typeof rawContent === 'string' || Array.isArray(rawContent)
      ? (rawContent as Message['content'])
      : JSON.stringify(rawContent ?? '');
  return {
    id,
    role: (asString(payload.role) as Message['role']) ?? 'assistant',
    content,
    modelId: asString(payload.modelId),
    displayFiles,
    displayText: asString(payload.displayText),
    reasoning: asString(payload.reasoning),
    toolCalls: Array.isArray(payload.toolCalls)
      ? (payload.toolCalls as Message['toolCalls'])
      : undefined,
    parentMessageId: asString(payload.parentMessageId),
    branchIndex: asNumber(payload.branchIndex) ?? 0,
    inputTokens: asNumber(payload.inputTokens),
    outputTokens: asNumber(payload.outputTokens),
    cachedInputTokens: asNumber(payload.cachedInputTokens),
    contextTokens: asNumber(payload.contextTokens),
    ttftMs: asNumber(payload.ttftMs),
  };
}

/** Merges pulled changes into the `datas` store by id (no full reload). */

/** Applies `update` to one message inside the `datas` store. */
export function patchMessage(
  threadId: string,
  messageId: string,
  update: (message: Message) => Message,
): void {
  setDatas('assistants', (list) =>
    list.map((assistant) => {
      const topic = assistant.topics.find((candidate) => candidate.id === threadId);
      if (!topic) return assistant;
      const history = topic.history.map((message) =>
        message.id === messageId ? update(message) : message,
      );
      const topics = assistant.topics.map((candidate) =>
        candidate.id === threadId ? { ...candidate, history } : candidate,
      );
      return { ...assistant, topics };
    }),
  );
}
