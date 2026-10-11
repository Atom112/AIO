import { Component, createSignal, onMount, onCleanup, createEffect, Show } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import {
  appendMessage,
  askBtwQuestion as askBtwQuestionCmd,
  branchTopic,
  callLlmStream,
  generateImage,
  getActiveCheckpoint,
  llmCreds,
  loadAssistants,
  runAgentTurn,
  stopActiveLlmStream,
  visionAnalyzeImages,
  type ActiveCheckpoint,
} from '../../core/api/chat';
import { storeChatAttachment } from '../../core/api/attachment';
import { registerChatUiListeners } from './hooks/chatEventListeners';
import {
  activeTitleGenTopicId,
  cancelActiveTitleGen,
  checkAndRename,
  checkAndSummarize,
  getModelContextWindow,
} from './hooks/chatTurnEffects';
import { registerChatCommands } from './hooks/chatCommands';
import { usePanelResize } from './hooks/usePanelResize';
// 阶段4 第2项（拆分切片）：API 载荷消息构建三件套抽至 utils/apiMessages（纯函数 + 11 单测）
import { buildApiMessages, ensureToolMessagesComplete } from './utils/apiMessages';
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
  webSearchEnabled,
  pendingRenameRequest,
  setPendingRenameRequest,
  resolveAssistantSkills,
  currentProjectId,
  setCurrentProjectId,
  currentProject,
  profileModelOverrides,
  allAvailableModels,
  customSubagentProfiles,
  workflowState,
  isChatMode,
  projects,
  ensureProjectAssistant,
  setShowProjectCreateModal,
  getLastAgentProjectId,
  initMcpServers,
  activeSubagentView,
  setActiveSubagentView,
  // CHAT-12: stream signals now live in core/store/stream.ts (module-level singletons)
  isThinking,
  setIsThinking,
  typingIndex,
  setTypingIndex,
  pendingApprovals,
  setPendingApprovals,
  pendingQuestions,
  setPendingQuestions,
  type AgentStep,
} from '../../core/store/store';
import { EVENTS } from '../../core/api/events';
// P3（localStorage 裸调收编）：面板布局持久化经 core/store/layout 域 API
import {
  loadPanelCollapsed,
  loadPanelWidth,
  savePanelCollapsed,
  savePanelWidth,
} from '../../core/store/layout';
import { buildAgentSystemPrompt } from '../../core/agent-prompts';
import { unregisterCommand, resolveSlashCommand } from '../../core/shortcuts';
import { locale, t } from '../../core/i18n';
import Icon from '../../shared/components/Icon';
import { isImageGenModel } from '../../core/utils/models';
import { modelSupportsVision, resolveVisionModel } from './vision';
import ProjectSidebar from './components/ProjectSidebar';
import ProjectSettingsModal from './components/ProjectSettingsModal';
import ProjectMcpTrustModal from './components/ProjectMcpTrustModal';
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

// getModelContextWindow (and the turn-finalization effects checkAndRename /
// checkAndSummarize) moved to hooks/chatTurnEffects.ts — CHAT-12: the global stream
// listeners (registered from Layout) call them after `done`, so they must be
// module-level singletons instead of page-scope closures.

import TaskBoardPanel from './components/TaskBoardPanel';
import { buildSkillMessages } from './utils/skillMessages';
import { clearAllDiagnostics } from '../../core/store/diagnostics';

