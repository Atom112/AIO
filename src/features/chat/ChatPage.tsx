import { Component, createSignal, onMount, onCleanup, createEffect, Show } from 'solid-js';
import {
  appendMessage,
  callLlmStream,
  generateImage,
  llmCreds,
  loadAssistants,
} from '../../core/api/chat';
import { registerChatUiListeners } from './hooks/chatEventListeners';
import {
  activeTitleGenTopicId,
  cancelActiveTitleGen,
  checkAndRename,
  checkAndSummarize,
} from './hooks/chatTurnEffects';
import { registerChatCommands } from './hooks/chatCommands';
import { usePanelResize } from './hooks/usePanelResize';
import { useBtwQuestion } from './hooks/useBtwQuestion';
import { useAttachments } from './hooks/useAttachments';
import { useChatSend } from './hooks/useChatSend';
import {
  DEFAULT_ASST_ID,
  createTopic,
  handleStopGeneration,
  switchToChat,
  switchToProject,
  addTopic,
  branchFromMessage,
} from './utils/topicActions';
import { useMessageSelection } from './hooks/useMessageSelection';
import { useCheckpointResume } from './hooks/useCheckpointResume';
import { buildOutgoingMessages, resolveOutgoingContent } from './utils/sendMessagePrep';
import { handleSlashActionMessage, handleSlashPromptMessage } from './utils/slashHandlers';
import { cleanupFailedPlaceholder } from './utils/failedTurnCleanup';
import { autoDetectLsp, startLspServer } from '../../core/api/lsp';
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
  currentProject,
  profileModelOverrides,
  allAvailableModels,
  customSubagentProfiles,
  workflowState,
  isChatMode,
  projects,
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
  config,
} from '../../core/store/store';
// P3（localStorage 裸调收编）：面板布局持久化经 core/store/layout 域 API
import {
  loadPanelCollapsed,
  loadPanelWidth,
  savePanelCollapsed,
  savePanelWidth,
} from '../../core/store/layout';
import { unregisterCommand, resolveSlashCommand } from '../../core/shortcuts';
import { t } from '../../core/i18n';
import Icon from '../../shared/components/Icon';
import { isImageGenModel } from '../../core/utils/models';
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

// getModelContextWindow (and the turn-finalization effects checkAndRename /
// checkAndSummarize) moved to hooks/chatTurnEffects.ts — CHAT-12: the global stream
// listeners (registered from Layout) call them after `done`, so they must be
// module-level singletons instead of page-scope closures.

import { clearAllDiagnostics } from '../../core/store/diagnostics';

