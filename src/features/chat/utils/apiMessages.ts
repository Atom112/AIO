/**
 * API payload message building (P3/阶段4 第2项拆分切片：ChatPage <900 程序).
 *
 * Verbatim extraction of the three pure payload builders that lived inline in
 * ChatPage — no component state involved, so they land in a plain util module
 * and become unit-testable (the pairing guard mirrors the backend's
 * `verify_tool_messages`, which only logs; this one repairs).
 */

import { t } from '../../../core/i18n';
import { TOOL_INTERRUPTED } from '../constants';

/**
 * Truncate oversized tool results (head+tail retention, aligned with the backend truncate_tool_result:
 * 60% head + 40% tail, 20k chars), preventing LLM context bloat while preserving the file head/tail
 * that read_file already kept. Full content stays in agentSteps for the user.
 */
export const truncateToolResult = (content: string): string => {
  const MAX_LEN = 20000;
  if (content.length <= MAX_LEN) return content;
  const head = Math.floor((MAX_LEN * 6) / 10);
  const tail = MAX_LEN - head;
  const headEnd = head;
  const tailStart = content.length - tail;
  if (tailStart > headEnd) {
    return (
      content.slice(0, headEnd) +
      `\n\n${t('chat.truncatedHeadTail', { total: content.length })}\n\n` +
      content.slice(tailStart)
    );
  }
  return content.slice(0, MAX_LEN) + `\n\n${t('chat.truncatedTail', { total: content.length })}`;
};

/**
 * Expand one history message into API payload messages (one or more: assistant + its tool results).
 *
 * Key defense: orphan toolCalls stuck in state 'calling' (user interrupt / app killed / legacy data)
 * are synthesized in place as interrupted-error tool results so every tool_calls has a paired
 * role:tool response; otherwise strictly-validated APIs return 400 (missing field
 * tool_call_id / insufficient tool messages).
 */
export function buildApiMessages(m: any): any[] {
  const obj: any = { role: m.role, content: m.content };
  if (m.images?.length) obj.images = m.images; // the backend strips the aio-image marker from this and expands image_url blocks
  if (m.displayFiles?.length) obj.displayFiles = m.displayFiles; // attachments (incl. images); the backend reads these and expands image_url blocks
  if (m.toolCallId) obj.toolCallId = m.toolCallId;
  if (m.name) obj.name = m.name;
  if (!m.toolCalls || m.toolCalls.length === 0) return [obj];

  // entries without an ID cannot be paired; drop them (avoids JSON.stringify dropping undefined fields and causing 400)
  const validCalls = m.toolCalls.filter((tc: any) => tc.id != null);
  if (validCalls.length === 0) return [obj];

  obj.toolCalls = validCalls.map((tc: any) => ({
    id: tc.id,
    type: tc.type || 'function',
    function: { name: tc.function?.name, arguments: tc.function?.arguments },
  }));
  const toolMsgs = validCalls.map((tc: any) => {
    // orphan (calling / no state): treat as interrupted, synthesize an error result
    const interrupted = tc.state !== 'success' && tc.state !== 'error';
    const raw = interrupted
      ? TOOL_INTERRUPTED
      : tc.state === 'error'
        ? (tc.error ?? tc.content ?? '[Tool Error]')
        : (tc.content ?? '');
    return {
      role: 'tool' as const,
      toolCallId: tc.id,
      name: tc.function?.name,
      content: truncateToolResult(raw),
    };
  });
  return [obj, ...toolMsgs];
}

/**
 * Ensure every assistant(tool_calls) in messagesForAI has matching role:tool messages.
 * If the tool response for a tool_call_id is missing, auto-fill a placeholder message
 * to prevent the API 400 "insufficient tool messages" error.
 */
export function ensureToolMessagesComplete(msgs: any[]): any[] {
  const result: any[] = [];
  let pendingToolCallIds: string[] = [];

  // fill placeholder tool messages for all unpaired toolCallIds in pending
  const flushPending = (target: any[]) => {
    for (const id of pendingToolCallIds) {
      console.warn(`[ensureToolMessages] patched missing tool response: ${id}`);
      target.push({
        role: 'tool' as const,
        toolCallId: id,
        name: '__pending__',
        content: '[System: tool result pending]',
      });
    }
    pendingToolCallIds = [];
  };

  for (const msg of msgs) {
    // on a new assistant(toolCalls), flush the previous unpaired toolCallIds first so the
    // previous pending set is not silently overwritten across consecutive tool-call turns
    if (msg.role === 'assistant' && msg.toolCalls?.length && pendingToolCallIds.length > 0) {
      flushPending(result);
    }

    result.push(msg);

    if (msg.role === 'assistant' && msg.toolCalls?.length) {
      // collect all toolCallIds declared by this assistant message
      pendingToolCallIds = msg.toolCalls.map((tc: any) => tc.id);
    } else if (msg.role === 'tool' && msg.toolCallId) {
      // a matching tool response was found; remove it
      const idx = pendingToolCallIds.indexOf(msg.toolCallId);
      if (idx !== -1) pendingToolCallIds.splice(idx, 1);
    }

    // on a user or assistant (no toolCalls) message, any unmatched toolCallId means the
    // previous assistant toolCalls lack sufficient tool responses — fill placeholder messages
    if (
      (msg.role === 'user' || (msg.role === 'assistant' && !msg.toolCalls?.length)) &&
      pendingToolCallIds.length > 0
    ) {
      flushPending(result);
    }
  }

  // final flush: fill placeholders for toolCallIds still unmatched at the end of the array
  if (pendingToolCallIds.length > 0) flushPending(result);

  return result;
}