let isFirstAppLaunch = true;
const DEFAULT_ASST_ID = 'default-assistant-id';

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
  const [leftPanelWidth, setLeftPanelWidth] = createSignal(loadPanelWidth('left')); // left panel width percentage (assistant list), default 18%, range 15%-30%
  const [rightPanelWidth, setRightPanelWidth] = createSignal(loadPanelWidth('right')); // right panel width percentage (topic list), default 18%, range 15%-30%
  const [isLeftCollapsed, setIsLeftCollapsed] = createSignal(loadPanelCollapsed('left')); // left/right panel width adjustment logic
  const [isRightCollapsed, setIsRightCollapsed] = createSignal(loadPanelCollapsed('right'));
  const [inputMessage, setInputMessage] = createSignal(''); // message text currently in the input box
  const [pendingFiles, setPendingFiles] = createSignal<PendingAttachment[]>([]); // files staged for sending (copied into the app attachment dir, not yet linked to a message)
  // CHAT-12: isThinking / typingIndex / pendingApprovals / pendingQuestions now live in
  // core/store/stream.ts — the global stream listeners drive them app-wide.
  const [isProcessing, setIsProcessing] = createSignal(false); // whether a file is being processed (drives the file-parsing loading state)
  const [isDragging, setIsDragging] = createSignal(false); // whether files are being dragged over the window (drives drag-state styling)
  const [editingTopicId, setEditingTopicId] = createSignal<string | null>(null); // ID of the topic whose name is being edited; null when not editing
  const [settingsAsstId, setSettingsAsstId] = createSignal<string | null>(null); // ID of the assistant whose settings modal is open; null when closed
  // tool calls awaiting user approval
  /** Share modal state */
  const [showShareModal, setShowShareModal] = createSignal(false);
  const [activeShareTopicId, setActiveShareTopicId] = createSignal<string | null>(null);
  /** Message-level selection mode */
  const [isSelectingMessages, setIsSelectingMessages] = createSignal(false);
  const [selectedMessageIds, setSelectedMessageIds] = createSignal<Set<string>>(new Set());
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
    savePanelCollapsed('left', collapsed);
    if (!collapsed && leftPanelWidth() < 5) {
      setLeftPanelWidth(18);
    }
  };

  // 阶段4 第2项（拆分切片）：面板拖拽/幕帘/吸附逻辑抽至 usePanelResize 钩子
  const { isResizing, leftDragWidth, leftRevealMinPx, startLeftResize, startResizeRight } =
    usePanelResize({
      chatPageRef: () => chatPageRef,
      isLeftCollapsed,
      setLeftCollapsed,
      leftPanelWidth,
      setLeftPanelWidth,
      rightPanelWidth,
      setRightPanelWidth,
      isRightCollapsed,
    });

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
    savePanelCollapsed('right', newState);
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

  // checkAndSummarize / fallbackTitle / checkAndRename moved to hooks/chatTurnEffects.ts (CHAT-12)

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
      console.error(
        'Assertion failed: the last message sent to the API is not a User message!',
        lastMsg,
      );
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
      // CHAT-10: the invoke failed after the placeholder was created (chat mode creates
      // it locally; agent mode creates it on llm-round-start). Without cleanup the
      // empty assistant bubble sat in a "thinking" state forever — no done event ever
      // arrives for a stream that never started.
      setDatas(
        'assistants',
        (a) => a.id === asstId,
        'topics',
        (t) => t.id === topicId,
        'history',
        (h: Message[]) => {
          const lastIdx = h.length - 1;
          if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
          const msg = h[lastIdx] as Message & {
            toolCalls?: unknown[];
            reasoning?: string;
            images?: unknown;
          };
          const steps = msg.agentSteps || [];
          const toolCalls = msg.toolCalls || [];
          const empty =
            !msg.content &&
            !msg.reasoning &&
            steps.length === 0 &&
            toolCalls.length === 0 &&
            !msg.images;
          if (empty) return h.slice(0, lastIdx); // drop the untouched placeholder
          // Partial stream preserved: close running steps / orphaned calling tools as interrupted
          const now = Date.now();
          const closedMsg = {
            ...msg,
            agentSteps: steps.map((s: AgentStep) => {
              if (s.status !== 'running') return s;
              const isCallingTool =
                s.type === 'tool_call' && (s as any).toolCall?.state === 'calling';
              return {
                ...s,
                status: isCallingTool ? ('error' as const) : ('complete' as const),
                duration: now - s.timestamp,
              };
            }),
            toolCalls: toolCalls.some((tc: any) => tc.state === 'calling')
              ? toolCalls.map((tc: any) =>
                  tc.state === 'calling' ? { ...tc, state: 'error', error: TOOL_INTERRUPTED } : tc,
                )
              : toolCalls,
          };
          return [...h.slice(0, lastIdx), closedMsg];
        },
      );
    }
  };

  /**
   * Stop the current AI generation.
   */
  const handleStopGeneration = async () => {
    try {
      // CHAT-4: 按流归属键停止——切话题后当前视图 ID 打不到流（后端按键控
      // 静默 no-op），用户被迫切回原话题才能停止
      await stopActiveLlmStream();
    } catch (err) {
      console.error('stop_llm_stream failed:', err);
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
      console.error('Failed to create branch:', e);
    }
  };

  /**
   * Generic drag-to-resize: binds mousemove/mouseup and updates width via a percentage callback.
   * @param e - MouseEvent mouse event
   * @param onDragMove - callback per mouse move, receives the percentage of the cursor across the full page width
   */
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
      webSearchEnabled,
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
        console.log('Data loaded successfully');
      })
      .catch((err) => {
        // backend errors are logged here
        console.error('Failed to load assistant list:', err);
        console.error('load_all_assistants failed:', err);
        alert(t('error.load'));
      });

    // register the page-scoped UI listeners (drag-and-drop only; CHAT-12: the 18
    // stream listeners now register once from the root Layout via
    // registerChatStreamListeners and live for the whole app lifetime — leaving the
    // chat page mid-generation no longer drops chunks or the final save).
    const unlistens = registerChatUiListeners({
      setIsDragging,
      handleFileUpload,
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
    const inFlightTopic = activeTitleGenTopicId();
    // skip processing on first run (record only)
    if (prevTitleGenTopicId === null && inFlightTopic === null) {
      prevTitleGenTopicId = tId;
      return;
    }
    if (inFlightTopic && inFlightTopic !== tId) {
      cancelActiveTitleGen();
    }
    prevTitleGenTopicId = tId;
  });
  createEffect(() => {
    savePanelWidth('left', leftPanelWidth());
  });
  createEffect(() => {
    savePanelWidth('right', rightPanelWidth());
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
      <Portal>
        <ProjectMcpTrustModal />
      </Portal>

      <ProblemsPanel />
    </div>
  );
};

export default ChatPage;
