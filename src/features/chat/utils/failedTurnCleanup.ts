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
  const steps: AgentStep[] = msg.agentSteps || [];
  const toolCalls: ToolCallDisplay[] = msg.toolCalls || [];
  const empty =
    !msg.content && !msg.reasoning && steps.length === 0 && toolCalls.length === 0 && !msg.images;
  if (empty) return history.slice(0, lastIdx); // drop the untouched placeholder
  // Partial stream preserved: close running steps / orphaned calling tools as interrupted
  const closedMsg = {
    ...msg,
    agentSteps: steps.map((s) => {
      if (s.status !== 'running') return s;
      const isCallingTool = s.type === 'tool_call' && s.toolCall?.state === 'calling';
      return {
        ...s,
        status: isCallingTool ? ('error' as const) : ('complete' as const),
        duration: now - s.timestamp,
      };
    }),
    toolCalls: toolCalls.some((tc) => tc.state === 'calling')
      ? toolCalls.map((tc) =>
          tc.state === 'calling' ? { ...tc, state: 'error' as const, error: TOOL_INTERRUPTED } : tc,
        )
      : toolCalls,
  };
  return [...history.slice(0, lastIdx), closedMsg];
}
