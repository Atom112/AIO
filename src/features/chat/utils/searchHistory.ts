/**
 * Chat history search (per-assistant scope).
 *
 * Scope design: each project owns exactly one assistant (`asst-${projectId}`,
 * created by ensureProjectAssistant) and pure-chat mode is the default chat
 * assistant — searching the CURRENT assistant's topics therefore equals
 * per-project isolation (one project can never surface another project's
 * records; pure chat acts as its own single "project"). The full history is
 * eager-loaded into the store at startup, so matching runs entirely in-memory
 * — no backend command, no IPC latency.
 */

import type { Assistant, Message, Topic } from '../../../core/store/types';

export interface ChatSearchMatch {
  topicId: string;
  topicName: string;
  messageId: string;
  role: Message['role'];
  /** Snippet around the first hit: `{ before, hit, after }` (the caller highlights `hit`) */
  snippet: { before: string; hit: string; after: string };
}

/** Result-list cap: the dropdown stays browsable even for hot terms in huge histories */
const MAX_MATCHES = 50;
const SNIPPET_BEFORE = 18;
const SNIPPET_AFTER = 42;

/**
 * The text a match runs against: `displayText` (the user-message display
 * override) when present, else raw `content`. Multimodal array content without
 * a displayText override has no searchable text (image-only parts) and is
 * skipped by the string check.
 */
const effectiveText = (msg: Message): string => {
  const t = msg.displayText !== undefined ? msg.displayText : msg.content;
  return typeof t === 'string' ? t : '';
};

/** UTF-16-safe slice: backs off one char when a boundary would split a surrogate pair (emoji). */
const safeSlice = (text: string, from: number, to: number): string => {
  let start = Math.max(0, Math.min(from, text.length));
  let end = Math.max(start, Math.min(to, text.length));
  // start sits between a high surrogate and its low partner → keep the pair on the left
  if (start > 0 && text.charCodeAt(start) >= 0xdc00 && text.charCodeAt(start) <= 0xdfff) {
    start -= 1;
  }
  // end sits right after a high surrogate → extend to include the low partner
  if (end < text.length && text.charCodeAt(end) >= 0xdc00 && text.charCodeAt(end) <= 0xdfff) {
    end += 1;
  }
  return text.slice(start, end);
};

/** Case-insensitive substring search over one assistant's topics (chat + user messages; tool rows are hidden in the UI and skipped). */
export function searchChatHistory(assistant: Assistant, query: string): ChatSearchMatch[] {
  const q = query.trim().toLowerCase();
  if (!q || !assistant) return [];
  const out: ChatSearchMatch[] = [];
  const topics: Topic[] = assistant.topics ?? [];
  for (const topic of topics) {
    for (const msg of topic.history ?? []) {
      if (out.length >= MAX_MATCHES) return out;
      // Skip rows the UI never renders: hidden tool results and id-less transients (nothing to jump to)
      if (!msg.id || msg.role === 'tool') continue;
      const text = effectiveText(msg);
      if (!text) continue;
      const at = text.toLowerCase().indexOf(q);
      if (at < 0) continue;
      const hitEnd = at + q.length;
      out.push({
        topicId: topic.id,
        topicName: topic.name,
        messageId: msg.id,
        role: msg.role,
        snippet: {
          before:
            at > 0
              ? at > SNIPPET_BEFORE
                ? `…${safeSlice(text, at - SNIPPET_BEFORE, at)}`
                : safeSlice(text, 0, at)
              : '',
          hit: safeSlice(text, at, hitEnd),
          after:
            hitEnd < text.length
              ? hitEnd + SNIPPET_AFTER < text.length
                ? `${safeSlice(text, hitEnd, hitEnd + SNIPPET_AFTER)}…`
                : safeSlice(text, hitEnd, text.length)
              : '',
        },
      });
    }
  }
  return out;
}
