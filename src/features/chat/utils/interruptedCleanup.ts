/**
 * Crash-leftover cleanup for loaded history (cold start only).
 *
 * A turn that dies mid-flight (app kill / freeze / power loss) keeps its partial
 * assistant message thanks to incremental persistence (each tool result lands it),
 * but its toolCalls stay stuck in `calling` and its steps stuck in `running` — they
 * can never resolve after a restart. Mark them interrupted so the timeline does not
 * show perpetual spinners. A trailing assistant message that is completely untouched
 * is a dead round-start placeholder and is dropped.
 *
 * The per-message semantics are shared with the failed-turn cleanup
 * (`closeInterruptedMessage` / `isUntouchedAssistantPlaceholder`); the cold-load path
 * deliberately sets no step durations — a crashed turn's true runtime is unknowable.
 */
import type { Assistant, Message } from '../../../core/store/store';
import { closeInterruptedMessage, isUntouchedAssistantPlaceholder } from './failedTurnCleanup';

export interface SanitizeResult {
  assistants: Assistant[];
  /** Ids of assistants whose data changed (the caller persists them back). */
  changedAssistantIds: string[];
}

/** Returns the cleaned message, or null when it is a dead placeholder to drop. */
const sanitizeMessage = (m: Message, isLast: boolean): Message | null => {
  if (m.role !== 'assistant') return m;
  // Dead round-start placeholder: the turn died before any content landed.
  if (isLast && isUntouchedAssistantPlaceholder(m)) return null;
  const needsClose =
    (m.toolCalls ?? []).some((c) => c.state === 'calling') ||
    (m.agentSteps ?? []).some((s) => s.status === 'running');
  if (!needsClose) return m;
  return closeInterruptedMessage(m);
};

/**
 * Walk all loaded assistants/topics and close every crash-leftover interrupted state.
 * Pure: returns a new array (unchanged branches keep their references) plus the ids
 * of assistants that changed and must be persisted back.
 */
export function sanitizeLoadedAssistants(assistants: Assistant[]): SanitizeResult {
  const changedAssistantIds: string[] = [];
  const out = assistants.map((asst) => {
    let asstChanged = false;
    const topics = asst.topics.map((topic) => {
      let topicChanged = false;
      const history: Message[] = [];
      topic.history.forEach((m, i) => {
        const cleaned = sanitizeMessage(m, i === topic.history.length - 1);
        if (cleaned === null) {
          topicChanged = true;
          return;
        }
        if (cleaned !== m) topicChanged = true;
        history.push(cleaned);
      });
      if (!topicChanged) return topic;
      asstChanged = true;
      return { ...topic, history };
    });
    if (asstChanged) changedAssistantIds.push(asst.id);
    return asstChanged ? { ...asst, topics } : asst;
  });
  return { assistants: out, changedAssistantIds };
}
