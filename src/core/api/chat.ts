// Chat-domain Tauri command wrappers (counterpart of commands/messages.rs and commands/llm/mod.rs).
// Convention: components must not invoke these command names directly; always go through this module.
import { invoke } from '@tauri-apps/api/core';
import type { AgentMode, Assistant, Message, Topic } from '../store/types';
import type { ProfileModelOverrideBackend, CustomSubagentProfile } from '../store/profiles';
import type { SkillConfig } from '../types/skill';

/** LLM connection credentials (apiUrl/apiKey/model). */
export interface LlmCredentials {
  apiUrl: string;
  apiKey: string;
  model: string;
}

/** Build LLM credentials from an ActivatedModel (canonical conversion so components don't re-assemble it). */
export const llmCreds = (m: {
  api_url: string;
  api_key?: string | null;
  model_id: string;
}): LlmCredentials => ({
  apiUrl: m.api_url,
  apiKey: m.api_key ?? '',
  model: m.model_id,
});

/** Append a message to a topic (persisted). */
export const appendMessage = (topicId: string, message: Message) =>
  invoke<void>('append_message', { topicId, message });

/** Delete a message from a topic. */
export const deleteTopicMessage = (topicId: string, messageId: string) =>
  invoke<void>('delete_topic_message', { topicId, messageId });

/** Branch a new topic from a message. */
export const branchTopic = (sourceTopicId: string, sourceMessageId: string) =>
  invoke<Topic>('branch_topic', { sourceTopicId, sourceMessageId });

/** Load all assistants (including their topic and message trees). */
export const loadAssistants = () => invoke<Assistant[]>('load_assistants');

/** Save (create or update) an assistant. */
export const saveAssistant = (assistant: Assistant) =>
  invoke<void>('save_assistant', { assistant: assistant as unknown as Record<string, unknown> });

/** Delete an assistant. */
export const deleteAssistant = (id: string) => invoke<void>('delete_assistant', { id });

/** Compress/summarize history (overwriting summary); returns the new summary text. locale selects the summary prompt language. */
export const summarizeHistory = (creds: LlmCredentials, messagesJson: string, locale: string) =>
  invoke<string>('summarize_history', { ...creds, messagesJson, locale });

/** Local archive fallback when summarization fails: writes to {app_data}/archives and returns an "archived history #id" placeholder text. */
export const archiveHistory = (messagesJson: string, model: string, locale: string) =>
  invoke<string>('archive_history', { messagesJson, model, locale });

/** Auto vision analysis: a vision-capable model describes the images (used when the main model lacks vision). Returns the structured description. */
export const visionAnalyzeImages = (
  creds: LlmCredentials,
  attachmentIds: string[],
  userText: string,
  locale: string,
) =>
  invoke<string>('vision_analyze_images', {
    ...creds,
    attachmentIds,
    userText,
    locale,
  });

/** Generate a title for a topic; returns the title text. */
export const generateTopicTitle = (creds: LlmCredentials, messagesJson: string) =>
  invoke<string>('generate_topic_title', { ...creds, messagesJson });

/** BTW side-question (streaming; reported back to the given overlay via the btw-chunk event). */
export const askBtwQuestion = (creds: LlmCredentials, overlayId: string, messagesJson: string) =>
  invoke<void>('ask_btw_question', { ...creds, overlayId, messagesJson });

/** Generate an image (progress reported via llm-chunk events). */
export const generateImage = (
  creds: LlmCredentials,
  assistantId: string,
  topicId: string,
  prompt: string,
) => invoke<void>('generate_image', { ...creds, assistantId, topicId, prompt });

/** Chat-mode streaming call (no tools). `reasoningLevel` (low/medium/high) attaches a native reasoning-effort parameter when the model supports one. */
export const callLlmStream = (
  creds: LlmCredentials,
  assistantId: string,
  topicId: string,
  messages: Message[],
  reasoningLevel?: string | null,
) => {
  recordStreamKey(assistantId, topicId);
  return invoke<void>('call_llm_stream', {
    ...creds,
    assistantId,
    topicId,
    messages,
    ...(reasoningLevel ? { reasoningLevel } : {}),
  });
};

// ---- CHAT-4: 流归属键 ----
// Stop 按流键控而非当前视图：流按 (assistantId, topicId) 键控，切话题后用当前
// 视图 ID 调 stop 会打到不存在的流（后端静默 no-op），原流继续跑而 isThinking
// 全局拦截所有发送——用户被迫切回原话题才能停止。
// Key set survives topic switches; a webview reload loses it, so startup
// reconciliation (reconcileActiveStreamKeys) re-imports the backend's live set.
const activeStreamKeys = new Map<string, { assistantId: string; topicId: string }>();
let lastStreamKey: string | null = null;

