/**
 * Turn-finalization effects (CHAT-12 companion): auto topic rename + auto context
 * compaction, extracted from ChatPage into a module-level singleton.
 *
 * The global stream listeners (registered from the root Layout, see
 * `registerChatStreamListeners`) call these after a turn's `done` event — they must
 * outlive any page component, so they cannot be page-scope closures.
 */

import {
  archiveHistory,
  generateTopicTitle,
  llmCreds,
  summarizeHistory,
} from '../../../core/api/chat';
import {
  currentAssistantId,
  currentTopicId,
  datas,
  saveSingleAssistantToBackend,
  selectedModel,
  setDatas,
  type CompactOutcome,
  type Message,
  type Topic,
} from '../../../core/store/store';
import { locale, t } from '../../../core/i18n';

/**
 * Get the context window size of the currently selected model (read from the model
 * catalog, default 128k). Used for backend Agent loop token budgeting and frontend
 * history compression thresholds.
 */
export async function getModelContextWindow(): Promise<number> {
  let maxContext = 128_000; // default 128K
  try {
    const { getCachedCatalog } = await import('../../../core/utils/models');
    const cat = getCachedCatalog();
    const mdl = selectedModel();
    if (cat && mdl) {
      const modelMeta = cat.models?.find((m: { id: string }) => m.id === mdl.model_id);
      if (modelMeta?.contextWindow && modelMeta.contextWindow > 0) {
        maxContext = modelMeta.contextWindow;
      }
    }
  } catch {
    // keep the default when the model catalog is unavailable
  }
  return maxContext;
}

/** The currently selected assistant (from the global store). */
const currentAssistant = () => datas.assistants.find((a) => a.id === currentAssistantId());

/** The currently active topic of that assistant. */
const activeTopic = (): Topic | null => {
  const asst = currentAssistant();
  if (!asst) return null;
  return asst.topics.find((tp) => tp.id === currentTopicId()) || asst.topics[0] || null;
};

/**
 * Heuristic fallback: derive a title from the history text.
 * Guarantees at least a meaningful name when LLM title generation fails.
 * Uses the first 12 characters of the first user message.
 */
const fallbackTitle = (history: Message[]): string => {
  const firstUser = history.find((m) => m.role === 'user');
  if (!firstUser) return '';
  let text: string;
  if (typeof firstUser.content === 'string') {
    text = firstUser.content;
  } else if (firstUser.displayText) {
    text = firstUser.displayText;
  } else {
    // multimodal: try extracting text parts
    try {
      text = (firstUser.content as { type?: string; text?: string }[])
        .filter((p) => p?.type === 'text' && p.text)
        .map((p) => p.text)
        .join(' ');
    } catch {
      text = '';
    }
  }
  // strip newlines so the title stays on one line
  const cleaned = text.trim().replace(/\s+/g, ' ');
  return cleaned.length > 12 ? cleaned.slice(0, 12) : cleaned;
};

// Tracks the in-flight "title generation" task so unfinished callbacks can be cancelled
// when the user switches topics (Tauri invoke has no AbortSignal support yet; only result
// handling is skipped on the JS side — the call still completes, its result is ignored)
let activeTitleGen: { topicId: string; cancelled: boolean } | null = null;

/** The topic whose title generation is currently in flight (null = none). */
export const activeTitleGenTopicId = (): string | null => activeTitleGen?.topicId ?? null;

/** Cancel the in-flight title-generation task (user switched topics / new rename). */
export const cancelActiveTitleGen = () => {
  if (activeTitleGen) {
    activeTitleGen.cancelled = true;
    activeTitleGen = null;
  }
};

/**
 * Auto-rename non-default topics after the first conversation turn.
 * Rules:
 *   1. Skip the default topic (first entry in each assistant's topic list)
 *   2. Skip already-renamed topics (topic.renamed === true)
 *   3. Skip empty or errored AI replies
 *   4. Prefer LLM-generated titles; fall back to the heuristic on failure
 *   5. On success update topic.name, set renamed=true, and persist to the backend
 *   6. Cancel unfinished callback handling via activeTitleGen when the user switches topics
 */
