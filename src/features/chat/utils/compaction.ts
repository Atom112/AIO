/**
 * Conversation serialization for context-compaction summarization, following pi's
 * design (packages/coding-agent compaction): messages become plain-text turns so the
 * summarizer cannot treat them as a conversation to continue, thinking is omitted
 * (replayed reasoning is not part of the model's live context), and tool results are
 * truncated to a fixed budget (they are typically the largest context contributors).
 */

/** Per-tool-result character budget in the serialized summarizer input (pi: 2000) */
export const TOOL_RESULT_TRUNCATE_CHARS = 2000;

/** Whole-serialized-text budget: head user message + tail keep, middle omitted */
const SERIALIZE_TOTAL_CHAR_CAP = 300_000;

/** Minimal structural view of a history message for serialization (Message-compatible) */
interface SummarizableMessage {
  role?: string;
  content?: unknown;
  displayText?: string;
  name?: string;
  toolCalls?: Array<{ function?: { name?: string; arguments?: string }; name?: string }>;
}

/** Minimal multimodal content part view */
interface SummarizablePart {
  type?: string;
  text?: unknown;
}

/** Truncate on a char boundary with an omitted-bytes marker (head keep) */
const truncateHead = (text: string, max: number): string => {
  if (text.length <= max) return text;
  return `${text.slice(0, max)}\n[... 已截断 ${text.length - max} 字符 / ${text.length - max} chars truncated ...]`;
};

/** Extract plain text from a message content (string or multimodal part array) */
const contentText = (content: unknown): string => {
  if (typeof content === 'string') return content;
  if (Array.isArray(content)) {
    return (content as SummarizablePart[])
      .map((p) =>
        p?.type === 'text' ? String(p.text ?? '') : p?.type === 'image_url' ? '[image]' : '',
      )
      .filter(Boolean)
      .join('\n');
  }
  return content == null ? '' : String(content);
};

/**
 * Serialize messages into pi-style plain text:
 *   [User]: ...
 *   [Assistant]: ...
 *   [Assistant tool calls]: name(args); name2(args)
 *   [Tool result]: ... (truncated)
 * Oversized conversations keep the head user message (original task) plus the most
 * recent tail within SERIALIZE_TOTAL_CHAR_CAP, with an omission marker in between.
 */
export function serializeConversationForSummary(
  messages: SummarizableMessage[] | null | undefined,
): string {
  const blocks = (messages ?? []).map((m) => {
    const role: string = m?.role ?? '?';
    if (role === 'assistant' && Array.isArray(m?.toolCalls) && m.toolCalls.length > 0) {
      const calls = m.toolCalls
        .map((tc) => {
          const name = tc?.function?.name ?? tc?.name ?? '?';
          const args = String(tc?.function?.arguments ?? '');
          return `${name}(${truncateHead(args, 200)})`;
        })
        .join('; ');
      const text = contentText(m.content) || m.displayText || '';
      const header = `[Assistant tool calls]: ${calls}`;
      return text.trim()
        ? `${header}\n[Assistant]: ${truncateHead(text, TOOL_RESULT_TRUNCATE_CHARS)}`
        : header;
    }
    const text = contentText(m.content) || m.displayText || '';
    const label =
      role === 'user'
        ? '[User]'
        : role === 'assistant'
          ? '[Assistant]'
          : role === 'tool'
            ? `[Tool result${m?.name ? ` (${m.name})` : ''}]`
            : `[${role}]`;
    return `${label}: ${truncateHead(text, TOOL_RESULT_TRUNCATE_CHARS)}`;
  });

  const joined = blocks.join('\n\n');
  if (joined.length <= SERIALIZE_TOTAL_CHAR_CAP) return joined;

  // Head user message (original task) + recent tail, middle omitted
  const headIdx = blocks.findIndex((b) => b.startsWith('[User]:'));
  const head = headIdx >= 0 ? `${blocks[headIdx]}\n\n` : '';
  const acc: string[] = [];
  let total = head.length;
  for (let i = blocks.length - 1; i >= 0; i--) {
    const len = blocks[i].length + 2;
    if (total + len > SERIALIZE_TOTAL_CHAR_CAP || (headIdx >= 0 && i <= headIdx)) break;
    acc.unshift(blocks[i]);
    total += len;
  }
  const omitted = Math.max(0, blocks.length - acc.length - (headIdx >= 0 ? 1 : 0));
  return `${head}[... 中间 ${omitted} 条消息已省略 / earlier messages omitted ...]\n\n${acc.join('\n\n')}`;
}
