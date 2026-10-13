/**
 * Outgoing-message preparation (阶段4 第2项拆分切片：ChatPage <900 目标).
 *
 * Extracted verbatim from handleSendMessage:
 * - `getReasoningPrompt`: the reasoning-level system prompt (module level in ChatPage before);
 * - `resolveOutgoingContent`: auto vision analysis — a non-vision model first gets the
 *   vision agent's structured image descriptions; the raw base64 parts would be ignored
 *   or rejected. The image attachments are unlinked (id stripped) so later turns never
 *   re-embed them; the history bubbles still show the thumbnails (rendered from
 *   storagePath, not the id).
 * - `buildOutgoingMessages`: the system-message stack (assistant prompt + output style +
 *   skills + agent prompt + reasoning + web-search + summary) + history + user message,
 *   with ensureToolMessagesComplete as the tool-pairing safety net.
 */
import { t, locale } from '../../../core/i18n';
import { visionAnalyzeImages, llmCreds } from '../../../core/api/chat';
import { modelSupportsVision, resolveVisionModel } from '../vision';
import { buildSkillMessages } from './skillMessages';
import { buildApiMessages, ensureToolMessagesComplete } from './apiMessages';
import { buildAgentSystemPrompt } from '../../../core/agent-prompts';
import type { Assistant, Message, PendingAttachment, Topic } from '../../../core/store/store';

/** Inject the system prompt matching the reasoning level so the model emits thinking output. */
export function getReasoningPrompt(level: string): string | null {
  if (level === 'low') return t('agent.prompt.reasoningLow');
  if (level === 'medium') return t('agent.prompt.reasoningMedium');
  if (level === 'high') return t('agent.prompt.reasoningHigh');
  return null;
}

/** Files attached to the outgoing user message; vision-analyzed images get their id stripped. */
export type OutgoingFiles = Array<Omit<PendingAttachment, 'id'> & { id?: string }>;

/** Resolved outgoing content after the optional vision pass. */
export interface OutgoingContent {
  outgoingText: string;
  outgoingFiles: OutgoingFiles;
}

/**
 * Auto vision analysis: when the selected model cannot see images, the vision agent
 * looks at them first — its structured description replaces the raw image parts.
 * Vision failure falls back to sending the images unchanged.
 */
export async function resolveOutgoingContent(
  currentMdl: { model_id: string },
  files: PendingAttachment[],
  text: string,
  setIsThinking: (v: boolean) => void,
): Promise<OutgoingContent> {
  let outgoingText = text;
  let outgoingFiles: OutgoingFiles = files;
  if (files.some((f) => f.type === 'image') && !modelSupportsVision(currentMdl)) {
    const visionModel = resolveVisionModel(currentMdl);
    if (visionModel) {
      setIsThinking(true);
      try {
        const imageIds = files.filter((f) => f.type === 'image').map((f) => f.id);
        const description = await visionAnalyzeImages(
          llmCreds(visionModel),
          imageIds,
          outgoingText,
          locale(),
        );
        outgoingFiles = files.map((f) => (f.type === 'image' ? { ...f, id: undefined } : f));
        outgoingText = `${outgoingText}\n\n${t('chat.visionAnalysisPrefix')}\n${description}`;
      } catch (err) {
        // Vision analysis failed — fall back to sending the images as-is
        console.warn('[vision] analysis failed, sending images unchanged:', err);
      } finally {
        setIsThinking(false);
      }
    } else {
      console.warn('[vision] no vision-capable model available; sending images unchanged');
    }
  }
  return { outgoingText, outgoingFiles };
}

/** Explicit inputs of `buildOutgoingMessages` (all signal reads happen at the caller). */
export interface BuildOutgoingMessagesArgs {
  currentAsst: Assistant;
  currentTopic: Topic;
  /** Vision-resolved text (may embed the analysis prefix + description). */
  outgoingText: string;
  /** Vision-resolved files (image ids stripped after a successful analysis). */
  outgoingFiles: OutgoingFiles;
  /** The raw user input (drives the displayed bubble text). */
  userInput: string;
  reasoningLevel: string;
  webSearchEnabled: boolean;
  isChatMode: boolean;
  /** Active project (agent system prompt); null in global mode / chat mode. */
  projectInfo: { path: string; name: string } | null;
  /** Main-agent tool access policy (drives the tool-policy line; "hybrid" default). */
  mainAgentToolAccess?: 'hybrid' | 'full' | 'orchestrator';
}

/** The assembled outgoing turn: the persisted user message + the full API message list. */
export interface OutgoingMessages {
  newUserMsg: Message;
  messagesForAI: Message[];
}

/**
 * Assembles the full outgoing turn:
 * system stack (assistant prompt + style + skills + agent prompt + reasoning +
 * web-search + summary) → history (per-message API shape) → the new user message.
 * `ensureToolMessagesComplete` backfills any missing role:tool pairs (safety net).
 */
export function buildOutgoingMessages(args: BuildOutgoingMessagesArgs): OutgoingMessages {
  const {
    currentAsst,
    currentTopic,
    outgoingText,
    outgoingFiles,
    userInput,
    reasoningLevel,
    webSearchEnabled,
    isChatMode,
    projectInfo,
    mainAgentToolAccess,
  } = args;

  const newUserMsg = {
    id: crypto.randomUUID(),
    role: 'user' as const,
    content: outgoingText,
    displayFiles: outgoingFiles.map((f) => ({
      id: f.id,
      name: f.name,
      mimeType: f.mimeType,
      size: f.size,
      storagePath: f.storagePath,
    })),
    displayText: userInput,
  };

  const agentMode = currentAsst?.agentMode || 'off';
  const agentPromptContent =
    agentMode !== 'off' && projectInfo
      ? buildAgentSystemPrompt(
          agentMode as Parameters<typeof buildAgentSystemPrompt>[0],
          projectInfo,
          undefined,
          mainAgentToolAccess || 'hybrid',
        )
      : null;
  const agentSystemPrompt = agentPromptContent
    ? [{ role: 'system' as const, content: agentPromptContent }]
    : [];

  const reasoningPrompt = getReasoningPrompt(reasoningLevel);

  let messagesForAI: Message[] = [
    { role: 'system', content: currentAsst.prompt },
    // output style constraints (emoji avoidance etc., mirroring mainstream agent system-prompt style sections)
    { role: 'system', content: t('chat.prompt.style') },
    ...buildSkillMessages(currentAsst, isChatMode),
    ...agentSystemPrompt,
    ...(reasoningPrompt ? [{ role: 'system', content: reasoningPrompt }] : []),
    ...(webSearchEnabled ? [{ role: 'system', content: t('agent.prompt.web') }] : []),
    ...(currentTopic.summary
      ? [
          {
            role: 'system',
            content: t('agent.prompt.summary', { summary: currentTopic.summary }),
          },
        ]
      : []),
    ...currentTopic.history
      // Compaction context boundary: messages before it are condensed into
      // topic.summary (injected above) and excluded from LLM requests, while the
      // full transcript stays visible in the message record
      .slice(currentTopic.contextTrimCount ?? 0)
      .flatMap((m) => buildApiMessages(m)),
    { role: 'user', content: newUserMsg.content, displayFiles: newUserMsg.displayFiles },
  ];
  // safety net: ensure every assistant(tool_calls) has matching role:tool messages
  messagesForAI = ensureToolMessagesComplete(messagesForAI);

  return { newUserMsg, messagesForAI };
}
