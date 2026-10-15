/**
 * Failed-turn placeholder cleanup (阶段4 第2项拆分切片：useChatSend 路径的 onInvokeError 实现).
 *
 * CHAT-10: when the invoke fails AFTER the assistant placeholder was created
 * (chat mode creates it locally; agent mode creates it on llm-round-start),
 * no done event ever arrives for a stream that never started. Without cleanup
 * the empty bubble sits in "thinking" forever. This module owns the cleanup:
 * - untouched empty placeholder → dropped entirely;
 * - partial stream → preserved, with running steps / orphaned calling tools
 *   closed as interrupted.
 *
 * Pure history-array transform (extracted verbatim from ChatPage's catch block
 * so it is unit-testable).
 */
import type { AgentStep, Message, ToolCallDisplay } from '../../../core/store/types';
import { TOOL_INTERRUPTED } from '../constants';

/** The message shape the cleanup touches (last assistant entry of a failed turn). */
type FailedAssistantMessage = Message & {
  toolCalls?: ToolCallDisplay[];
  reasoning?: string;
  images?: unknown;
};

/** True when an assistant message is a placeholder nothing ever landed in. */
export function isUntouchedAssistantPlaceholder(msg: Message): boolean {
  const m = msg as FailedAssistantMessage;
  return (
    !m.content &&
    !m.reasoning &&
    (m.agentSteps ?? []).length === 0 &&
    (m.toolCalls ?? []).length === 0 &&
    !m.images
  );
}

/**
 * Closes running steps / orphaned calling tools on ONE assistant message as
 * interrupted (shared by the failed-turn cleanup and the cold-load crash sanitize).
 * `now` sets the closed-step duration when given (mid-session cleanup); the cold-load
 * path omits it — a crashed turn's true runtime is unknowable, so no duration is set.
 */
export function closeInterruptedMessage(msg: Message, now?: number): Message {
  const m = msg as FailedAssistantMessage;
  const steps: AgentStep[] = m.agentSteps || [];
  const toolCalls: ToolCallDisplay[] = m.toolCalls || [];
  return {
    ...m,
    agentSteps: steps.map((s) => {
      if (s.status !== 'running') return s;
      const isCallingTool = s.type === 'tool_call' && s.toolCall?.state === 'calling';
      return {
        ...s,
        status: isCallingTool ? ('error' as const) : ('complete' as const),
        ...(now !== undefined ? { duration: now - s.timestamp } : {}),
        ...(isCallingTool && s.toolCall
          ? { toolCall: { ...s.toolCall, state: 'error' as const, error: TOOL_INTERRUPTED } }
          : {}),
      };
    }),
    toolCalls: toolCalls.some((tc) => tc.state === 'calling')
      ? toolCalls.map((tc) =>
          tc.state === 'calling' ? { ...tc, state: 'error' as const, error: TOOL_INTERRUPTED } : tc,
        )
      : toolCalls,
  };
}

/**
 * Closes the last assistant message after a failed turn:
 * drops it when untouched, or marks running steps / calling tools as interrupted.
 * Returns the new history array (identity when the last message is not an
 * assistant one or needs no change).
 */
export function cleanupFailedPlaceholder(history: Message[], now: number = Date.now()): Message[] {
  const lastIdx = history.length - 1;
  if (lastIdx < 0 || history[lastIdx]?.role !== 'assistant') return history;
  const msg = history[lastIdx] as FailedAssistantMessage;
  if (isUntouchedAssistantPlaceholder(msg)) return history.slice(0, lastIdx);
  // Partial stream preserved: close running steps / orphaned calling tools as interrupted
  return [...history.slice(0, lastIdx), closeInterruptedMessage(msg, now)];
}