let isFirstAppLaunch = true;

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

  // 阶段4 第2项（拆分切片）：/btw 侧问浮层逻辑抽至 useBtwQuestion 钩子
  // （置于 currentAssistant/activeTopic 声明之后：依赖其响应式访问器）
  const { btwOverlays, openBtwQuestion, dismissBtwOverlay } = useBtwQuestion({
    selectedModel,
    currentAssistant,
    activeTopic,
  });

  // 阶段4 第2项（拆分切片）：附件暂存逻辑抽至 useAttachments 钩子
  const { handleFileUpload } = useAttachments({
    setPendingFiles,
    setIsProcessing,
  });
  // 阶段4 第2项（拆分切片）：共享的 run_agent_turn 载荷组装 + thinking 生命周期
  const { startAgentTurn } = useChatSend({
    selectedModel,
    currentAssistantId,
    currentTopicId,
    currentProjectId,
    webSearchEnabled,
    setIsThinking,
    setTypingIndex,
  });

  /** Topic shown in the share modal: prefer the ID set from the TopicSidebar context menu, otherwise the active topic */
  // 阶段4 第2项（<900 目标切片）：slash 处理器抽至 utils/slashHandlers（组件级依赖经此传递）
  const slashDeps = {
    currentAssistant,
    activeTopic,
    startAgentTurn,
  };

  // 阶段4 第2项（<900 目标切片）：选择模式家族 → useMessageSelection；
  // checkpoint 恢复入口 → useCheckpointResume（信号与效果随钩子走）
  const {
    isSelectingMessages,
    selectedMessageIds,
    showShareModal,
    shareTopic,
    enterSelectionMode,
    handleToggleMessage,
    handleSelectAll,
    handleCancelSelection,
    handleConfirmSelection,
    closeShareModal,
  } = useMessageSelection({ currentAssistant, activeTopic });
  const { activeCheckpoint, handleResumeTurn } = useCheckpointResume({ startAgentTurn });

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
          openBtwQuestion(question);
          setInputMessage('');
          setPendingFiles([]);
          return;
        }
        if (resolved.resolvedBody) {
          handleSlashPromptMessage(slashDeps, resolved);
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

    // ---- auto vision analysis + outgoing-message assembly ----
    // (extracted to utils/sendMessagePrep: resolveOutgoingContent + buildOutgoingMessages)
    const { outgoingText, outgoingFiles } = await resolveOutgoingContent(
      currentMdl,
      files,
      userInput,
      setIsThinking,
    );

    // Re-read after the vision await: the user may have switched topic/assistant meanwhile
    const currentAsst = currentAssistant();
    const currentTopic = activeTopic();
    if (!currentAsst || !currentTopic) return;

    const pid = currentProjectId();
    const { newUserMsg, messagesForAI } = buildOutgoingMessages({
      currentAsst,
      currentTopic,
      outgoingText,
      outgoingFiles,
      userInput,
      reasoningLevel: reasoningLevel(),
      webSearchEnabled: webSearchEnabled(),
      isChatMode: isChatMode(),
      projectInfo:
        pid && currentProject()
          ? { path: currentProject()!.path, name: currentProject()!.name }
          : null,
      mainAgentToolAccess:
        (config().mainAgentToolAccess as 'hybrid' | 'full' | 'orchestrator' | undefined) ||
        'hybrid',
    });

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
      const agentMode = currentAsst?.agentMode || 'off';
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

        // 阶段4 第2项（拆分切片）：共享载荷经 useChatSend 原语；CHAT-10 占位
        // 气泡清理经 onInvokeError（实现抽至 utils/failedTurnCleanup，含 6 单测）
        await startAgentTurn({
          messages: messagesForAI,
          agentMode,
          skills: isChatMode() ? [] : resolveAssistantSkills(currentAsst),
          profileModelOverrides: resolvedOverrides,
          customSubagentProfiles: customSubagentProfiles(),
          onInvokeError: () => {
            setDatas(
              'assistants',
              (a) => a.id === asstId,
              'topics',
              (t) => t.id === topicId,
              'history',
              (h: Message[]) => cleanupFailedPlaceholder(h),
            );
          },
        });
        return; // the primitive owns the failure teardown; skip the shared catch below
      }
    } catch (err) {
      alert(err);
      setIsThinking(false);
      setTypingIndex(null);
      // CHAT-10 (chat-mode callLlmStream/generateImage paths): same cleanup
      setDatas(
        'assistants',
        (a) => a.id === asstId,
        'topics',
        (t) => t.id === topicId,
        'history',
        (h: Message[]) => cleanupFailedPlaceholder(h),
      );
    }
  };

  /**
   * Stop the current AI generation.
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
    // first entry only: load all assistant data from SQLite. Re-loading on every mount
    // REPLACED the store with DB state — an in-flight streaming message (persisted only
    // at the turn's done epilogue) was wiped when the user left the chat page and came
    // back, making the streaming bubble vanish mid-generation. The 18 stream listeners
    // are global (CHAT-12, registered from the root Layout) and keep the store current
    // while away, so after cold start the store IS the live truth; skip the reload.
    if (isFirstAppLaunch) {
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
            const lastProjectValid =
              lastProjectId && projects().find((p) => p.id === lastProjectId);
            if (lastProjectValid) {
              void switchToProject(lastProjectId!);
            } else {
              setCurrentAssistantId(DEFAULT_ASST_ID);

              const asst = datas.assistants.find((a) => a.id === DEFAULT_ASST_ID);
              if (asst && asst.topics.length > 0) {
                setCurrentTopicId(asst.topics[0].id);
              } else if (asst) {
                const newDefaultTopic = createTopic(t('chat.defaultTopic'));
                setDatas('assistants', (a) => a.id === DEFAULT_ASST_ID, 'topics', [
                  newDefaultTopic,
                ]);
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
    }

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
            onDismissBtw={dismissBtwOverlay}
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
          onClose={closeShareModal}
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