export const checkAndRename = async (eventAsstId: string, eventTopicId: string) => {
  // cancel any unfinished title-generation task (the user switched topics or triggered a new rename)
  if (activeTitleGen) {
    activeTitleGen.cancelled = true;
    activeTitleGen = null;
  }
  const task = { topicId: eventTopicId, cancelled: false };
  activeTitleGen = task;

  const asst = datas.assistants.find((a) => a.id === eventAsstId);
  if (!asst) {
    if (activeTitleGen === task) activeTitleGen = null;
    return;
  }
  // must re-locate via the event's topicId (the user may have switched topics)
  const topic = (asst.topics as Topic[]).find((tp) => tp.id === eventTopicId);
  if (!topic) {
    if (activeTitleGen === task) activeTitleGen = null;
    return;
  }

  // 1. never rename the default topic
  if (asst.topics[0]?.id === topic.id) {
    if (activeTitleGen === task) activeTitleGen = null;
    return;
  }
  // 2. never rename an already-renamed topic
  if (topic.renamed) {
    if (activeTitleGen === task) activeTitleGen = null;
    return;
  }
  // 3. no conversation content to reference
  if (topic.history.length < 2) {
    if (activeTitleGen === task) activeTitleGen = null;
    return;
  }

  // 4. check the last AI message is valid (avoid error responses triggering a rename)
  const lastMsg = topic.history[topic.history.length - 1];
  if (lastMsg?.role !== 'assistant') {
    if (activeTitleGen === task) activeTitleGen = null;
    return;
  }
  const lastContent = typeof lastMsg.content === 'string' ? lastMsg.content.trim() : '';
  if (lastContent.length < 2 || lastContent.startsWith('[Error:')) {
    if (activeTitleGen === task) activeTitleGen = null;
    return;
  }

  const currentMdl = selectedModel();
  if (!currentMdl) {
    if (activeTitleGen === task) activeTitleGen = null;
    return;
  }

  // truncate to the first 2 messages: standard practice (user + assistant is enough to summarize the topic, saves tokens)
  const sample = topic.history.slice(0, 2);

  // phase 1: try LLM generation
  let newName: string | null = null;
  let llmError: unknown = null;
  try {
    const result = await generateTopicTitle(llmCreds(currentMdl), JSON.stringify(sample));
    // user switched topics or started a new task mid-flight: discard this result
    if (task.cancelled) return;
    if (result && result.trim()) {
      newName = result.trim();
    }
  } catch (e) {
    if (task.cancelled) return;
    llmError = e;
  }

  // user switched topics during the LLM call: abandon processing
  if (task.cancelled) return;

  // phase 2: heuristic fallback when the LLM fails
  if (!newName) {
    newName = fallbackTitle(topic.history);
    if (newName) {
      console.warn('LLM 标题生成失败，使用启发式后备:', llmError);
    }
  }

  // re-validate: state may have changed during the async work
  const latestAsst = datas.assistants.find((a) => a.id === eventAsstId);
  const latestTopic = latestAsst?.topics.find((tp) => tp.id === eventTopicId);
  if (!latestAsst || !latestTopic) {
    if (activeTitleGen === task) activeTitleGen = null;
    return;
  }
  if (latestAsst.topics[0]?.id === latestTopic.id) {
    if (activeTitleGen === task) activeTitleGen = null;
    return;
  }
  if (latestTopic.renamed) {
    if (activeTitleGen === task) activeTitleGen = null;
    return;
  }

  // no name obtainable at all: just mark as renamed to avoid retrying
  if (!newName) {
    setDatas(
      'assistants',
      (a) => a.id === eventAsstId,
      'topics',
      (tp) => tp.id === eventTopicId,
      'renamed',
      true,
    );
    await saveSingleAssistantToBackend(eventAsstId);
    if (activeTitleGen === task) activeTitleGen = null;
    console.warn('自动重命名失败，无可用后备:', llmError);
    return;
  }

  // same as the old name: treat as invalid but still mark as renamed
  if (newName === latestTopic.name) {
    setDatas(
      'assistants',
      (a) => a.id === eventAsstId,
      'topics',
      (tp) => tp.id === eventTopicId,
      'renamed',
      true,
    );
    await saveSingleAssistantToBackend(eventAsstId);
    if (activeTitleGen === task) activeTitleGen = null;
    return;
  }

  setDatas(
    'assistants',
    (a) => a.id === eventAsstId,
    'topics',
    (tp) => tp.id === eventTopicId,
    {
      name: newName,
      renamed: true,
    },
  );
  await saveSingleAssistantToBackend(eventAsstId);
  if (activeTitleGen === task) activeTitleGen = null;
  console.log(`话题已自动重命名: ${latestTopic.name} → ${newName}`);
};

/**
 * Check and summarize conversation history (token-based).
 * Automatic path (conversation-end event): compress when usage exceeds 75% of the
 * model context window; manual path (/compact, force=true) skips the threshold guard
 * and compresses immediately (mainstream agent behavior).
 * On summarize failure, falls back to local archiving (snapcompact-style): early
 * context is not lost and can be read back via read_artifact.
 */
