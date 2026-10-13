/**
 * Chat send primitive (阶段4 第2项拆分切片：useChatSend 路径).
 *
 * `startAgentTurn` centralizes the run_agent_turn payload assembly that every
 * send path repeats (credentials + ids + mode + project + web-search + locale +
 * context window) plus the thinking-state lifecycle (set before the invoke;
 * alert + teardown on failure). Callers keep only their unique parts: message
 * construction, persistence, and per-path options (overrides / custom profiles
 * / resumeTurnId / extra error cleanup).
 */
import { runAgentTurn, llmCreds, type RunAgentTurnArgs } from '../../../core/api/chat';
import { locale as localeSignal } from '../../../core/i18n';
import { datas, type Assistant } from '../../../core/store/store';
import { modelSupportsReasoningEffort } from '../../../core/utils/models';
import type { SkillConfig } from '../../../core/types/skill';
import { getModelContextWindow } from './chatTurnEffects';

/** Deps the ChatPage component provides. */
export interface ChatSendDeps {
  selectedModel: () =>
    { api_url: string; api_key?: string | null; model_id: string } | null | undefined;
  currentAssistantId: () => string | null;
  currentTopicId: () => string | null;
  currentProjectId: () => string | null;
  webSearchEnabled: () => boolean;
  reasoningLevel: () => string;
  setIsThinking: (v: boolean) => void;
  setTypingIndex: (v: number | null) => void;
}

/** Per-path options; only `messages`/`agentMode`/`skills` are mandatory. */
export interface StartAgentTurnOptions {
  messages: RunAgentTurnArgs['messages'];
  agentMode: RunAgentTurnArgs['agentMode'];
  /** Skills resolved by the caller (per-path condition: agent mode / chat mode). */
  skills: SkillConfig[];
  /** Per-profile model overrides (resolved by the main send path). */
  profileModelOverrides?: RunAgentTurnArgs['profileModelOverrides'];
  /** Custom subagent profiles (main send path). */
  customSubagentProfiles?: RunAgentTurnArgs['customSubagentProfiles'];
  /** Checkpoint turn to resume (the resume entry point). */
  resumeTurnId?: string;
  /** Extra caller-side cleanup after the invoke fails (e.g. placeholder removal). */
  onInvokeError?: (err: unknown) => void;
}

export function useChatSend(deps: ChatSendDeps) {
  const {
    selectedModel,
    currentAssistantId,
    currentTopicId,
    currentProjectId,
    webSearchEnabled,
    reasoningLevel,
    setIsThinking,
    setTypingIndex,
  } = deps;

  /** Starts one agent turn with the shared payload + thinking lifecycle. */
  const startAgentTurn = async (opts: StartAgentTurnOptions) => {
    const currentMdl = selectedModel();
    const asstId = currentAssistantId();
    const topicId = currentTopicId();
    if (!currentMdl || !asstId || !topicId) return;
    const currentAsst = datas.assistants.find((a: Assistant) => a.id === asstId);
    setIsThinking(true);
    try {
      const level = reasoningLevel();
      await runAgentTurn({
        ...llmCreds(currentMdl),
        assistantId: asstId,
        topicId,
        messages: opts.messages,
        mcpServerIds: currentAsst?.mcpServerIds ?? [],
        agentMode: opts.agentMode,
        projectId: currentProjectId() ?? null,
        webSearchEnabled: webSearchEnabled(),
        profileModelOverrides: opts.profileModelOverrides ?? [],
        customSubagentProfiles: opts.customSubagentProfiles ?? [],
        locale: localeSignal(),
        contextWindow: await getModelContextWindow(),
        skills: opts.skills,
        // Native reasoning-effort parameter only for models that accept one
        // (catalog capabilities.reasoning or id heuristic); otherwise the
        // prompt-level nudge in buildOutgoingMessages remains the control.
        reasoningLevel:
          level !== 'off' && modelSupportsReasoningEffort(currentMdl.model_id) ? level : undefined,
        ...(opts.resumeTurnId ? { resumeTurnId: opts.resumeTurnId } : {}),
      });
    } catch (err) {
      alert(err);
      setIsThinking(false);
      setTypingIndex(null);
      opts.onInvokeError?.(err);
    }
  };

  return { startAgentTurn };
}
