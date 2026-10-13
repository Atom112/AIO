/**
 * Checkpoint resume entry (阶段4 第2项拆分切片：ChatPage <900 目标).
 *
 * Verbatim extraction from ChatPage: the "continue unfinished turn" banner state
 * for crash-orphaned turns. Watches the active topic, loads its pending checkpoint
 * (race-guarded against topic switches), and resumes the turn via the shared
 * send primitive — the backend substitutes the checkpoint snapshot on resume.
 */
import { createSignal, createEffect } from 'solid-js';
import { getActiveCheckpoint, type ActiveCheckpoint } from '../../../core/api/chat';
import {
  currentAssistantId,
  currentTopicId,
  isThinking,
  resolveAssistantSkills,
  selectedModel,
  datas,
} from '../../../core/store/store';
import type { StartAgentTurnOptions } from './useChatSend';

/** Deps the ChatPage component provides. */
export interface CheckpointResumeDeps {
  startAgentTurn: (opts: StartAgentTurnOptions) => Promise<void>;
}

export function useCheckpointResume(deps: CheckpointResumeDeps) {
  // ===== Checkpoint resume: "continue unfinished turn" entry for crash-orphaned turns =====
  const [activeCheckpoint, setActiveCheckpoint] = createSignal<ActiveCheckpoint | null>(null);
  createEffect(() => {
    const tid = currentTopicId();
    setActiveCheckpoint(null);
    if (!tid) return;
    void (async () => {
      try {
        const cp = await getActiveCheckpoint(tid);
        // race guard: apply only if the topic has not switched again
        if (currentTopicId() === tid) setActiveCheckpoint(cp);
      } catch {
        /* silent on query failure: the entry point is not shown */
      }
    })();
  });

  const handleResumeTurn = async () => {
    const cp = activeCheckpoint();
    const tid = currentTopicId();
    const asstId = currentAssistantId();
    const currentMdl = selectedModel();
    if (!cp || !tid || !asstId || !currentMdl || isThinking()) return;
    const currentAsst = datas.assistants.find((a) => a.id === asstId);
    const resumeAgentMode = currentAsst?.agentMode || 'off';
    setActiveCheckpoint(null);
    // shared payload + thinking lifecycle in useChatSend; the backend substitutes
    // the checkpoint snapshot on resume (frontend messages unused)
    await deps.startAgentTurn({
      messages: [],
      agentMode: resumeAgentMode,
      // 只读 (off) with a project is an agent tier now — skills load like other tiers
      skills:
        resumeAgentMode === 'off' && !currentAsst?.projectId
          ? []
          : resolveAssistantSkills(currentAsst),
      resumeTurnId: cp.turnId,
    });
  };

  return { activeCheckpoint, handleResumeTurn };
}
