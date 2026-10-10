import { Component, createSignal, onMount, onCleanup, createEffect, Show } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import {
  appendMessage,
  askBtwQuestion as askBtwQuestionCmd,
  branchTopic,
  callLlmStream,
  generateImage,
  generateTopicTitle,
  getActiveCheckpoint,
  llmCreds,
  loadAssistants,
  runAgentTurn,
  stopLlmStream,
  summarizeHistory,
  archiveHistory,
  visionAnalyzeImages,
  type ActiveCheckpoint,
} from '../../core/api/chat';
import { storeChatAttachment } from '../../core/api/attachment';
import { registerChatEventListeners } from './hooks/chatEventListeners';
import { registerChatCommands } from './hooks/chatCommands';
import { TOOL_INTERRUPTED } from './constants';
import { autoDetectLsp, startLspServer } from '../../core/api/lsp';
import { listen } from '@tauri-apps/api/event';
import { useNavigate } from '@solidjs/router';
import {
  datas,
  setDatas,
  currentAssistantId,
  setCurrentAssistantId,
  currentTopicId,
  setCurrentTopicId,
  saveSingleAssistantToBackend,
  Assistant,
  Topic,
  Message,
  PendingAttachment,
  selectedModel,
  setSelectedModel,
  resolveAssistantModel,
  modelKey,
  reasoningLevel,
  setReasoningLevel,
  webSearchEnabled,
  setWebSearchEnabled,
  pendingRenameRequest,
  setPendingRenameRequest,
  resolveAssistantSkills,
  currentProjectId,
  currentProject,
  profileModelOverrides,
  allAvailableModels,
  customSubagentProfiles,
  workflowState,
  setWorkflowState,
  setAgentTaskBoard,
  isChatMode,
  projects,
  ensureProjectAssistant,
  setShowProjectCreateModal,
  getLastAgentProjectId,
  initMcpServers,
  activeSubagentView,
  setActiveSubagentView,
  type CompactOutcome,
} from '../../core/store/store';
import { EVENTS } from '../../core/api/events';
import { buildAgentSystemPrompt } from '../../core/agent-prompts';
import { unregisterCommand, resolveSlashCommand } from '../../core/shortcuts';
import { locale, t } from '../../core/i18n';
import Icon from '../../shared/components/Icon';
import { isImageGenModel } from '../../core/utils/models';
import { modelSupportsVision, resolveVisionModel } from './vision';
import ProjectSidebar from './components/ProjectSidebar';
import ProjectSettingsModal from './components/ProjectSettingsModal';
import ChatInterface from './components/ChatInterface';
import SubagentConversationView from './components/SubagentConversationView';
import TopicSidebar from './components/TopicSidebar';
import { Portal } from 'solid-js/web';
import ShareModal from './components/ShareModal';
import ProblemsPanel from './components/ProblemsPanel';
import WorkflowVisualization from './components/WorkflowVisualization';

function getReasoningPrompt(level: string): string | null {
  if (level === 'low') return t('agent.prompt.reasoningLow');
  if (level === 'medium') return t('agent.prompt.reasoningMedium');
  if (level === 'high') return t('agent.prompt.reasoningHigh');
  return null;
}

/**
 * Get the context window size of the currently selected model (read from the model catalog, default 128k).
 * Used for backend Agent loop token budgeting and frontend history compression thresholds.
 */
async function getModelContextWindow(): Promise<number> {
  let maxContext = 128_000; // default 128K
  try {
    const { getCachedCatalog } = await import('../../core/utils/models');
    const cat = getCachedCatalog();
    const mdl = selectedModel();
    if (cat && mdl) {
      const modelMeta = cat.models?.find((m: any) => m.id === mdl.model_id);
      if (modelMeta?.contextWindow && modelMeta.contextWindow > 0) {
        maxContext = modelMeta.contextWindow;
      }
    }
  } catch {
    // keep the default when the model catalog is unavailable
  }
  return maxContext;
}

import type { PendingApproval } from './components/ToolApprovalBubble';
import type { PendingQuestion } from './components/AgentQuestionBubble';
import TaskBoardPanel from './components/TaskBoardPanel';
import { buildSkillMessages } from './utils/skillMessages';
import { clearAllDiagnostics } from '../../core/store/diagnostics';

let isFirstAppLaunch = true;
const DEFAULT_ASST_ID = 'default-assistant-id';

// Tracks the in-flight "title generation" task so unfinished callbacks can be cancelled
// when the user switches topics (Tauri invoke has no AbortSignal support yet; only result
// handling is skipped on the JS side — the call still completes, its result is ignored)
let activeTitleGen: { topicId: string; cancelled: boolean } | null = null;

/**
 * Helper: create a new topic object.
 * @param name - optional topic name; defaults to a timestamped name
 * @returns Topic with a unique ID, name, empty history, and empty summary
 */
const createTopic = (name?: string): Topic => ({
  id: Date.now().toString(), // current timestamp used as the unique identifier
  name: name || t('chat.untitledTopic', { time: new Date().toLocaleTimeString(locale()) }), // default name includes the creation time
  history: [], // message history array
  summary: '', // long-term memory summary (SQLite storage), used to compress history context
});

/**
 * Helper: create a new assistant object.
 * @param name - optional assistant name
 * @param id - optional explicit ID (reuses the existing ID when restoring from the database)
 * @returns Assistant with ID, name, system prompt, and a default topic
 */
const createAssistant = (name?: string, id?: string): Assistant => ({
  id: id ?? Date.now().toString(), // generate a fresh timestamp ID when none is provided
  name: name || t('chat.newAssistant'), // default assistant name
  prompt: t('chat.defaultAssistantPrompt'), // default system prompt
  modelId: selectedModel() ? modelKey(selectedModel()!) : undefined, // inherit the active model (composite key) as the default model
  // with a project context, auto-enable the built-in filesystem MCP so agents work out of the box
  mcpServerIds: currentProjectId() ? ['__aio-filesystem__'] : [],
  skillIds: [],
  projectId: currentProjectId() ?? undefined, // linked project
  assistantType: id === DEFAULT_ASST_ID ? 'chat' : 'project',
  topics: [createTopic(t('chat.defaultTopic'))], // every assistant starts with one "default topic"
});

/**
 * Chat page root component.
 * Owns the three-column layout, all business logic, and state transitions.
 * @component
 */
