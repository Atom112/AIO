/**
 * Topic & agent-mode lifecycle actions (阶段4 第2项拆分切片：ChatPage <900 目标).
 *
 * Verbatim extraction of the five action handlers from ChatPage: stop generation,
 * switch to chat mode, switch to project, add topic, branch from message. Every
 * reactive dependency is a module-level store singleton or API wrapper, so these
 * are plain functions — no component deps. The DEFAULT_ASSISTANT id and the
 * topic factory moved along (shared by onMount's default-assistant bootstrap).
 */
import { stopActiveLlmStream, branchTopic } from '../../../core/api/chat';
import { t, locale } from '../../../core/i18n';
import {
  currentAssistantId,
  currentTopicId,
  datas,
  ensureProjectAssistant,
  initMcpServers,
  saveSingleAssistantToBackend,
  setCurrentAssistantId,
  setCurrentProjectId,
  setCurrentTopicId,
  setDatas,
  setPendingApprovals,
  type Topic,
} from '../../../core/store/store';

/** Stable id of the default (chat-mode) assistant. */
export const DEFAULT_ASST_ID = 'default-assistant-id';

/**
 * Helper: create a new topic object.
 * @param name - optional topic name; defaults to a timestamped name
 * @returns Topic with a unique ID, name, empty history, and empty summary
 */
export const createTopic = (name?: string): Topic => ({
  id: Date.now().toString(), // current timestamp used as the unique identifier
  name: name || t('chat.untitledTopic', { time: new Date().toLocaleTimeString(locale()) }), // default name includes the creation time
  history: [], // message history array
  summary: '', // long-term memory summary (SQLite storage), used to compress history context
});

/** Stop the current AI generation (CHAT-4: stops by stream ownership key, not view ids). */
export const handleStopGeneration = async () => {
  try {
    // CHAT-4: 按流归属键停止——切话题后当前视图 ID 打不到流（后端按键控
    // 静默 no-op），用户被迫切回原话题才能停止。优先停止当前视图的流，
    // 否则停止最近记录的活动流（启动对账后键集与后端一致）。
    await stopActiveLlmStream(currentAssistantId() ?? undefined, currentTopicId() ?? undefined);
  } catch (err) {
    console.error('stop_llm_stream failed:', err);
  }
  setPendingApprovals([]);
};

/** Switch to chat mode */
export const switchToChat = () => {
  setCurrentAssistantId(DEFAULT_ASST_ID);
  const asst = datas.assistants.find((a) => a.id === DEFAULT_ASST_ID);
  if (asst?.topics?.length) {
    setCurrentTopicId(asst.topics[0].id);
  }
};

/** Switch to the given project */
export const switchToProject = async (projectId: string) => {
  try {
    // CHAT-3: 必须同步 currentProjectId——Agent 工具按 currentProjectId 解析
    // 项目根（runAgentTurn 的 projectId 参数取自它）；漏同步时冷启动恢复
    // （localStorage 两键不一致）会让 agent 在错误工作区操作文件。
    // 与 ProjectSidebar 的同名实现保持五步一致。
    setCurrentProjectId(projectId);
    const asstId = await ensureProjectAssistant(projectId);
    setCurrentAssistantId(asstId);
    initMcpServers(projectId);
    const asst = datas.assistants.find((a) => a.id === asstId);
    if (asst?.topics?.length) {
      setCurrentTopicId(asst.topics[0].id);
    }
  } catch (e) {
    console.error('Failed to switch project:', e);
  }
};

/**
 * Add a new topic to the current assistant.
 * Creates the topic and makes it the selected one
 */
export const addTopic = async () => {
  const asstId = currentAssistantId();
  if (!asstId) return;
  const newT = createTopic();
  setDatas(
    'assistants',
    (a) => a.id === asstId,
    'topics',
    (prev) => [...prev, newT],
  );
  setCurrentTopicId(newT.id);
  await saveSingleAssistantToBackend(asstId);
};

/**
 * Branch a new topic from the given message (conversation branching).
 * Copies that message and everything before it into a new topic, then navigates to it.
 * @param sourceMessageId - message ID at the branch point
 */
export const branchFromMessage = async (sourceMessageId: string) => {
  const asstId = currentAssistantId();
  const topicId = currentTopicId();
  if (!asstId || !topicId) return;
  try {
    const newTopic: Topic = await branchTopic(topicId, sourceMessageId);
    setDatas(
      'assistants',
      (a) => a.id === asstId,
      'topics',
      (prev: Topic[]) => [...prev, newTopic],
    );
    setCurrentTopicId(newTopic.id);
    await saveSingleAssistantToBackend(asstId);
  } catch (e) {
    console.error('Failed to create branch:', e);
  }
};