export const checkAndSummarize = async (
  keepBudgetOverride?: number,
  opts?: { force?: boolean },
): Promise<CompactOutcome> => {
  const topic = activeTopic();
  const currentMdl = selectedModel();
  if (!topic || !currentMdl) return { kind: 'skipped', reason: 'nothing-to-compact' };

  // get the current model's context window size
  const maxContext = await getModelContextWindow();

  // Measure context usage: prefer the context peak of the last assistant message
  // (contextTokens already includes all history + system prefix + output; do not
  // double-count user messages)
  let totalUsed = 0;
  let hasAssistantCtx = false;
  for (let i = topic.history.length - 1; i >= 0; i--) {
    const msg = topic.history[i];
    if (msg.role === 'assistant') {
      totalUsed = (msg.contextTokens || msg.inputTokens || 0) + (msg.outputTokens || 0);
      hasAssistantCtx = true;
      break;
    }
  }
  // fall back to character estimation when there is no assistant reply yet (no completed turn)
  if (!hasAssistantCtx) {
    for (const msg of topic.history) {
      const text = typeof msg.content === 'string' ? msg.content : msg.displayText || '';
      totalUsed += Math.ceil(text.length / 4);
    }
  }

  const usageRatio = maxContext > 0 ? totalUsed / maxContext : 0;
  // automatic path keeps the threshold and minimum-history guards; manual /compact
  // always passes (summarizeCount decides whether anything is compressible)
  if (!opts?.force && (usageRatio < 0.75 || topic.history.length <= 10)) {
    return {
      kind: 'skipped',
      reason: usageRatio < 0.75 ? 'below-threshold' : 'short-history',
    };
  }

  if (hasAssistantCtx) {
    console.log(
      `[Context] 触发压缩: ${(usageRatio * 100).toFixed(0)}% (${totalUsed}/${maxContext} tokens, ${topic.history.length} 条消息)`,
    );
  }

  // Token-budget retention: by default keep ~20k tokens accumulated from the tail (at
  // least the last 4 messages, i.e. one full turn); `/compact <n>` overrides the keep
  // budget for this run
  const KEEP_TOKEN_BUDGET =
    keepBudgetOverride && keepBudgetOverride > 0 ? keepBudgetOverride : 20_000;
  const estimateMsgTokens = (m: Message): number => {
    const text = typeof m.content === 'string' ? m.content : (m.displayText ?? '');
    return Math.ceil(text.length / 4) + 8;
  };
  let keepCount = 0;
  let acc = 0;
  for (let i = topic.history.length - 1; i >= 0; i--) {
    acc += estimateMsgTokens(topic.history[i]);
    keepCount = topic.history.length - i;
    if (acc >= KEEP_TOKEN_BUDGET) break;
  }
  keepCount = Math.max(keepCount, 4);
  const summarizeCount = topic.history.length - keepCount;
  if (summarizeCount <= 0) return { kind: 'skipped', reason: 'nothing-to-compact' };
  // Overwriting summary: the old summary is injected as input context and replaced
  // wholesale by the new one, avoiding concatenation bloat
  const messagesToSummarize = topic.summary
    ? [
        { role: 'system' as const, content: t('chat.historyBackground') + topic.summary },
        ...topic.history.slice(0, summarizeCount),
      ]
    : topic.history.slice(0, summarizeCount);

  // Persist: trim the history and replace topic.summary with the new summary or
  // archive placeholder
  const persistCompactedTopic = async (newSummary: string): Promise<CompactOutcome> => {
    const latestTopic = activeTopic();
    if (!latestTopic) return { kind: 'skipped', reason: 'nothing-to-compact' };
    // CHAT-2: 入口捕获的话题与当前话题必须一致——多秒 summarize await 期间用户
    // 切换话题后，旧 count 裁新话题历史会把整个更短话题清空并落盘
    if (latestTopic.id !== topic.id) {
      console.warn('[Context] 压缩期间话题已切换，放弃持久化');
      return { kind: 'skipped', reason: 'topic-switched' } as CompactOutcome;
    }
    const updatedHistory = latestTopic.history.slice(summarizeCount);
    // CHAT-2 加固：裁剪结果不应清空历史（同话题变短即中止，防数据丢失）
    if (updatedHistory.length === 0 && latestTopic.history.length > 0) {
      console.warn('[Context] 压缩裁剪结果为空，放弃持久化（防数据丢失）');
      return { kind: 'skipped', reason: 'empty-result' } as CompactOutcome;
    }
    setDatas(
      'assistants',
      (a) => a.id === currentAssistantId(),
      'topics',
      (tp) => tp.id === latestTopic.id,
      {
        history: updatedHistory,
        summary: newSummary,
      },
    );
    await saveSingleAssistantToBackend(currentAssistantId()!);
    console.log('[Context] 压缩完成');
    return {
      kind: 'compressed',
      summarizedCount: summarizeCount,
      keptCount: updatedHistory.length,
    };
  };

  try {
    const newSummarySnippet = await summarizeHistory(
      llmCreds(currentMdl),
      JSON.stringify(messagesToSummarize),
      locale(),
    );
    return await persistCompactedTopic(newSummarySnippet);
  } catch (e) {
    // summarize failed -> local archive fallback (zero API cost; in Agent mode the
    // read_artifact tool can read the original back via the placeholder ID)
    try {
      const placeholder = await archiveHistory(
        JSON.stringify(messagesToSummarize),
        currentMdl.model_id,
        locale(),
      );
      const outcome = await persistCompactedTopic(placeholder);
      return outcome.kind === 'compressed' ? { kind: 'archived', count: summarizeCount } : outcome;
    } catch (archiveErr) {
      console.error('生成总结失败:', e, '归档亦失败:', archiveErr);
      return { kind: 'failed', error: String(e) };
    }
  }
};