const ChatPage: Component = () => {
  const [leftPanelWidth, setLeftPanelWidth] = createSignal(
    Number(localStorage.getItem('chat-left-panel-width')) || 18,
  ); // left panel width percentage (assistant list), default 18%, range 15%-30%
  const [rightPanelWidth, setRightPanelWidth] = createSignal(
    Number(localStorage.getItem('chat-right-panel-width')) || 18,
  ); // right panel width percentage (topic list), default 18%, range 15%-30%
  /**
   * Left panel: width tracks the cursor 1:1 throughout the drag (both directions), snapping only
   * on release; the narrowest tracking width equals the collapsed rail width, so the handle stays grabbable and content never overflows.
   */
  const LEFT_RAIL_WIDTH_PX = 54; // collapsed rail width, kept in sync with ProjectSidebar
  const LEFT_MAX_WIDTH_PCT = 30;
  /** Release deadzone (px): dragging out less than this counts as not really opened, so it still collapses on release */
  const LEFT_RELEASE_DEADZONE_PX = 12;
  const [isResizing, setIsResizing] = createSignal(false);
  /** Live width preview (%) while dragging: rendered during tracking, cleared on release to fall back to the remembered leftPanelWidth */
  const [leftDragWidth, setLeftDragWidth] = createSignal<number | null>(null);
  /**
   * "Curtain" layout width of the expanded content (px, = content-box width of the 15% minimal panel).
   * While narrowing, content neither reflows nor fades — it is progressively clipped by the panel's left
   * edge (curtain reveal), avoiding an empty transition frame with both content sets invisible; recomputed on resize.
   */
  const [leftRevealMinPx, setLeftRevealMinPx] = createSignal(0);
  /** rAF id of the release snap animation: cancelled when a new drag starts so it never fights the user's gesture */
  let leftSnapRaf: number | undefined;
  const [isLeftCollapsed, setIsLeftCollapsed] = createSignal(
    localStorage.getItem('left-collapsed') === 'true',
  ); // left/right panel width adjustment logic
  const [isRightCollapsed, setIsRightCollapsed] = createSignal(
    localStorage.getItem('right-collapsed') === 'true',
  );
  const [inputMessage, setInputMessage] = createSignal(''); // message text currently in the input box
  const [pendingFiles, setPendingFiles] = createSignal<PendingAttachment[]>([]); // files staged for sending (copied into the app attachment dir, not yet linked to a message)
  const [isThinking, setIsThinking] = createSignal(false); // whether the AI is thinking/generating a reply (drives the loading animation and stop button)
  const [isProcessing, setIsProcessing] = createSignal(false); // whether a file is being processed (drives the file-parsing loading state)
  const [isDragging, setIsDragging] = createSignal(false); // whether files are being dragged over the window (drives drag-state styling)
  const [typingIndex, setTypingIndex] = createSignal<number | null>(null); // index of the message currently shown with the typewriter effect; null means no effect
  const [editingTopicId, setEditingTopicId] = createSignal<string | null>(null); // ID of the topic whose name is being edited; null when not editing
  const [settingsAsstId, setSettingsAsstId] = createSignal<string | null>(null); // ID of the assistant whose settings modal is open; null when closed
  // tool calls awaiting user approval
  /** Share modal state */
  const [showShareModal, setShowShareModal] = createSignal(false);
  const [activeShareTopicId, setActiveShareTopicId] = createSignal<string | null>(null);
  /** Message-level selection mode */
  const [isSelectingMessages, setIsSelectingMessages] = createSignal(false);
  const [selectedMessageIds, setSelectedMessageIds] = createSignal<Set<string>>(new Set());
  const [pendingApprovals, setPendingApprovals] = createSignal<PendingApproval[]>([]);
  /** Pending ask_user question requests (agent pauses until answered/skipped) */
  const [pendingQuestions, setPendingQuestions] = createSignal<PendingQuestion[]>([]);
  /** /btw floating Q&A overlay list (ephemeral, not stored in history) */
  const [btwOverlays, setBtwOverlays] = createSignal<
    Array<{ id: string; question: string; answer: string | null; loading: boolean }>
  >([]);
  /** Page root element ref, used to compute relative positions while resizing panels */
  let chatPageRef: HTMLDivElement | undefined;
  /**
   * Compute the left panel display width.
   * @returns {number} live tracking width while dragging, otherwise the remembered width (collapsed state is rendered as rail width by the child component)
   */
  const displayLeftWidth = () => leftDragWidth() ?? leftPanelWidth();
  /**
   * Whether the left panel currently renders as the collapsed rail.
   * During tracking (leftDragWidth non-null) it always renders expanded, so dragging
   * out from the rail is truly cursor-driven instead of popping open via CSS on release.
   */
  const leftPanelCollapsed = () => isLeftCollapsed() && leftDragWidth() === null;

  // Legacy: old drag code could persist intermediate widths < 15% to localStorage; restore the default when expanded
  createEffect(() => {
    if (!isLeftCollapsed() && leftPanelWidth() < 15) setLeftPanelWidth(18);
  });

  // Curtain-reveal target width: content-box width of the 15% minimal panel (asymmetric padding 15 left
  // + 21 right, plus a 2px safety margin so content never overflows at 15%); recomputed on resize; ProjectSidebar infers container width from -38
  onMount(() => {
    const updateRevealMin = () => {
      const base = chatPageRef?.clientWidth ?? window.innerWidth;
      setLeftRevealMinPx(Math.max(base * 0.15 - 38, 120));
    };
    updateRevealMin();
    window.addEventListener('resize', updateRevealMin);
    onCleanup(() => window.removeEventListener('resize', updateRevealMin));
    onCleanup(() => {
      if (leftSnapRaf !== undefined) cancelAnimationFrame(leftSnapRaf);
    });
  });
  /**
   * Compute the right panel display width.
   * @returns {number} right panel width, or 0 when collapsed
   */
  const displayRightWidth = () => (isRightCollapsed() ? 0 : rightPanelWidth());

  /**
   * Set the left panel collapsed state and persist it (single entry point for collapse/expand, with width fallback).
   * @param collapsed - target collapsed state
   */
  const setLeftCollapsed = (collapsed: boolean) => {
    setIsLeftCollapsed(collapsed);
    localStorage.setItem('left-collapsed', String(collapsed));
    if (!collapsed && leftPanelWidth() < 5) {
      setLeftPanelWidth(18);
    }
  };

  /**
   * Toggle the left panel collapsed state.
   * @param {MouseEvent} e - mouse event
   */
  const toggleLeft = (e: MouseEvent) => {
    e.stopPropagation(); // prevent triggering the drag
    setLeftCollapsed(!isLeftCollapsed());
  };

  /**
   * Toggle the right panel collapsed state.
   * @param {MouseEvent} e - mouse event
   */
  const toggleRight = (e: MouseEvent) => {
    e.stopPropagation();
    const newState = !isRightCollapsed();
    setIsRightCollapsed(newState);
    localStorage.setItem('right-collapsed', String(newState));
    if (!newState && rightPanelWidth() < 5) {
      setRightPanelWidth(18);
    }
  };

  /**
   * Currently selected assistant object.
   * @returns Assistant | undefined
   */
  const currentAssistant = () => datas.assistants.find((a) => a.id === currentAssistantId());

  /**
   * Currently active topic object.
   * @returns Topic | null
   */
  const activeTopic = () => {
    const asst = currentAssistant();
    if (!asst) return null;
    return asst.topics.find((t: Topic) => t.id === currentTopicId()) || asst.topics[0] || null;
  };

  /** Topic shown in the share modal: prefer the ID set from the TopicSidebar context menu, otherwise the active topic */
  const shareTopic = (): Topic | null => {
    const asst = currentAssistant();
    if (!asst) return null;
    const sid = activeShareTopicId();
    if (sid) return asst.topics.find((t: Topic) => t.id === sid) || null;
    return activeTopic();
  };

  // -------- message-level selection --------

  /** Enter selection mode */
  const enterSelectionMode = (topicId?: string | null) => {
    if (topicId) {
      setActiveShareTopicId(topicId);
      setCurrentTopicId(topicId);
    }
    setSelectedMessageIds(new Set<string>());
    setIsSelectingMessages(true);
  };

  /** Toggle the selected state of one message */
  const handleToggleMessage = (msgId: string) => {
    setSelectedMessageIds((prev) => {
      const next = new Set<string>(prev);
      if (next.has(msgId)) next.delete(msgId);
      else next.add(msgId);
      return next;
    });
  };

  /** Select all exportable messages in the current topic (excluding tool messages) */
  const handleSelectAll = () => {
    const topic = activeTopic();
    if (!topic) return;
    const ids = new Set<string>();
    for (const msg of topic.history) {
      if (msg.role !== 'tool' && msg.id) ids.add(msg.id);
    }
    setSelectedMessageIds(ids);
  };

  /** Cancel the selection and exit selection mode */
  const handleCancelSelection = () => {
    setIsSelectingMessages(false);
    setSelectedMessageIds(new Set<string>());
    setActiveShareTopicId(null);
  };

  /** Confirm the selection and open the share modal */
  const handleConfirmSelection = () => {
    setIsSelectingMessages(false);
    setShowShareModal(true);
  };

  /**
   * Handle file upload and parsing.
   * Calls the Tauri backend to read local file content; supports text files and images.
   * Hardening: the frontend pre-validates the extension whitelist to avoid pointless calls.
   * @param filePath - absolute path of the file
   * @param fileType - file type hint ('file' or 'image')
   */
  const handleFileUpload = async (filePath: string, fileType: 'file' | 'image') => {
    const fileName = filePath.split(/[\\/]/).pop() || t('chat.unknownFile');
    const ext = (fileName.split('.').pop() || '').toLowerCase();
    const ALLOWED_IMG = ['png', 'jpg', 'jpeg', 'webp'];
    const ALLOWED_DOC = [
      'pdf',
      'docx',
      'pptx',
      'txt',
      'md',
      'json',
      'csv',
      'log',
      'xml',
      'yaml',
      'yml',
      'ini',
      'tsv',
      // source code / text
      'rs',
      'c',
      'h',
      'cpp',
      'hpp',
      'cc',
      'cxx',
      'cs',
      'go',
      'java',
      'rb',
      'py',
      'js',
      'mjs',
      'cjs',
      'ts',
      'tsx',
      'jsx',
      'php',
      'swift',
      'kt',
      'kts',
      'scala',
      'lua',
      'sql',
      'toml',
      'sh',
      'bash',
      'zsh',
      'dart',
      'html',
      'css',
      'scss',
      'less',
      'vue',
      'svelte',
      'gradle',
      'properties',
      'r',
      'pl',
    ];
    const isImg = fileType === 'image' || ALLOWED_IMG.includes(ext);
    const isDoc = ALLOWED_DOC.includes(ext);
    if (!isImg && !isDoc) {
      alert(
        t('chat.unsupportedFile', {
          extension: ext,
          supported: [...ALLOWED_IMG, ...ALLOWED_DOC].join(', '),
        }),
      );
      return;
    }
    setIsProcessing(true);
    try {
      const stored = await storeChatAttachment(filePath);
      setPendingFiles((prev) =>
        prev.some((file) => file.id === stored.id)
          ? prev
          : [
              ...prev,
              {
                ...stored,
                name: fileName,
                type: isImg ? 'image' : 'text',
                previewUrl: isImg ? convertFileSrc(stored.storagePath) : undefined,
              },
            ],
      );
    } catch (err) {
      alert(err);
    } finally {
      setIsProcessing(false);
    }
  };

  /**
   * Check and summarize conversation history (token-based).
   * Automatic path (conversation-end event): compress when usage exceeds 75% of the model context window;
   * manual path (/compact, force=true): skips the threshold guard and compresses immediately (mainstream agent behavior).
   * On summarize failure, falls back to local archiving (snapcompact-style): early context is not lost and can be read back via read_artifact.
   */
  const checkAndSummarize = async (
    keepBudgetOverride?: number,
    opts?: { force?: boolean },
  ): Promise<CompactOutcome> => {
    const topic = activeTopic();
    const currentMdl = selectedModel();
    if (!topic || !currentMdl) return { kind: 'skipped', reason: 'nothing-to-compact' };

    // get the current model's context window size
    const maxContext = await getModelContextWindow();

    // Measure context usage: prefer the context peak of the last assistant message
    // (contextTokens already includes all history + system prefix + output; do not double-count user messages)
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
    // automatic path keeps the threshold and minimum-history guards; manual /compact always passes (summarizeCount decides whether anything is compressible)
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

    // Token-budget retention: by default keep ~20k tokens accumulated from the tail (at least the
    // last 4 messages, i.e. one full turn); `/compact <n>` overrides the keep budget for this run
    const KEEP_TOKEN_BUDGET =
      keepBudgetOverride && keepBudgetOverride > 0 ? keepBudgetOverride : 20_000;
    const estimateMsgTokens = (m: any): number => {
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
    // Overwriting summary: the old summary is injected as input context and replaced wholesale by the new one, avoiding concatenation bloat
    const messagesToSummarize = topic.summary
      ? [
          { role: 'system' as const, content: t('chat.historyBackground') + topic.summary },
          ...topic.history.slice(0, summarizeCount),
        ]
      : topic.history.slice(0, summarizeCount);

    // Persist: trim the history and replace topic.summary with the new summary or archive placeholder
    const persistCompactedTopic = async (newSummary: string): Promise<CompactOutcome> => {
      const latestTopic = activeTopic();
      if (!latestTopic) return { kind: 'skipped', reason: 'nothing-to-compact' };
      const updatedHistory = latestTopic.history.slice(summarizeCount);
      setDatas(
        'assistants',
        (a) => a.id === currentAssistantId(),
        'topics',
        (t) => t.id === latestTopic.id,
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
      // summarize failed -> local archive fallback (zero API cost; in Agent mode the read_artifact tool can read the original back via the placeholder ID)
      try {
        const placeholder = await archiveHistory(
          JSON.stringify(messagesToSummarize),
          currentMdl.model_id,
          locale(),
        );
        const outcome = await persistCompactedTopic(placeholder);
        return outcome.kind === 'compressed'
          ? { kind: 'archived', count: summarizeCount }
          : outcome;
      } catch (archiveErr) {
        console.error('生成总结失败:', e, '归档亦失败:', archiveErr);
        return { kind: 'failed', error: String(e) };
      }
    }
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
        text = (firstUser.content as any[])
          .filter((p: any) => p?.type === 'text' && p?.text)
          .map((p: any) => p.text)
          .join(' ');
      } catch {
        text = '';
      }
    }
    // strip newlines so the title stays on one line
    const cleaned = text.trim().replace(/\s+/g, ' ');
    return cleaned.length > 12 ? cleaned.slice(0, 12) : cleaned;
  };

  /**
   * Auto-rename non-default topics after the first conversation turn.
   * Rules:
   *   1. Skip the default topic (first entry in each assistant's topic list)
   *   2. Skip already-renamed topics (topic.renamed === true)
   *   3. Skip empty or errored AI replies
   *   4. Prefer LLM-generated titles; fall back to the heuristic on failure (first 12 chars of the first user message)
   *   5. On success update topic.name, set renamed=true, and persist to the backend
   *   6. Cancel unfinished callback handling via activeTitleGen when the user switches topics
   */
  const checkAndRename = async (eventAsstId: string, eventTopicId: string) => {
    // cancel any unfinished title-generation task (the user switched topics or triggered a new rename)
    if (activeTitleGen) {
      activeTitleGen.cancelled = true;
      activeTitleGen = null;
    }
    const task = { topicId: eventTopicId, cancelled: false };
    activeTitleGen = task;

    const asst = datas.assistants.find((a: any) => a.id === eventAsstId);
    if (!asst) {
      if (activeTitleGen === task) activeTitleGen = null;
      return;
    }
    // must re-locate via the event's topicId (the user may have switched topics)
    const topic = (asst.topics as Topic[]).find((t: Topic) => t.id === eventTopicId);
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
    const latestAsst = datas.assistants.find((a: any) => a.id === eventAsstId);
    const latestTopic = latestAsst?.topics.find((t: Topic) => t.id === eventTopicId);
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
        (t) => t.id === eventTopicId,
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
        (t) => t.id === eventTopicId,
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
      (t) => t.id === eventTopicId,
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
   * Truncate oversized tool results (head+tail retention, aligned with the backend truncate_tool_result:
   * 60% head + 40% tail, 20k chars), preventing LLM context bloat while preserving the file head/tail
   * that read_file already kept. Full content stays in agentSteps for the user.
   */
  const truncateToolResult = (content: string): string => {
    const MAX_LEN = 20000;
    if (content.length <= MAX_LEN) return content;
    const head = Math.floor((MAX_LEN * 6) / 10);
    const tail = MAX_LEN - head;
    const headEnd = head;
    const tailStart = content.length - tail;
    if (tailStart > headEnd) {
      return (
        content.slice(0, headEnd) +
        `\n\n${t('chat.truncatedHeadTail', { total: content.length })}\n\n` +
        content.slice(tailStart)
      );
    }
    return content.slice(0, MAX_LEN) + `\n\n${t('chat.truncatedTail', { total: content.length })}`;
  };

  /**
   * Expand one history message into API payload messages (one or more: assistant + its tool results).
   *
   * Key defense: orphan toolCalls stuck in state 'calling' (user interrupt / app killed / legacy data)
   * are synthesized in place as interrupted-error tool results so every tool_calls has a paired
   * role:tool response; otherwise strictly-validated APIs return 400 (missing field
   * tool_call_id / insufficient tool messages).
   */
  function buildApiMessages(m: any): any[] {
    const obj: any = { role: m.role, content: m.content };
    if (m.images?.length) obj.images = m.images; // the backend strips the aio-image marker from this and expands image_url blocks
    if (m.displayFiles?.length) obj.displayFiles = m.displayFiles; // attachments (incl. images); the backend reads these and expands image_url blocks
    if (m.toolCallId) obj.toolCallId = m.toolCallId;
    if (m.name) obj.name = m.name;
    if (!m.toolCalls || m.toolCalls.length === 0) return [obj];

    // entries without an ID cannot be paired; drop them (avoids JSON.stringify dropping undefined fields and causing 400)
    const validCalls = m.toolCalls.filter((tc: any) => tc.id != null);
    if (validCalls.length === 0) return [obj];

    obj.toolCalls = validCalls.map((tc: any) => ({
      id: tc.id,
      type: tc.type || 'function',
      function: { name: tc.function?.name, arguments: tc.function?.arguments },
    }));
    const toolMsgs = validCalls.map((tc: any) => {
      // orphan (calling / no state): treat as interrupted, synthesize an error result
      const interrupted = tc.state !== 'success' && tc.state !== 'error';
      const raw = interrupted
        ? TOOL_INTERRUPTED
        : tc.state === 'error'
          ? (tc.error ?? tc.content ?? '[Tool Error]')
          : (tc.content ?? '');
      return {
        role: 'tool' as const,
        toolCallId: tc.id,
        name: tc.function?.name,
        content: truncateToolResult(raw),
      };
    });
    return [obj, ...toolMsgs];
  }

  /**
   * Ensure every assistant(tool_calls) in messagesForAI has matching role:tool messages.
   * If the tool response for a tool_call_id is missing, auto-fill a placeholder message
   * to prevent the API 400 "insufficient tool messages" error.
   */
  function ensureToolMessagesComplete(msgs: any[]): any[] {
    const result: any[] = [];
    let pendingToolCallIds: string[] = [];

    // fill placeholder tool messages for all unpaired toolCallIds in pending
    const flushPending = (target: any[]) => {
      for (const id of pendingToolCallIds) {
        console.warn(`[ensureToolMessages] 补全缺失的 tool 响应: ${id}`);
        target.push({
          role: 'tool' as const,
          toolCallId: id,
          name: '__pending__',
          content: '[System: tool result pending]',
        });
      }
      pendingToolCallIds = [];
    };

    for (const msg of msgs) {
      // on a new assistant(toolCalls), flush the previous unpaired toolCallIds first so the
      // previous pending set is not silently overwritten across consecutive tool-call turns
      if (msg.role === 'assistant' && msg.toolCalls?.length && pendingToolCallIds.length > 0) {
        flushPending(result);
      }

      result.push(msg);

      if (msg.role === 'assistant' && msg.toolCalls?.length) {
        // collect all toolCallIds declared by this assistant message
        pendingToolCallIds = msg.toolCalls.map((tc: any) => tc.id);
      } else if (msg.role === 'tool' && msg.toolCallId) {
        // a matching tool response was found; remove it
        const idx = pendingToolCallIds.indexOf(msg.toolCallId);
        if (idx !== -1) pendingToolCallIds.splice(idx, 1);
      }

      // on a user or assistant (no toolCalls) message, any unmatched toolCallId means the
      // previous assistant toolCalls lack sufficient tool responses — fill placeholder messages
      if (
        (msg.role === 'user' || (msg.role === 'assistant' && !msg.toolCalls?.length)) &&
        pendingToolCallIds.length > 0
      ) {
        flushPending(result);
      }
    }

    // final flush: fill placeholders for toolCallIds still unmatched at the end of the array
    if (pendingToolCallIds.length > 0) flushPending(result);

    return result;
  }

  /**
   * /btw side question: streams an answer over the existing conversation context (no tools, no history changes).
   * Listens to the `btw-chunk` event and updates the overlay chunk by chunk.
   * unlisten fires on the done/error event, avoiding a race between invoke resolve and event delivery.
   */
  const askBtwQuestion = async (question: string, overlayId: string) => {
    const currentMdl = selectedModel();
    const asstObj = currentAssistant();
    const topicObj = activeTopic();
    if (!currentMdl || !asstObj || !topicObj) return;

    let resolved = false;

    // register the listener before invoke so the first chunk is never lost
    const unlisten = await listen<{
      overlay_id: string;
      content: string;
      done: boolean;
      error?: string;
    }>(EVENTS.btwChunk, (e) => {
      const { overlay_id, content, done, error } = e.payload;
      if (overlay_id !== overlayId) return;

      if (error) {
        resolved = true;
        setBtwOverlays((prev) =>
          prev.map((o) =>
            o.id === overlayId ? { ...o, answer: `[Error] ${error}`, loading: false } : o,
          ),
        );
        unlisten();
        return;
      }
      if (done) {
        resolved = true;
        setBtwOverlays((prev) =>
          prev.map((o) => (o.id === overlayId ? { ...o, loading: false } : o)),
        );
        unlisten();
        return;
      }
      setBtwOverlays((prev) =>
        prev.map((o) => (o.id === overlayId ? { ...o, answer: (o.answer || '') + content } : o)),
      );
    });

    try {
      // dedicated BTW system prompt: forbid continuing the main task, answer from context only
      const btwSystemPrompt = `[SIDE QUESTION — DO NOT CONTINUE THE MAIN TASK]
You are answering a brief side question about the conversation above.
Do NOT continue, repeat, or redo any work from the conversation.
Only answer the specific question asked. Be concise.

${asstObj.prompt}`;

      const messagesForAI: Record<string, unknown>[] = [
        { role: 'system', content: btwSystemPrompt },
        ...topicObj.history.flatMap((m: any) => buildApiMessages(m)),
        { role: 'user', content: question },
      ];

      await askBtwQuestionCmd(llmCreds(currentMdl), overlayId, JSON.stringify(messagesForAI));
    } catch (err) {
      if (!resolved) {
        setBtwOverlays((prev) =>
          prev.map((o) =>
            o.id === overlayId ? { ...o, answer: `[Error] ${err}`, loading: false } : o,
          ),
        );
        unlisten();
      }
    }
  };

  /**
   * Slash command — promptBody type (/review, /explain, etc.)
   * Chat shows the raw command text; the resolved promptBody is sent to the LLM
   */
  const handleSlashPromptMessage = async (resolved: ReturnType<typeof resolveSlashCommand>) => {
    if (!resolved) return;
    const asstId = currentAssistantId();
    const topicId = currentTopicId();
    if (!asstId || !topicId) return;

    const currentMdl = selectedModel();
    const currentAsst = currentAssistant();
    const currentTopic = activeTopic();
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

    const newUserMsg = {
      id: crypto.randomUUID(),
      role: 'user' as const,
      content,
      displayText,
    };

    // build the full message array (same as the normal send flow)
    const reasoningPrompt = getReasoningPrompt(reasoningLevel());

    const agentMode = currentAsst?.agentMode || 'off';
    const pid = currentProjectId();
    const projectInfo =
      pid && currentProject()
        ? { path: currentProject()!.path, name: currentProject()!.name }
        : null;
    const agentPromptContent =
      agentMode !== 'off' && projectInfo
        ? buildAgentSystemPrompt(agentMode as any, projectInfo)
        : null;
    const agentSystemPrompt = agentPromptContent
      ? [{ role: 'system' as const, content: agentPromptContent }]
      : [];

    let messagesForAI: any[] = [
      { role: 'system', content: currentAsst.prompt },
      // output style constraints (emoji avoidance etc., mirroring mainstream agent system-prompt style sections)
      { role: 'system', content: t('chat.prompt.style') },
      ...buildSkillMessages(currentAsst, agentMode === 'off'),
      ...agentSystemPrompt,
      ...(reasoningPrompt ? [{ role: 'system', content: reasoningPrompt }] : []),
      ...(webSearchEnabled() ? [{ role: 'system', content: t('agent.prompt.web') }] : []),
      ...(currentTopic.summary
        ? [
            {
              role: 'system',
              content: t('agent.prompt.summary', { summary: currentTopic.summary }),
            },
          ]
        : []),
      ...currentTopic.history.flatMap((m: any) => buildApiMessages(m)),
      { role: 'user', content: newUserMsg.content },
    ];
    messagesForAI = ensureToolMessagesComplete(messagesForAI);

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
      (a: any) => a.id === asstId,
      'topics',
      (t: any) => t.id === topicId,
      'history',
      (h: any[]) => [...h, newUserMsg],
    );

    // call the LLM
    try {
      setIsThinking(true);
      await runAgentTurn({
        ...llmCreds(currentMdl),
        assistantId: asstId,
        topicId,
        messages: messagesForAI,
        mcpServerIds: currentAsst?.mcpServerIds ?? [],
        agentMode,
        projectId: currentProjectId() ?? null,
        webSearchEnabled: webSearchEnabled(),
        profileModelOverrides: [],
        customSubagentProfiles: [],
        locale: locale(),
        contextWindow: await getModelContextWindow(),
        skills: agentMode === 'off' ? [] : resolveAssistantSkills(currentAsst),
      });
    } catch (err) {
      alert(err);
      setIsThinking(false);
      setTypingIndex(null);
    }
  };

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
    const currentAsst = datas.assistants.find((a: Assistant) => a.id === asstId);
    const resumeAgentMode = currentAsst?.agentMode || 'off';
    setActiveCheckpoint(null);
    setIsThinking(true);
    try {
      await runAgentTurn({
        ...llmCreds(currentMdl),
        assistantId: asstId,
        topicId: tid,
        messages: [], // on resume the backend substitutes the checkpoint snapshot; frontend messages are unused
        mcpServerIds: currentAsst?.mcpServerIds ?? [],
        agentMode: resumeAgentMode,
        projectId: currentProjectId() ?? null,
        webSearchEnabled: webSearchEnabled(),
        profileModelOverrides: [],
        customSubagentProfiles: [],
        locale: locale(),
        contextWindow: await getModelContextWindow(),
        skills: resumeAgentMode === 'off' ? [] : resolveAssistantSkills(currentAsst),
        resumeTurnId: cp.turnId,
      });
    } catch (err) {
      alert(err);
      setIsThinking(false);
    }
  };

  /**
   * Slash command — pure action type (/compact, /clear, /search, /settings, /help)
   * Chat shows the command, runs the handler, then shows brief feedback
   */
  const handleSlashActionMessage = async (resolved: ReturnType<typeof resolveSlashCommand>) => {
    if (!resolved) return;
    const asstId = currentAssistantId();
    const topicId = currentTopicId();
    if (!asstId || !topicId) return;

    const cmd = resolved.command;
    const displayText = cmd.label;

    // append the user message
    setDatas(
      'assistants',
      (a: any) => a.id === asstId,
      'topics',
      (t: any) => t.id === topicId,
      'history',
      (h: any[]) => [
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
      (a: any) => a.id === asstId,
      'topics',
      (t: any) => t.id === topicId,
      'history',
      (h: any[]) => [
        ...h,
        {
          id: crypto.randomUUID(),
          role: 'assistant' as const,
          content: feedback,
          displayText: feedback,
        },
      ],
    );
  };

  /**
   * Send-message flow.
   * Covers file handling, API calls, and state updates
   */
  const handleSendMessage = async () => {
    const currentMdl = selectedModel();
    const topicObj = activeTopic();
    const asstObj = currentAssistant();

    // precondition check: model, topic, and assistant must exist, and the AI must not be generating
    if (!currentMdl || !topicObj || !asstObj) return;

    const userInput = inputMessage().trim();
    const files = pendingFiles();
    // block normal message sends while isThinking (/btw is handled separately below)
    if (isThinking() && !userInput.startsWith('/btw')) return;
    // require text input or file attachments
    if (!userInput && files.length === 0) return;

    // ---- slash command interception ----
    if (userInput.startsWith('/')) {
      const resolved = resolveSlashCommand(userInput);
      if (resolved) {
        // /btw handled separately: answer immediately from the current context without interrupting the main agent
        if (resolved.command.id === 'slash-btw') {
          const question = resolved.args || '';
          const id = crypto.randomUUID();
          setBtwOverlays((prev) => [...prev, { id, question, answer: '', loading: true }]);
          askBtwQuestion(question, id);
          setInputMessage('');
          setPendingFiles([]);
          return;
        }
        if (resolved.resolvedBody) {
          handleSlashPromptMessage(resolved);
        } else {
          handleSlashActionMessage(resolved);
        }
        setInputMessage('');
        setPendingFiles([]);
        return;
      }
      // unrecognized slash command — show feedback and do not send to the LLM
      const unknownAsstId = currentAssistantId();
      const unknownTopicId = currentTopicId();
      if (unknownAsstId && unknownTopicId) {
        setDatas(
          'assistants',
          (a: any) => a.id === unknownAsstId,
          'topics',
          (t: any) => t.id === unknownTopicId,
          'history',
          (h: any[]) => [
            ...h,
            {
              id: crypto.randomUUID(),
              role: 'user' as const,
              content: userInput,
              displayText: userInput,
            },
            {
              id: crypto.randomUUID(),
              role: 'assistant' as const,
              content: t('slash.feedback.unknown', { command: userInput }),
            },
          ],
        );
      }
      setInputMessage('');
      setPendingFiles([]);
      return;
    }

    const asstId = currentAssistantId();
    const topicId = currentTopicId();
    if (!asstId || !topicId) return;

    // ---- auto vision analysis ----
    // When the selected model cannot see images, the vision agent looks at
    // them first: its structured description replaces the raw image parts
    // (a non-vision model would ignore or reject the base64 payloads). The
    // image attachments are unlinked (id stripped) so later turns never
    // re-embed them; the history bubbles still show the thumbnails (they
    // render from storagePath, not the id).
    let outgoingText = userInput;
    // Stripped image entries carry an optional id (AttachmentMeta-level):
    // message_for_api skips id-less files, so later turns never re-embed them.
    let outgoingFiles: Array<Omit<PendingAttachment, 'id'> & { id?: string }> = files;
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

    const currentAsst = currentAssistant();
    const currentTopic = activeTopic();
    if (!currentAsst || !currentTopic) return;

    /** Inject the system prompt matching the reasoning level so the model uses <think>...</think> to emit its thinking output */
    const reasoningPrompt = getReasoningPrompt(reasoningLevel());

    const agentMode = currentAsst?.agentMode || 'off';
    const pid = currentProjectId();
    const projectInfo =
      pid && currentProject()
        ? { path: currentProject()!.path, name: currentProject()!.name }
        : null;
    const agentPromptContent =
      agentMode !== 'off' && projectInfo
        ? buildAgentSystemPrompt(agentMode as any, projectInfo)
        : null;
    const agentSystemPrompt = agentPromptContent
      ? [{ role: 'system' as const, content: agentPromptContent }]
      : [];
    let messagesForAI: any[] = [
      { role: 'system', content: currentAsst.prompt },
      // output style constraints (emoji avoidance etc., mirroring mainstream agent system-prompt style sections)
      { role: 'system', content: t('chat.prompt.style') },
      ...buildSkillMessages(currentAsst, isChatMode()),
      ...agentSystemPrompt,
      ...(reasoningPrompt ? [{ role: 'system', content: reasoningPrompt }] : []),
      ...(webSearchEnabled() ? [{ role: 'system', content: t('agent.prompt.web') }] : []),
      ...(currentTopic.summary
        ? [
            {
              role: 'system',
              content: t('agent.prompt.summary', { summary: currentTopic.summary }),
            },
          ]
        : []),
      ...currentTopic.history.flatMap((m: any) => buildApiMessages(m)),
      { role: 'user', content: newUserMsg.content, displayFiles: newUserMsg.displayFiles },
    ];
    // safety net: ensure every assistant(tool_calls) has matching role:tool messages
    messagesForAI = ensureToolMessagesComplete(messagesForAI);

    const lastMsg = messagesForAI[messagesForAI.length - 1];
    if (lastMsg.role !== 'user') {
      console.error('错误：发送给 API 的最后一条消息不是 User!', lastMsg);
      return;
    }

    // persist the user message and attachment links first so exiting during streaming or duplicate uploads does not orphan attachments.
    try {
      await appendMessage(topicId, newUserMsg);
    } catch (err) {
      console.error('save_message_with_attachments failed:', err);
      alert(t('error.save'));
      return;
    }

    // update the local Store: append the user message; in chat mode also create an empty assistant placeholder
    // (in project/Agent mode the placeholder is created by the llm-round-start event)
    if (isChatMode()) {
      setDatas(
        'assistants',
        (a) => a.id === asstId,
        'topics',
        (t) => t.id === topicId,
        'history',
        (h) => [
          ...h,
          newUserMsg,
          {
            id: crypto.randomUUID(),
            role: 'assistant' as const,
            content: '',
            modelId: currentMdl?.model_id,
            reasoning: '',
            agentStartTime: Date.now(),
            agentSteps: [] as any[],
          },
        ],
      );
    } else {
      setDatas(
        'assistants',
        (a) => a.id === asstId,
        'topics',
        (t) => t.id === topicId,
        'history',
        (h) => [...h, newUserMsg],
      );
    }

    // clear input state and file list, mark as generating
    setInputMessage('');
    setPendingFiles([]);
    setIsThinking(true);

    // set the typewriter index (position of the assistant placeholder just created in chat mode)
    if (isChatMode()) {
      const asst = datas.assistants.find((a) => a.id === asstId);
      const topic = asst?.topics.find((t: Topic) => t.id === topicId);
      if (topic) setTypingIndex(topic.history.length - 1);
    }

    try {
      // chat mode: plain conversation, calls call_llm_stream directly (no agent loop, no tools)
      // project/Agent mode: calls run_agent_turn (with tool calls, subagents, etc.)
      if (isChatMode() && isImageGenModel(currentMdl.model_id, currentMdl.api_url)) {
        // dedicated endpoints (dall-e-2/3 etc.): non-streaming generation, the result comes back via llm-chunk(done)
        await generateImage(llmCreds(currentMdl), asstId, topicId, userInput);
      } else if (isChatMode()) {
        await callLlmStream(llmCreds(currentMdl), asstId, topicId, messagesForAI);
      } else {
        // resolve per-profile model overrides (only needed in agent mode)
        const resolvedOverrides: Array<{
          profileId: string;
          modelId: string;
          apiUrl: string;
          apiKey: string;
        }> = [];
        for (const [profileId, mKey] of Object.entries(profileModelOverrides())) {
          if (!mKey) continue;
          const mdl = allAvailableModels().find((m) => modelKey(m) === mKey);
          if (mdl) {
            resolvedOverrides.push({
              profileId,
              modelId: mdl.model_id,
              apiUrl: mdl.api_url,
              apiKey: mdl.api_key ?? '',
            });
          }
        }

        await runAgentTurn({
          ...llmCreds(currentMdl),
          assistantId: asstId,
          topicId,
          messages: messagesForAI,
          mcpServerIds: currentAsst?.mcpServerIds ?? [],
          agentMode,
          projectId: currentProjectId() ?? null,
          webSearchEnabled: webSearchEnabled(),
          profileModelOverrides: resolvedOverrides,
          customSubagentProfiles: customSubagentProfiles(),
          locale: locale(),
          contextWindow: await getModelContextWindow(),
          skills: isChatMode() ? [] : resolveAssistantSkills(currentAsst),
        });
      }
    } catch (err) {
      alert(err);
      setIsThinking(false);
      setTypingIndex(null);
    }
  };

  /**
   * Stop the current AI generation.
   */
  const handleStopGeneration = async () => {
    try {
      await stopLlmStream(currentAssistantId() ?? '', currentTopicId() ?? '');
    } catch (err) {
      console.error('stop_llm_stream 失败:', err);
    }
    setPendingApprovals([]);
  };

  /** Switch to chat mode */
  const switchToChat = () => {
    setCurrentAssistantId(DEFAULT_ASST_ID);
    const asst = datas.assistants.find((a) => a.id === DEFAULT_ASST_ID);
    if (asst?.topics?.length) {
      setCurrentTopicId(asst.topics[0].id);
    }
  };

  /** Switch to the given project */
  const switchToProject = async (projectId: string) => {
    try {
      const asstId = await ensureProjectAssistant(projectId);
      setCurrentAssistantId(asstId);
      const asst = datas.assistants.find((a) => a.id === asstId);
      if (asst?.topics?.length) {
        setCurrentTopicId(asst.topics[0].id);
      }
    } catch (e) {
      console.error('切换项目失败:', e);
    }
  };

  /**
   * Add a new topic to the current assistant.
   * Creates the topic and makes it the selected one
   */
  const addTopic = async () => {
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
  const branchFromMessage = async (sourceMessageId: string) => {
    const asstId = currentAssistantId();
    const topicId = currentTopicId();
    if (!asstId || !topicId) return;
    try {
      const newTopic: Topic = await branchTopic(topicId, sourceMessageId);
      setDatas(
        'assistants',
        (a: any) => a.id === asstId,
        'topics',
        (prev: Topic[]) => [...prev, newTopic],
      );
      setCurrentTopicId(newTopic.id);
      await saveSingleAssistantToBackend(asstId);
    } catch (e) {
      console.error('分支创建失败:', e);
    }
  };

  /**
   * Generic drag-to-resize: binds mousemove/mouseup and updates width via a percentage callback.
   * @param e - MouseEvent mouse event
   * @param onDragMove - callback per mouse move, receives the percentage of the cursor across the full page width
   */
  const attachResizeDrag = (e: MouseEvent, onDragMove: (pct: number) => void) => {
    e.preventDefault();
    setIsResizing(true);

    const totalW = chatPageRef?.offsetWidth ?? window.innerWidth;
    const handleMove = (moveEvent: MouseEvent) => {
      onDragMove((moveEvent.clientX / totalW) * 100);
    };

    const stopResize = () => {
      setIsResizing(false);
      document.removeEventListener('mousemove', handleMove);
      document.removeEventListener('mouseup', stopResize);
    };

    document.addEventListener('mousemove', handleMove);
    document.addEventListener('mouseup', stopResize);
  };

  /**
   * Drag-resize the left panel: strict 1:1 cursor tracking in both expand and collapse directions; snap only on release.
   *
   * - Tracking origin: map the panel's "currently occupied pixel width" onto the cursor axis — the rail
   *   width when collapsed, the DOM-measured rendered width when expanded (the handle sits at the
   *   panel's right edge, so measuring guarantees zero first-frame displacement and no jump).
   *   Width delta = cursor delta, strictly 1:1;
   * - Percentage conversion uses `clientWidth` (content width), matching how CSS `width:%` resolves;
   * - Minimum width: tracking width never drops below the collapsed rail width, so the handle cannot
   *   be dragged to the far left and content never overflows;
   * - Release snap: pinned at the minimum or still inside the deadzone -> collapse; otherwise land at >= 15% and remember.
   *
   * @param e - MouseEvent mouse event
   */
  const startLeftResize = (e: MouseEvent) => {
    // user re-grabbed the handle: cancel the in-flight snap animation, the gesture takes over
    if (leftSnapRaf !== undefined) {
      cancelAnimationFrame(leftSnapRaf);
      leftSnapRaf = undefined;
    }
    const cssBase = chatPageRef?.clientWidth ?? window.innerWidth;
    const wasCollapsed = isLeftCollapsed();
    const doCollapse = LEFT_RAIL_WIDTH_PX / cssBase; // narrowest tracking width (%)
    // panel's current occupied pixel width: rail width when collapsed; measured when expanded (falls back to the remembered width if unmeasurable)
    const handleEl = e.currentTarget instanceof Element ? e.currentTarget : null;
    const measuredPx = handleEl?.parentElement?.getBoundingClientRect().width ?? 0;
    const basePx = wasCollapsed
      ? LEFT_RAIL_WIDTH_PX
      : measuredPx > 0
        ? measuredPx
        : (leftPanelWidth() / 100) * cssBase;
    // cursor axis zero point: cursor X minus this equals the width the panel should occupy
    const anchorX = e.clientX - basePx;
    setIsResizing(true);
    e.preventDefault();

    const handleMove = (moveEvent: MouseEvent) => {
      const pct = ((moveEvent.clientX - anchorX) / cssBase) * 100;
      setLeftDragWidth(Math.min(Math.max(pct, doCollapse), LEFT_MAX_WIDTH_PCT));
    };

    const stopResize = () => {
      document.removeEventListener('mousemove', handleMove);
      document.removeEventListener('mouseup', stopResize);
      const dragged = leftDragWidth();
      if (dragged === null) {
        setIsResizing(false);
        return; // pressed without dragging: keep as-is
      }
      const draggedPx = (dragged / 100) * cssBase;
      if (draggedPx <= LEFT_RAIL_WIDTH_PX + LEFT_RELEASE_DEADZONE_PX) {
        // pinned at the minimum / never really opened: snap back to the collapsed rail (via the CSS collapse transition)
        setLeftDragWidth(null);
        setIsResizing(false);
        setLeftCollapsed(true);
        return;
      }
      // Opened: ease the preview width to the snap target (>= 15%) with rAF.
      // Mask/padding/opacity follow each frame's width, so the ending has no hard clip or jump;
      // isResizing stays true (CSS transitions disabled) during it to avoid double smoothing.
      const target = Math.max(dragged, 15);
      const durationMs = 250;
      const start = performance.now();
      const step = (now: number) => {
        const t = Math.min((now - start) / durationMs, 1);
        const eased = 1 - Math.pow(1 - t, 3); // easeOutCubic
        setLeftDragWidth(dragged + (target - dragged) * eased);
        if (t < 1) {
          leftSnapRaf = requestAnimationFrame(step);
          return;
        }
        leftSnapRaf = undefined;
        setLeftPanelWidth(target);
        setLeftCollapsed(false);
        setLeftDragWidth(null);
        setIsResizing(false);
      };
      leftSnapRaf = requestAnimationFrame(step);
    };

    document.addEventListener('mousemove', handleMove);
    // listen for mouseup immediately: a press-and-release on the handle without any mousemove still completes the snap
    document.addEventListener('mouseup', stopResize);
  };

  /**
   * Drag-resize the right panel (original behavior: only available when expanded).
   * @param e - MouseEvent mouse event
   */
  const startResizeRight = (e: MouseEvent) => {
    attachResizeDrag(e, (pct) => {
      setRightPanelWidth(Math.min(Math.max(100 - pct, 15), 30));
    });
  };

  onMount(() => {
    const navigate = useNavigate();

    // register all keyboard/slash commands (19 registerCommand calls, extracted to hooks/chatCommands.ts).
    const commandIds = registerChatCommands({
      isLeftCollapsed,
      setLeftCollapsed,
      isRightCollapsed,
      setIsRightCollapsed,
      rightPanelWidth,
      setRightPanelWidth,
      addTopic,
      setShowProjectCreateModal,
      switchToChat,
      switchToProject,
      isThinking,
      setIsThinking,
      handleStopGeneration,
      reasoningLevel,
      setReasoningLevel,
      webSearchEnabled,
      setWebSearchEnabled,
      datas,
      setDatas,
      projects,
      currentAssistant,
      currentAssistantId,
      currentTopicId,
      setCurrentTopicId,
      saveSingleAssistantToBackend,
      checkAndSummarize,
      navigate,
    });

    // ---- pre-existing initialization logic ----
    // first entry: load all assistant data from SQLite
    loadAssistants()
      .then(async (loaded) => {
        let finalAssistants = [...loaded];

        // 1. ensure the default assistant exists
        let defaultAsst = finalAssistants.find((a) => a.id === DEFAULT_ASST_ID);
        if (!defaultAsst) {
          // createAssistant already includes one "default topic"
          defaultAsst = createAssistant(t('chat.defaultAssistant'), DEFAULT_ASST_ID);
          finalAssistants = [defaultAsst, ...finalAssistants];
          setDatas('assistants', finalAssistants);
          await saveSingleAssistantToBackend(DEFAULT_ASST_ID);
        } else {
          setDatas('assistants', finalAssistants);
        }

        // 2. apply the default selection on app startup (cold start only)
        if (isFirstAppLaunch) {
          // try to restore the last project selection; otherwise default to the chat assistant
          const lastProjectId = getLastAgentProjectId();
          const lastProjectValid = lastProjectId && projects().find((p) => p.id === lastProjectId);
          if (lastProjectValid) {
            void switchToProject(lastProjectId!);
          } else {
            setCurrentAssistantId(DEFAULT_ASST_ID);

            const asst = datas.assistants.find((a) => a.id === DEFAULT_ASST_ID);
            if (asst && asst.topics.length > 0) {
              setCurrentTopicId(asst.topics[0].id);
            } else if (asst) {
              const newDefaultTopic = createTopic(t('chat.defaultTopic'));
              setDatas('assistants', (a) => a.id === DEFAULT_ASST_ID, 'topics', [newDefaultTopic]);
              setCurrentTopicId(newDefaultTopic.id);
              await saveSingleAssistantToBackend(DEFAULT_ASST_ID);
            }
          }

          isFirstAppLaunch = false;
        }
        console.log('成功加载数据');
      })
      .catch((err) => {
        // backend errors are logged here
        console.error('加载助手列表失败:', err);
        console.error('load_all_assistants failed:', err);
        alert(t('error.load'));
      });

    // register all event listeners (20 listen() calls, extracted to hooks/chatEventListeners.ts;
    // the unlisten functions are stored for cleanup on unmount).
    const unlistens = registerChatEventListeners({
      setDatas,
      setIsDragging,
      setIsThinking,
      setPendingApprovals,
      setTypingIndex,
      setWorkflowState,
      setAgentTaskBoard,
      setPendingQuestions,
      handleFileUpload,
      datas,
      selectedModel,
      saveSingleAssistantToBackend,
      checkAndRename,
      checkAndSummarize,
    });

    // clean up all event listeners and command registrations on unmount
    onCleanup(() => {
      unlistens.forEach((u) => u.then((fn) => fn()));
      // unregister all registered commands (IDs are returned by registerChatCommands)
      commandIds.forEach(unregisterCommand);
    });
  });

  // watch for manually triggered "regenerate title" requests (from the TopicSidebar context menu);
  // the signal is cleared right after consumption to avoid accidental re-triggering
  createEffect(() => {
    const req = pendingRenameRequest();
    if (req) {
      setPendingRenameRequest(null);
      void checkAndRename(req.asstId, req.topicId);
    }
  });

  // exit the subagent conversation view when the topic switches (or the assistant switches, which implies a topic switch), returning to the main agent conversation
  createEffect(() => {
    currentTopicId();
    setActiveSubagentView(null);
  });

  // watch currentTopicId changes: if the user switches to a topic other than the one whose title
  // is being generated, abort that task's result handling
  let prevTitleGenTopicId: string | null = null;
  createEffect(() => {
    const tId = currentTopicId();
    // skip processing on first run (record only)
    if (prevTitleGenTopicId === null && activeTitleGen === null) {
      prevTitleGenTopicId = tId;
      return;
    }
    if (activeTitleGen && activeTitleGen.topicId !== tId) {
      activeTitleGen.cancelled = true;
      activeTitleGen = null;
    }
    prevTitleGenTopicId = tId;
  });
  createEffect(() => {
    localStorage.setItem('chat-left-panel-width', leftPanelWidth().toString());
  });
  createEffect(() => {
    localStorage.setItem('chat-right-panel-width', rightPanelWidth().toString());
  });

  // auto-detect and start language servers when the project switches
  createEffect(() => {
    const pid = currentProjectId();
    if (!pid) {
      // no project open; clear diagnostics
      clearAllDiagnostics();
      return;
    }
    const project = currentProject();
    if (!project) return;

    void (async () => {
      try {
        // auto-detect languages
        const result = await autoDetectLsp(project.path);
        const languages: Array<{ languageId: string }> = result?.languages || [];

        // start a language server for each detected language
        for (const lang of languages) {
          try {
            await startLspServer(project.path, lang.languageId);
          } catch (e) {
            // fail silently (the language server may not be installed)
            console.debug(`[LSP] 无法启动 ${lang.languageId} 服务器:`, e);
          }
        }
      } catch (e) {
        console.debug('[LSP] 自动检测失败:', e);
      }
    })();
  });

  // initialize MCP servers when the project switches (incl. auto-connect of the built-in
  // filesystem MCP); must wait for projects to load or currentProject is null and the MCP config is unreadable
  createEffect(() => {
    const pid = currentProjectId();
    const loaded = projects().length > 0 || !pid;
    if (pid && loaded) {
      void initMcpServers(pid);
    }
  });

  /**
   * Model follows the current assistant:
   * on assistant switch, assistant model binding change, or available model list change,
   * sync the global selectedModel to the assistant's resolved model.
   * When the assistant has no bound modelId, resolveAssistantModel falls back to the current
   * selectedModel, so no override happens. A null resolved value (no available models yet)
   * is skipped to avoid clearing it early at startup.
   */
  createEffect(() => {
    const id = currentAssistantId();
    const asst = datas.assistants.find((a: any) => a.id === id) as Assistant | undefined;
    // trigger deps: assistant modelId and the available model list (providerConfigs / activatedModels drive it in the store)
    void asst?.modelId;
    const resolved = resolveAssistantModel(asst ?? null);
    if (resolved && resolved.model_id !== selectedModel()?.model_id) {
      setSelectedModel(resolved);
    }
  });

  return (
    <div
      class="h-full flex gap-[3px] px-[6px] pt-[1px] pb-[6px]"
      style={{ background: 'transparent' }}
      classList={{ 'is-resizing': isResizing() }}
      ref={chatPageRef}
    >
      <ProjectSidebar
        width={displayLeftWidth()}
        isCollapsed={leftPanelCollapsed()}
        revealMinPx={leftRevealMinPx()}
        onToggle={toggleLeft}
        onResize={(e) => startLeftResize(e as MouseEvent)}
        isResizing={isResizing()}
        onOpenSettings={(id) => setSettingsAsstId(id)}
      />

      <div class="flex-1 flex flex-col min-w-0 min-h-0">
        {/* Subagent standalone conversation view: replaces the whole chat column (incl. status bar and input); the back button top-left returns to the main conversation */}
        <Show
          when={!activeSubagentView()}
          fallback={<SubagentConversationView topic={activeTopic()} />}
        >
          <Show when={workflowState()}>
            <WorkflowVisualization />
          </Show>

          {/* Agent task list panel (create_tasks / update_task live mirror) */}
          <TaskBoardPanel topicId={activeTopic()?.id ?? ''} />

          {/* Checkpoint resume entry: banner for crash-orphaned turns */}
          <Show when={activeCheckpoint() && !isThinking()}>
            <button
              type="button"
              class="flex items-center gap-2 px-4 py-1.5 text-xs border-b cursor-pointer bg-transparent hover:bg-white/[0.03] transition-colors"
              style={{ color: 'rgb(var(--primary-rgb))', 'border-color': 'var(--border-dim)' }}
              onClick={() => void handleResumeTurn()}
            >
              <Icon name="refresh" size={12} />
              <span>{t('agent.checkpoint.resume')}</span>
            </button>
          </Show>

          <ChatInterface
            activeTopic={activeTopic()}
            isThinking={isThinking()}
            isProcessing={isProcessing()}
            isDragging={isDragging()}
            typingIndex={typingIndex()}
            inputMessage={inputMessage()}
            setInputMessage={setInputMessage}
            pendingFiles={pendingFiles()}
            setPendingFiles={setPendingFiles}
            handleSendMessage={handleSendMessage}
            handleStopGeneration={handleStopGeneration}
            handleFileUpload={handleFileUpload}
            pendingApprovals={pendingApprovals()}
            onResolveApproval={(id) =>
              setPendingApprovals((prev) => prev.filter((a) => a.approvalId !== id))
            }
            pendingQuestions={pendingQuestions()}
            onResolveQuestion={(id) =>
              setPendingQuestions((prev) => prev.filter((q) => q.requestId !== id))
            }
            canShare={!!activeTopic()}
            onOpenShare={() => enterSelectionMode(null)}
            isSelectingMessages={isSelectingMessages()}
            selectedMessageIds={selectedMessageIds()}
            onToggleMessage={handleToggleMessage}
            onSelectAll={handleSelectAll}
            onCancelSelection={handleCancelSelection}
            onConfirmSelection={handleConfirmSelection}
            onBranchFromMessage={branchFromMessage}
            btwOverlays={btwOverlays()}
            onDismissBtw={(id) => setBtwOverlays((prev) => prev.filter((o) => o.id !== id))}
          />
        </Show>
      </div>

      <TopicSidebar
        width={displayRightWidth()}
        isCollapsed={isRightCollapsed()}
        onToggle={toggleRight}
        onResize={(e) => !isRightCollapsed() && startResizeRight(e)}
        currentAssistant={currentAssistant()}
        editingTopicId={editingTopicId()}
        setEditingTopicId={setEditingTopicId}
        addTopic={addTopic}
        onExportTopic={(topicId) => enterSelectionMode(topicId)}
        isResizing={isResizing()}
      />

      <Portal>
        <ShareModal
          open={showShareModal()}
          onClose={() => {
            setShowShareModal(false);
            setActiveShareTopicId(null);
            setSelectedMessageIds(new Set<string>());
          }}
          topic={shareTopic()}
          selectedMessageIds={selectedMessageIds().size > 0 ? selectedMessageIds() : undefined}
        />
      </Portal>
      <Portal>
        <ProjectSettingsModal
          show={settingsAsstId() !== null}
          assistantId={settingsAsstId()}
          onClose={() => setSettingsAsstId(null)}
        />
      </Portal>

      <ProblemsPanel />
    </div>
  );
};

export default ChatPage;