const streamKeyOf = (assistantId: string, topicId: string) => `${assistantId}::${topicId}`;

function recordStreamKey(assistantId: string, topicId: string) {
  const k = streamKeyOf(assistantId, topicId);
  activeStreamKeys.set(k, { assistantId, topicId });
  lastStreamKey = k;
}

/** Drop the stream key when its terminal done event arrives (epilogue = stream over). */
export function clearStreamKey(assistantId: string, topicId: string) {
  activeStreamKeys.delete(streamKeyOf(assistantId, topicId));
}

/** Stop the LLM streaming task actually in flight (keyed by stream ownership).
 *  Prefers the current view's stream when given; otherwise the most recent one. */
export const stopActiveLlmStream = (currentAssistantId?: string, currentTopicId?: string) => {
  const preferred =
    currentAssistantId && currentTopicId
      ? activeStreamKeys.get(streamKeyOf(currentAssistantId, currentTopicId))
      : undefined;
  const fallback = lastStreamKey ? activeStreamKeys.get(lastStreamKey) : undefined;
  const target = preferred ?? fallback ?? [...activeStreamKeys.values()][0];
  if (!target) return Promise.resolve();
  return invoke<void>('stop_llm_stream', target);
};

/** Re-import the backend's active stream set after a webview reload (module state is
 *  lost while backend tasks keep running). Returns the number of live streams so the
 *  caller can restore the thinking state. */
export const reconcileActiveStreamKeys = async (): Promise<number> => {
  const list = await invoke<{ assistantId: string; topicId: string }[]>('list_active_streams');
  for (const s of list) recordStreamKey(s.assistantId, s.topicId);
  return list.length;
};

/** run_agent_turn arguments (one autonomous agent-loop turn). */
export interface RunAgentTurnArgs extends LlmCredentials {
  assistantId: string;
  topicId: string;
  messages: Message[];
  mcpServerIds: string[];
  agentMode: AgentMode;
  projectId: string | null;
  webSearchEnabled: boolean;
  profileModelOverrides: ProfileModelOverrideBackend[];
  customSubagentProfiles: CustomSubagentProfile[];
  locale: string;
  contextWindow: number;
  skills?: SkillConfig[];
  /** Reasoning effort applied to the main-model request (low/medium/high); omitted = no native reasoning parameter. */
  reasoningLevel?: string;
  /** Resume the checkpoint of the given turn (continue after a crash); omitted = a normal new turn */
  resumeTurnId?: string;
}

/** Start one autonomous agent-loop turn (progress reported via the llm-* event series). */
export const runAgentTurn = (args: RunAgentTurnArgs) => {
  recordStreamKey(args.assistantId, args.topicId);
  return invoke<void>('run_agent_turn', args as unknown as Record<string, unknown>);
};

/** Stop the LLM streaming task of a topic. */
export const stopLlmStream = (assistantId: string, topicId: string) =>
  invoke<void>('stop_llm_stream', { assistantId, topicId });

/** Stop a background subagent (fire-and-forget). Returns whether it was found and signalled to cancel (false = already finished or absent; idempotent). */
export const stopSubagent = (subagentId: string) =>
  invoke<boolean>('stop_subagent', { subagentId });

/** Active checkpoint info (a turn left over by a crash, resumable). */
export interface ActiveCheckpoint {
  turnId: string;
  topicId: string;
  round: number;
  createdAt: number;
}

/** Query the active checkpoint of a topic (entry point for resuming an unfinished turn after a crash). */
export const getActiveCheckpoint = (topicId: string) =>
  invoke<ActiveCheckpoint | null>('get_active_checkpoint', { topicId });

/** One answer of an ask_user request (matches backend QuestionAnswer, camelCase). */
export interface QuestionAnswerPayload {
  questionId: string;
  answer: string;
  isCustom: boolean;
}

/**
 * Answer an ask_user request (agent-question-requested event): resolves the paused agent
 * loop with the user's answers; skipped = let the agent decide on its own.
 */
export const respondAgentQuestion = (
  requestId: string,
  skipped: boolean,
  answers: QuestionAnswerPayload[],
) => invoke<void>('respond_agent_question', { requestId, skipped, answers });
