/**
 * Slash-command handlers (阶段4 第2项拆分切片：ChatPage <900 目标).
 *
 * Verbatim extraction of the two slash dispatch branches from ChatPage:
 * - prompt-type commands (/review, /explain, ...): the resolved promptBody goes to
 *   the LLM as a user turn — the message assembly now REUSES buildOutgoingMessages
 *   (sendMessagePrep); the previously inlined system stack was field-for-field
 *   identical to the main send path (the only differences — no displayFiles, skills
 *   gated on `agentMode === 'off'` — flow through the same args).
 * - action-type commands (/compact, /clear, /search, /settings, /help): append the
 *   command bubble, run the handler, append per-command feedback.
 *
 * Nearly all reactive deps (ids, model, reasoning, web search, project) are
 * module-level store singletons and are imported directly; only the three
 * component-scoped values arrive via `SlashHandlerDeps`.
 */
import { appendMessage } from '../../../core/api/chat';
import { t } from '../../../core/i18n';
import {
  currentAssistantId,
  currentProject,
  currentProjectId,
  currentTopicId,
  reasoningLevel,
  resolveAssistantSkills,
  selectedModel,
  setDatas,
  webSearchEnabled,
  type Assistant,
  type Topic,
} from '../../../core/store/store';
import { resolveSlashCommand } from '../../../core/shortcuts';
import { buildOutgoingMessages } from './sendMessagePrep';
import type { StartAgentTurnOptions } from '../hooks/useChatSend';

/** Component-scoped deps (the rest come from module-level store signals). */
export interface SlashHandlerDeps {
  currentAssistant: () => Assistant | undefined;
  activeTopic: () => Topic | null | undefined;
  startAgentTurn: (opts: StartAgentTurnOptions) => Promise<void>;
}

/**
 * Slash command — promptBody type (/review, /explain, etc.)
 * Chat shows the raw command text; the resolved promptBody is sent to the LLM.
 */
export async function handleSlashPromptMessage(
  deps: SlashHandlerDeps,
  resolved: ReturnType<typeof resolveSlashCommand>,
) {
  if (!resolved) return;
  const asstId = currentAssistantId();
  const topicId = currentTopicId();
  if (!asstId || !topicId) return;

  const currentMdl = selectedModel();
  const currentAsst = deps.currentAssistant();
  const currentTopic = deps.activeTopic();
  if (!currentMdl || !currentAsst || !currentTopic) return;

  const displayText = resolved.command.label + (resolved.args ? ` ${resolved.args}` : '');
  let content = resolved.resolvedBody || resolved.command.label;

  // with no args, inject the current project path to give the LLM context
  if (!resolved.args) {
    const project = currentProject();
    if (project) {
      content = `${content}\n\n${t('agent.prompt.projectContext', { path: project.path })}`;
    }
  }

  // build the full message array (same assembly as the normal send flow; no
  // attachments on slash turns — an empty displayFiles list behaves identically)
  const agentMode = currentAsst?.agentMode || 'off';
  const pid = currentProjectId();
  const { newUserMsg, messagesForAI } = buildOutgoingMessages({
    currentAsst,
    currentTopic,
    outgoingText: content,
    outgoingFiles: [],
    userInput: displayText,
    reasoningLevel: reasoningLevel(),
    webSearchEnabled: webSearchEnabled(),
    isChatMode: agentMode === 'off' && !pid,
    projectInfo:
      pid && currentProject()
        ? { path: currentProject()!.path, name: currentProject()!.name }
        : null,
  });

  // persist the user message
  try {
    await appendMessage(topicId, newUserMsg);
  } catch (err) {
    console.error('[chat] save message failed:', err);
    alert(t('error.save'));
    return;
  }

  // update local state
  setDatas(
    'assistants',
    (a) => a.id === asstId,
    'topics',
    (t) => t.id === topicId,
    'history',
    (h) => [...h, newUserMsg],
  );

  // call the LLM (shared payload + thinking lifecycle in useChatSend)
  await deps.startAgentTurn({
    messages: messagesForAI,
    agentMode,
    skills: agentMode === 'off' && !pid ? [] : resolveAssistantSkills(currentAsst),
  });
}

/**
 * Slash command — pure action type (/compact, /clear, /search, /settings, /help)
 * Chat shows the command, runs the handler, then shows brief feedback.
 */
export async function handleSlashActionMessage(resolved: ReturnType<typeof resolveSlashCommand>) {
  if (!resolved) return;
  const asstId = currentAssistantId();
  const topicId = currentTopicId();
  if (!asstId || !topicId) return;

  const cmd = resolved.command;
  const displayText = cmd.label;

  // append the user message
  setDatas(
    'assistants',
    (a) => a.id === asstId,
    'topics',
    (t) => t.id === topicId,
    'history',
    (h) => [
      ...h,
      {
        id: crypto.randomUUID(),
        role: 'user' as const,
        content: displayText,
        displayText,
      },
    ],
  );

  // run the handler (action commands may take args, e.g. `/compact 10000` sets the keep-token budget);
  // a string return value is used as custom feedback (/compact distinguishes compressed / skipped / failed)
  const handlerResult = await cmd.handler(resolved.args);

  // produce a feedback message per command type
  let feedback = '';
  if (cmd.id === 'slash-compact') {
    feedback =
      typeof handlerResult === 'string'
        ? handlerResult
        : `✓ /compact — ${t('slash.feedback.compactSkipped')}`;
  } else if (cmd.id === 'slash-clear') {
    // the /clear handler already emptied the history; no extra feedback needed
    return;
  } else if (cmd.id === 'slash-search') {
    feedback = webSearchEnabled()
      ? `✓ ${t('slash.feedback.searchOn')}`
      : `✓ ${t('slash.feedback.searchOff')}`;
  } else if (cmd.id === 'slash-help') {
    // the /help handler already appended help info; no extra feedback needed
    return;
  } else if (cmd.id === 'slash-settings') {
    // the /settings handler already navigated away
    return;
  } else {
    feedback = `✓ ${t('slash.feedback.executed', { command: cmd.label })}`;
  }

  // append the feedback message
  setDatas(
    'assistants',
    (a) => a.id === asstId,
    'topics',
    (t) => t.id === topicId,
    'history',
    (h) => [
      ...h,
      {
        id: crypto.randomUUID(),
        role: 'assistant' as const,
        content: feedback,
        displayText: feedback,
      },
    ],
  );
}
