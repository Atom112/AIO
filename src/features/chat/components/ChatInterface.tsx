import {
  Component,
  For,
  Show,
  Setter,
  createSignal,
  createEffect,
  createMemo,
  onCleanup,
  onMount,
  on,
} from 'solid-js';
import { Portal } from 'solid-js/web';
import ModelSelector from './ModelSelector';
import {
  Topic,
  PendingAttachment,
  selectedModel,
  isStartingLocalModel,
  localEngineConnected,
  localEngineName,
  currentProject,
  datas,
  currentAssistantId,
  compactingTopicId,
  isChatMode,
  gitBranch,
  gitBranches,
  switchBranch,
  type GeneratedImageInfo,
} from '../../../core/store/store';
import { open } from '@tauri-apps/plugin-dialog';
import { pickSavePath } from '../../../core/api/dialog';
import { copyStoredImage } from '../../../core/api/attachment';
import { registerCommand, unregisterCommand } from '../../../core/shortcuts';
import SlashCommandMenu from '../../../shared/components/SlashCommandMenu';
import Icon from '../../../shared/components/Icon';
import { formatNumber, t } from '../../../core/i18n';
import ReasoningButton from './ReasoningButton';
import WebSearchButton from './WebSearchButton';
import ToolApprovalBubble, { type PendingApproval } from './ToolApprovalBubble';
import AgentQuestionBubble, { type PendingQuestion } from './AgentQuestionBubble';
import TaskBoardPanel from './TaskBoardPanel';
import TokenStatsBar from './TokenStatsBar';

import AgentModeSelector from './AgentModeSelector';
import WelcomeScreen from './WelcomeScreen';
import { useChatAutoScroll } from '../hooks/useChatAutoScroll';
import MessageBubble from './MessageBubble';
import SubagentMenu from './SubagentMenu';
import BackgroundTaskMenu from './BackgroundTaskMenu';
import PendingFileChips from './PendingFileChips';
import DiagnosticsToggle from './DiagnosticsToggle';
import BtwResponseBox from './BtwResponseBox';
import {
  btnGhost,
  btnIcon,
  btnPrimaryOutlineSm,
  btnSecondarySm,
} from '../../../shared/components/buttonStyles';

// Message IDs that already played their entrance animation; avoids replaying on agentSteps updates.
// Module-level (not component state): this component unmounts when entering a subagent view and
// remounts on return; a component-scoped set would replay every entrance animation (same pattern
// as AgentProcessBlock). Cleared on real topic switch by createEffect (see lastAnimatedTopicId).
const animatedMessageIds = new Set<string>();
// Topic ID whose animation memory was last cleared (module-level; pairs with the set above to distinguish remount from topic switch)
let lastAnimatedTopicId: string | null | undefined;

interface ChatInterfaceProps {
  activeTopic: Topic | null;
  isThinking: boolean;
  isProcessing: boolean;
  isDragging: boolean;
  typingIndex: number | null;
  inputMessage: string;
  setInputMessage: Setter<string>;
  pendingFiles: PendingAttachment[];
  setPendingFiles: Setter<PendingAttachment[]>;
  handleSendMessage: () => void;
  handleStopGeneration: () => void;
  handleFileUpload: (path: string, type: 'file' | 'image') => Promise<void>;
  pendingApprovals: PendingApproval[];
  onResolveApproval: (approvalId: string) => void;
  /** Pending ask_user question requests (the agent loop pauses until answered/skipped) */
  pendingQuestions: PendingQuestion[];
  /** Callback after a question bubble resolves (answered / skipped / expired) */
  onResolveQuestion: (requestId: string) => void;
  /** Whether to show the share button (plain chat mode only) */
  canShare: boolean;
  /** Callback to open the share modal (enters message selection mode) */
  onOpenShare: () => void;
  /** Message selection mode */
  isSelectingMessages: boolean;
  /** Currently selected message IDs */
  selectedMessageIds: Set<string>;
  /** Toggle selection of a single message */
  onToggleMessage: (msgId: string) => void;
  /** Select all */
  onSelectAll: () => void;
  /** Cancel selection */
  onCancelSelection: () => void;
  /** Confirm selection */
  onConfirmSelection: () => void;
  /** Branch from this message */
  onBranchFromMessage: (messageId: string) => void;
  /** /btw overlay Q&A boxes (transient, not persisted to history) */
  btwOverlays: Array<{ id: string; question: string; answer: string | null; loading: boolean }>;
  /** Dismiss a specific /btw overlay */
  onDismissBtw: (id: string) => void;
  /** A resumable crash-orphaned checkpoint exists for the active topic */
  checkpointAvailable: boolean;
  /** Resume the interrupted turn from its checkpoint */
  onResumeCheckpoint: () => void;
}

const ChatInterface: Component<ChatInterfaceProps> = (props) => {
  let textareaRef: HTMLTextAreaElement | undefined;
  let scrollContainerRef: HTMLDivElement | undefined;
  // Message content wrapper (all children of the scroll container): observed by the streaming
  // follow-bottom ResizeObserver. The scroll container is a fixed-height flex-grow box, so
  // content growth changes scrollHeight but not its box size (RO on it only fires once, at
  // observe()); the wrapper grows with content, so each streaming expansion triggers RO.
  let streamContentRef: HTMLDivElement | undefined;
  // 阶段4 第2项（拆分切片）：滚动跟随引擎抽至 useChatAutoScroll（报告点名路径）
  const { autoScroll, handleWheel, handleScroll, scrollBackToBottom } = useChatAutoScroll({
    scrollContainerRef: () => scrollContainerRef,
    streamContentRef: () => streamContentRef,
    topicId: () => props.activeTopic?.id,
    isThinking: () => props.isThinking,
    historyLength: () => props.activeTopic?.history?.length ?? 0,
  });
  // Filesystem tool readiness indicator.
  // Agent file tools run in-process (injected via file_tools::get_file_tool_specs in
  // commands/llm/mod.rs, under __builtin__); there is no __aio-filesystem__ stdio
  // subprocess handshake to track. Ready therefore means a project is loaded, so the
  // dot does not sit permanently on the "connecting" yellow state.
  const fsStatus = createMemo<'connected' | 'connecting' | 'error'>(() =>
    currentProject() ? 'connected' : 'connecting',
  );
  const fsColor = createMemo(() =>
    fsStatus() === 'connected' ? 'var(--color-success)' : 'var(--color-warn)',
  );
  const fsTooltip = createMemo(() =>
    fsStatus() === 'connected' ? t('chat.filesystemConnected') : t('chat.filesystemConnecting'),
  );
  /**
   * CHAT-11: wait until the running generation actually finishes (isThinking flips to
   * false in the stream's done handler), bounded to `timeoutMs`. The resend/edit paths
   * previously slept a magic 150ms after stop and hoped the backend's cancel epilogue
   * had finished; a slow epilogue (checkpoint finalize etc.) let the old stream's tail
   * interleave into the freshly resent message.
   */
  const awaitGenerationStopped = async (timeoutMs = 5000) => {
    const started = Date.now();
    while (props.isThinking && Date.now() - started < timeoutMs) {
      await new Promise((r) => setTimeout(r, 50));
    }
  };
  // Skip message animations on resend/edit (no exit + no entrance animation)
  const [skipMessageAnimation, setSkipMessageAnimation] = createSignal(false);
  // Git branch dropdown state
  const [branchOpen, setBranchOpen] = createSignal(false);
  const [branchToast, setBranchToast] = createSignal<string | null>(null);
  // Subagent pull-up menu state
  // Message DOM element map (id → HTMLElement), used for exit animations
  const messageEls = new Map<string, HTMLElement>();
  // Keep only messages still rendered in the current topic; prevents DOM references from accumulating across topic switches or deletions.
  createEffect(() => {
    const history = props.activeTopic?.history;
    if (!Array.isArray(history)) return;
    const live = new Set<string>();
    for (const m of history) {
      if (m && m.id) live.add(m.id as string);
    }
    for (const id of Array.from(messageEls.keys())) {
      if (!live.has(id)) messageEls.delete(id);
    }
  });
  // Reverted message IDs (local signal, not persisted)
  const [revertedMessages, setRevertedMessages] = createSignal<Set<string>>(new Set());

  // Round-11 feedback: the textarea height was only resized in onInput + reset only in
  // the Enter keydown — sending via the button (or any path that clears the signal)
  // left the box expanded until the next keypress. Drive the height REACTIVELY from the
  // signal instead: every path (typing, button send, /command fill, edit-resend) resizes.
  createEffect(() => {
    const value = props.inputMessage;
    const el = textareaRef;
    if (!el) return;
    if (!value) {
      el.style.height = '40px';
      el.style.overflowY = 'hidden';
      return;
    }
    el.style.height = 'auto';
    const natural = Math.min(el.scrollHeight, 200);
    el.style.height = `${natural}px`;
    el.style.overflowY = natural >= 200 ? 'auto' : 'hidden';
  });

  // All subagent steps of the current topic (including past turns, persisted on messages
  // via agentSteps). The status-bar pull-up menu entry shows when any subagent exists.

  // Close the Git branch dropdown on outside click (the subagent menu owns its closer)
  const closeOverlays = (e: MouseEvent) => {
    const target = e.target as HTMLElement;
    if (branchOpen() && !target.closest('.branch-dropdown-container')) {
      setBranchOpen(false);
    }
  };
  onMount(() => document.addEventListener('mousedown', closeOverlays));
  onCleanup(() => document.removeEventListener('mousedown', closeOverlays));

  // ---- Register shortcut commands ----
  const cmdFocusInput = registerCommand({
    id: 'focus-input',
    // M2 (i18n): display copy goes through COMMAND_TRANSLATIONS (command.focusInput.*),
    // same P3 convention as the sibling shortcuts in core/shortcuts.ts — the baked
    // value is a placeholder only, never user-visible copy.
    label: 'focus-input',
    description: '',
    category: 'chat',
    defaultKeys: 'Ctrl+I',
    handler: () => {
      if (textareaRef) {
        textareaRef.focus();
        textareaRef.scrollIntoView({ behavior: 'smooth', block: 'center' });
      }
    },
  });

  const cmdUploadFile = registerCommand({
    id: 'upload-file',
    label: 'upload-file',
    description: '',
    category: 'chat',
    defaultKeys: 'Ctrl+U',
    handler: async () => {
      try {
        const selected = await open({
          multiple: false,
          filters: [{ name: t('common.allFiles'), extensions: ['*'] }],
        });
        if (selected) {
          const path =
            typeof selected === 'string'
              ? selected
              : 'path' in selected
                ? (selected as any).path
                : String(selected);
          await props.handleFileUpload(path, 'file');
        }
      } catch (e) {
        console.warn('[shortcut] file upload cancelled or failed:', e);
      }
    },
  });

  // Clean up command registrations on component teardown
  onCleanup(() => {
    unregisterCommand(cmdFocusInput);
    unregisterCommand(cmdUploadFile);
  });

  /** Download a model-generated image to a user-chosen directory */
  const downloadGeneratedImage = async (img: GeneratedImageInfo) => {
    try {
      const dest = await pickSavePath(img.name);
      if (!dest) return;
      await copyStoredImage(img.storagePath, dest);
    } catch (e) {
      console.error('Failed to save image:', e);
      alert(t('error.save'));
    }
  };

  /** On topic switch: reset auto-scroll and the history-length baseline, then snap to the latest conversation */
  // 阶段4 第2项（拆分切片）：滚动跟随逻辑已抽至 useChatAutoScroll；此处只保留
  // 动画记忆的清理（入场动画去重，非滚动职责）。
  createEffect(
    on(
      () => props.activeTopic?.id,
      (id) => {
        if (id !== lastAnimatedTopicId) {
          animatedMessageIds.clear();
          lastAnimatedTopicId = id;
        }
      },
    ),
  );

  /** Index of the last user message, used to show buttons such as "resend" */
  const lastUserMsgIndex = createMemo(() => {
    const history = props.activeTopic?.history;
    if (!history || !Array.isArray(history)) return -1;
    let lastIdx = -1;
    for (let i = 0; i < history.length; i++) {
      if (history[i]?.role === 'user') lastIdx = i;
    }
    return lastIdx;
  });

  /** Last assistant reply (immediately after the last user message); deleted together on resend/edit */
  const lastAssistantReply = createMemo(() => {
    const history = props.activeTopic?.history;
    if (!history || !Array.isArray(history)) return null;
    const lastUserIdx = lastUserMsgIndex();
    if (lastUserIdx < 0 || lastUserIdx >= history.length - 1) return null;
    const next = history[lastUserIdx + 1];
    return next?.role === 'assistant' ? next : null;
  });

  // ---- Paged message rendering (segmented): long topics render the most recent
  // window first; an IntersectionObserver sentinel at the top loads earlier pages
  // with scroll anchoring, so switching to a huge topic does not mount every
  // Markdown bubble at once (the cause of the reported switch jank). ----
  const PAGE_SIZE = 40;
  const [visibleCount, setVisibleCount] = createSignal(PAGE_SIZE);
  // Reset the window on every topic switch
  createEffect(
    on(
      () => props.activeTopic?.id,
      () => setVisibleCount(PAGE_SIZE),
    ),
  );
  /** Absolute index of the first rendered message (the tail window start) */
  const windowStart = createMemo(() =>
    Math.max(0, (props.activeTopic?.history?.length ?? 0) - visibleCount()),
  );

  // ---- M-12: append-direction window pinning. While the user is scrolled away
  // from the bottom (autoScroll false — reading history mid-agent-turn), an
  // append (round-start placeholder / chat-mode message) would slide the tail
  // window's start forward and swap the content under their viewport. Grow the
  // window by the append delta so its top stays anchored to the same message;
  // returning to the bottom (or the scroll-to-bottom button) snaps the window
  // back to the tail. Deliberately separate from loadEarlier's growth, whose
  // user-driven expansion is never trimmed here. ----
  {
    let lastTopicId: string | undefined;
    let lastHistLen = 0;
    let pinnedGrowth = 0;
    createEffect(() => {
      const tid = props.activeTopic?.id;
      const histLen = props.activeTopic?.history?.length ?? 0;
      const following = autoScroll();
      if (tid !== lastTopicId) {
        lastTopicId = tid;
        lastHistLen = histLen;
        pinnedGrowth = 0;
        return;
      }
      const delta = histLen - lastHistLen;
      lastHistLen = histLen;
      if (delta > 0 && !following && windowStart() > 0) {
        // Reading history while new messages land: pin the window's top anchor.
        pinnedGrowth += delta;
        setVisibleCount((c) => c + delta);
      } else if (pinnedGrowth > 0 && (following || delta > 0)) {
        // Back at the bottom (or a later append while following): re-trim to the
        // tail window — the user sees the last PAGE_SIZE messages either way. The
        // unmount happens ABOVE the viewport; the explicit snap compensates the
        // lost scrollHeight so the view stays glued to the bottom.
        pinnedGrowth = 0;
        setVisibleCount(PAGE_SIZE);
        if (following) {
          requestAnimationFrame(() => {
            const el = scrollContainerRef;
            if (el) el.scrollTop = el.scrollHeight - el.clientHeight;
          });
        }
      }
    });
  }

  let topSentinelRef: HTMLDivElement | undefined;
  let loadingEarlier = false;
  const loadEarlier = () => {
    if (loadingEarlier || windowStart() === 0) return;
    loadingEarlier = true;
    const el = scrollContainerRef;
    const prevHeight = el?.scrollHeight ?? 0;
    const prevTop = el?.scrollTop ?? 0;
    // Pre-mark the about-to-reveal messages as animated: history bubbles must not
    // replay entrance animations when paged in (module-level set, topic-scoped clear)
    const start = Math.max(0, windowStart() - PAGE_SIZE);
    for (const m of props.activeTopic?.history?.slice(start, windowStart()) ?? []) {
      if (m?.id) animatedMessageIds.add(m.id);
    }
    setVisibleCount((c) => c + PAGE_SIZE);
    // Scroll anchoring: keep the viewport on the same content after prepending
    requestAnimationFrame(() => {
      if (el) el.scrollTop = prevTop + (el.scrollHeight - prevHeight);
      loadingEarlier = false;
    });
  };
  onMount(() => {
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((en) => en.isIntersecting)) loadEarlier();
      },
      { root: scrollContainerRef, rootMargin: '160px' },
    );
    if (topSentinelRef) io.observe(topSentinelRef);
    onCleanup(() => io.disconnect());
  });

  return (
    <div class="flex flex-col flex-grow items-stretch rounded-[12px] box-border overflow-hidden p-[15px] pb-5 relative h-full">
      {/* Top action bar */}
      <Show
        when={props.isSelectingMessages}
        fallback={
          <div class="flex items-center justify-between px-1 pb-3 shrink-0">
            <span
              class="text-sm font-medium truncate"
              style={{ color: 'rgba(var(--text-base-rgb),0.7)' }}
            >
              {(currentProject()?.name || props.activeTopic?.name) ?? ''}
            </span>
            <Show when={props.canShare && props.activeTopic}>
              <button
                class="group inline-flex items-center justify-center bg-transparent rounded-lg cursor-pointer w-8 h-8 hover:w-[68px] transition-all duration-200 hover:px-2 hover:bg-[rgba(var(--text-base-rgb),0.06)]"
                style={{
                  border: '1px solid var(--border-dim)',
                  color: 'rgba(var(--text-base-rgb),0.45)',
                }}
                onClick={() => props.onOpenShare()}
              >
                <Icon src="/icons/app-logo/share.svg" class="w-[15px] h-[15px] shrink-0" />
                <span class="overflow-hidden whitespace-nowrap text-[11px] max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[80px] group-hover:opacity-100 group-hover:ml-1">
                  {t('common.share')}
                </span>
              </button>
            </Show>
          </div>
        }
      >
        {/* Selection-mode toolbar */}
        <div class="flex items-center justify-between px-1 pb-3 shrink-0 animate-fade-in">
          <div class="flex items-center gap-2">
            <button class={btnSecondarySm} onClick={() => props.onSelectAll()}>
              {t('common.selectAll')}
            </button>
            <button class={btnSecondarySm} onClick={() => props.onCancelSelection()}>
              {t('common.cancel')}
            </button>
          </div>
          <div class="flex items-center gap-3">
            <span class="text-xs text-white/50">
              {t('chat.selectedMessages', { count: formatNumber(props.selectedMessageIds.size) })}
            </span>
            <button
              class={btnPrimaryOutlineSm}
              disabled={props.selectedMessageIds.size === 0}
              onClick={() => props.onConfirmSelection()}
            >
              {t('common.confirm')}
            </button>
          </div>
        </div>
      </Show>

      <div
        ref={scrollContainerRef}
        onScroll={handleScroll}
        onWheel={handleWheel}
        class="flex-grow overflow-y-auto pb-[15px] z-[1]"
      >
        <div ref={streamContentRef} class="flex flex-col min-h-full">
          <Show when={!props.activeTopic || (props.activeTopic?.history?.length ?? 0) === 0}>
            <div class="flex-1 flex items-center justify-center">
              <WelcomeScreen
                onSuggestionClick={(text) => {
                  props.setInputMessage(text);
                  if (textareaRef) {
                    textareaRef.focus();
                    textareaRef.scrollIntoView({ behavior: 'smooth', block: 'center' });
                  }
                }}
              />
            </div>
          </Show>

          <Show when={props.activeTopic}>
            {/* Load-earlier sentinel: always mounted so the IntersectionObserver keeps a
                stable target; the button is only shown when older pages exist */}
            <div ref={topSentinelRef} />
            <Show when={windowStart() > 0}>
              <div class="flex justify-center py-2">
                <button
                  class={btnGhost + ' text-xs'}
                  onClick={loadEarlier}
                  title={t('chat.loadEarlierTitle')}
                >
                  {t('chat.loadEarlier', { count: formatNumber(windowStart()) })}
                </button>
              </div>
            </Show>
            <For each={props.activeTopic?.history.slice(windowStart())}>
              {(msg: any, index) => (
                <MessageBubble
                  msg={msg}
                  index={() => windowStart() + index()}
                  parentTypingIndex={() => props.typingIndex}
                  isThinking={() => props.isThinking}
                  isSelectingMessages={() => props.isSelectingMessages}
                  selectedMessageIds={() => props.selectedMessageIds}
                  onToggleMessage={props.onToggleMessage}
                  skipMessageAnimation={skipMessageAnimation}
                  markAnimated={(id) => animatedMessageIds.add(id)}
                  hasAnimated={(id) => animatedMessageIds.has(id)}
                  registerEl={(id, el) => messageEls.set(id, el)}
                  getEl={(id) => messageEls.get(id)}
                  lastUserMsgIndex={lastUserMsgIndex}
                  lastAssistantReply={lastAssistantReply}
                  handleStopGeneration={props.handleStopGeneration}
                  awaitGenerationStopped={awaitGenerationStopped}
                  onBranchFromMessage={props.onBranchFromMessage}
                  setSkipMessageAnimation={setSkipMessageAnimation}
                  resendWithInput={(text) => {
                    props.setInputMessage(text);
                    setTimeout(() => props.handleSendMessage(), 0);
                  }}
                  editWithInput={(text) => {
                    props.setInputMessage(text);
                    if (textareaRef) {
                      textareaRef.focus();
                      textareaRef.scrollIntoView({ behavior: 'smooth', block: 'center' });
                    }
                  }}
                  downloadGeneratedImage={downloadGeneratedImage}
                  project={currentProject}
                  isReverted={(msgId, idx) => revertedMessages().has(msgId ?? String(idx))}
                  markReverted={(key) =>
                    setRevertedMessages((prev) => {
                      const next = new Set(prev);
                      next.add(key);
                      return next;
                    })
                  }
                  showUndo={() => !!currentProject()?.path && !!gitBranch()}
                />
              )}
            </For>

            {/* Context compaction in progress: in-record pill with a spinner
                (mainstream visual feedback, cf. DSH's in-record compaction indicator) */}
            <Show when={props.activeTopic && compactingTopicId() === props.activeTopic.id}>
              <div class="flex items-center justify-center gap-2 my-3 animate-fade-in">
                <svg
                  class="w-4 h-4 animate-spin"
                  viewBox="0 0 24 24"
                  fill="none"
                  xmlns="http://www.w3.org/2000/svg"
                >
                  <circle
                    cx="12"
                    cy="12"
                    r="9"
                    stroke="rgba(var(--primary-rgb),0.18)"
                    stroke-width="2.5"
                  />
                  <path
                    d="M21 12a9 9 0 0 0-9-9"
                    stroke="rgba(var(--primary-rgb),0.85)"
                    stroke-width="2.5"
                    stroke-linecap="round"
                  />
                </svg>
                <span class="text-xs" style={{ color: 'rgba(var(--text-base-rgb),0.55)' }}>
                  {t('chat.compacting')}
                </span>
              </div>
            </Show>

            {/* /btw overlay Q&A boxes */}
            <Show when={props.btwOverlays.length > 0}>
              <div class="flex flex-col items-center mb-3 space-y-2">
                <For each={props.btwOverlays}>
                  {(item) => <BtwResponseBox item={item} onDismiss={props.onDismissBtw} />}
                </For>
              </div>
            </Show>
          </Show>
        </div>
      </div>

      {/* Scroll-to-bottom button: shown after the user scrolls up into history; click returns to the latest messages */}
      <Show when={!autoScroll()}>
        <button
          class="absolute bottom-[200px] right-[30px] z-[50] flex items-center justify-center w-9 h-9 rounded-full cursor-pointer
                           animate-fade-in transition-all duration-200 hover:scale-110 active:scale-95"
          style={{
            background: 'rgba(var(--primary-rgb),0.15)',
            border: '1px solid rgba(var(--primary-rgb),0.25)',
            color: 'rgba(var(--primary-rgb),0.7)',
            'box-shadow': '0 2px 8px rgba(0,0,0,0.3)',
          }}
          onClick={scrollBackToBottom}
          title={t('chat.scrollLatest')}
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            fill="none"
            viewBox="0 0 24 24"
            stroke-width="2"
            stroke="currentColor"
            class="w-4 h-4"
          >
            <path stroke-linecap="round" stroke-linejoin="round" d="M19.5 8.25l-7.5 7.5-7.5-7.5" />
          </svg>
        </button>
      </Show>

      <Show when={props.isProcessing}>
        <div
          class="absolute inset-0 flex items-center justify-center z-[100]"
          style={{
            background: 'rgba(0,0,0,0.6)',
            color: 'rgba(var(--primary-rgb),0.6)',
            'font-size': '0.875rem',
            'backdrop-filter': 'blur(4px)',
          }}
        >
          {t('chat.parsingFile')}
        </div>
      </Show>

      {/* Pending attachments (extracted: PendingFileChips) */}
      <PendingFileChips
        pendingFiles={() => props.pendingFiles}
        setPendingFiles={props.setPendingFiles}
      />

      <div class="bg-transparent flex flex-col relative w-full z-10">
        {/* Agent mode status bar */}
        <Show when={!isChatMode()}>
          {(() => {
            const asst = datas.assistants.find((a: any) => a.id === currentAssistantId());
            const mode = asst?.agentMode || 'off';
            const project = currentProject();
            const modeLabel: string =
              {
                normal: t('agent.mode.normal'),
                auto: t('agent.mode.auto'),
                danger: t('agent.mode.danger'),
                plan: t('agent.mode.plan'),
              }[mode as 'normal' | 'auto' | 'danger' | 'plan'] || mode;
            return (
              <div
                class="flex flex-col rounded-2xl text-xs"
                style={{
                  background: 'rgba(var(--surface-bg), 0.5)',
                  border: '1px solid rgba(var(--border-rgb), 0.15)',
                }}
              >
                {/* Task list (create_tasks / update_task live mirror): expands inside the
                    status bar frame above the status row; collapsible to a one-line summary */}
                <TaskBoardPanel topicId={props.activeTopic?.id ?? ''} />

                {/* Tool approvals + ask_user: interaction panels now live in the status bar
                    (DSH-style) instead of popping cards into the message flow */}
                <Show when={props.pendingApprovals.length > 0 || props.pendingQuestions.length > 0}>
                  <div
                    class="px-2 py-2 space-y-2 max-h-[45vh] overflow-y-auto border-b"
                    style={{ 'border-color': 'rgba(var(--border-rgb), 0.15)' }}
                  >
                    <For each={props.pendingApprovals}>
                      {(approval) => (
                        <ToolApprovalBubble
                          approval={approval}
                          onResolved={props.onResolveApproval}
                        />
                      )}
                    </For>
                    {/* ask_user: one question at a time — show only the first pending
                        request; resolving it reveals the next (a request carrying
                        several questions steps through them inside the bubble) */}
                    <Show when={props.pendingQuestions.length > 0}>
                      <AgentQuestionBubble
                        question={props.pendingQuestions[0]}
                        onResolved={props.onResolveQuestion}
                      />
                    </Show>
                  </div>
                </Show>

                <div class="flex items-center gap-2 px-3 h-8">
                  <Icon name="wrench" size={14} />
                  <span style={{ color: 'rgba(var(--text-base-rgb),0.7)' }}>
                    {t('chat.agentStatus', { mode: modeLabel, project: project?.name ?? '' })}
                  </span>
                  <Show when={gitBranch()}>
                    <div class="branch-dropdown-container relative inline-block">
                      <button
                        type="button"
                        class={'px-1.5 py-0.5 bg-[rgba(var(--surface-alt-bg),0.5)] ' + btnGhost}
                        style={{ color: 'rgba(var(--text-base-rgb),0.5)', 'font-weight': '400' }}
                        classList={{
                          // 全响应式：open 恒深背景（important 压过常态类）；
                          // 非 open 才允许 hover 浅深背景。不做任何 JS 样式写入，
                          // 杜绝 mouseleave 守卫漏恢复导致的深色残留。
                          '!bg-[rgba(var(--surface-bg),0.7)]': branchOpen(),
                          'hover:bg-[rgba(var(--surface-bg),0.6)]': !branchOpen(),
                        }}
                        onClick={(e) => {
                          e.stopPropagation();
                          setBranchOpen(!branchOpen());
                        }}
                      >
                        {t('chat.branchPrefix')}{' '}
                        <Icon
                          name="git-branch"
                          size={12}
                          style={{ display: 'inline', 'vertical-align': 'middle' }}
                        />{' '}
                        {gitBranch()}
                        <Icon
                          name="chevron-up"
                          size={10}
                          class="ml-0.5"
                          style={{ display: 'inline', 'vertical-align': 'middle' }}
                        />
                      </button>
                      <Show when={branchOpen()}>
                        <div
                          ref={(el) =>
                            requestAnimationFrame(() => {
                              el.classList.remove('opacity-0', 'scale-95');
                              el.classList.add('opacity-100', 'scale-100');
                            })
                          }
                          class="absolute bottom-full left-0 mb-2 z-[41] w-[200px] rounded-xl overflow-hidden opacity-0 scale-95 transition-all duration-200 ease-out origin-bottom"
                          style={{
                            background: 'var(--popup-bg)',
                            border: '1px solid var(--border-dim)',
                            'backdrop-filter': 'var(--popup-blur)',
                            'box-shadow': '0 -8px 30px rgba(0,0,0,0.4)',
                          }}
                          onClick={(e) => e.stopPropagation()}
                        >
                          <div
                            class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest"
                            style={{
                              color: 'rgba(var(--text-base-rgb),0.35)',
                              background: 'rgba(var(--text-base-rgb),0.04)',
                              'border-bottom': '1px solid var(--border-dim)',
                            }}
                          >
                            {t('chat.switchBranch')}
                          </div>
                          <div class="py-1 max-h-[200px] overflow-y-auto">
                            <For each={gitBranches()}>
                              {(branch) => (
                                <button
                                  type="button"
                                  class="w-full flex items-center gap-2 px-3 py-2 text-left transition-colors cursor-pointer border-none text-[13px]"
                                  style={{ color: 'rgba(var(--text-base-rgb),0.75)' }}
                                  classList={{
                                    '!bg-[rgba(var(--surface-bg),0.5)]': branch === gitBranch(),
                                  }}
                                  onClick={async () => {
                                    if (branch === gitBranch()) {
                                      setBranchOpen(false);
                                      return;
                                    }
                                    try {
                                      await switchBranch(branch);
                                      setBranchOpen(false);
                                    } catch (e) {
                                      console.error('[git] switch branch failed:', e);
                                      setBranchToast(t('chat.branchSwitchFailed'));
                                      setTimeout(() => setBranchToast(null), 3000);
                                    }
                                  }}
                                  onMouseEnter={(e) => {
                                    if (branch !== gitBranch())
                                      e.currentTarget.style.background =
                                        'rgba(var(--text-base-rgb),0.05)';
                                  }}
                                  onMouseLeave={(e) => {
                                    if (branch !== gitBranch())
                                      e.currentTarget.style.background = 'transparent';
                                  }}
                                >
                                  <span class="flex-1 truncate">{branch}</span>
                                  <Show when={branch === gitBranch()}>
                                    <Icon
                                      name="check"
                                      size={13}
                                      class="shrink-0"
                                      style={{ color: 'rgba(var(--text-base-rgb),0.7)' }}
                                    />
                                  </Show>
                                </button>
                              )}
                            </For>
                          </div>
                        </div>
                      </Show>
                    </div>
                  </Show>
                  <Show when={project}>
                    <span
                      class="w-2 h-2 rounded-full shrink-0"
                      style={{ background: fsColor() }}
                      title={fsTooltip()}
                    />
                  </Show>
                  <Show when={project && selectedModel()?.engine_type && localEngineName()}>
                    <span
                      class="w-px h-3 shrink-0"
                      style={{ background: 'rgba(var(--text-base-rgb),0.15)' }}
                    />
                  </Show>
                  <Show when={selectedModel()?.engine_type && localEngineName()}>
                    <span
                      class="inline-flex items-center gap-1 text-[11px] shrink-0"
                      style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}
                    >
                      <Show
                        when={isStartingLocalModel()}
                        fallback={
                          <Show when={localEngineConnected()}>
                            <span style={{ color: 'var(--color-success)' }}>
                              {t('chat.localEngineConnected', { engine: localEngineName() })}
                            </span>
                          </Show>
                        }
                      >
                        <Icon
                          src="/icons/app-logo/loading.svg"
                          class="w-3 h-3 animate-spin shrink-0"
                        />
                        <span class="text-white/80">
                          {t('chat.localEngineConnecting', { engine: localEngineName() })}
                        </span>
                      </Show>
                    </span>
                  </Show>

                  {/* Right-aligned pull-up menu group: app background jobs (memory /
                      downloads / indexing) sit LEFT of the subagent menu; the group
                      wrapper owns the ml-auto push so either menu can hide without
                      moving the other */}
                  <div class="ml-auto flex items-center gap-2">
                    <BackgroundTaskMenu activeTopic={() => props.activeTopic} />
                    <SubagentMenu activeTopic={() => props.activeTopic} />
                  </div>
                </div>
              </div>
            );
          })()}
        </Show>
        <div
          class="rounded-xl box-border flex flex-col gap-[10px] mt-[3px] p-[10px] transition-all duration-200 w-full"
          style={{ background: 'rgba(0,0,0,0.25)', border: '1px solid var(--border-dim)' }}
        >
          <textarea
            ref={textareaRef}
            class="bg-transparent border-none text-white font-inherit text-base leading-relaxed min-h-[40px] max-h-[20vh] outline-none overflow-y-hidden px-[5px] pb-[5px] resize-none w-full focus:overflow-y-auto"
            placeholder={t('chat.input.placeholder')}
            value={props.inputMessage}
            onInput={(e) => props.setInputMessage(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.ctrlKey && !e.shiftKey) {
                e.preventDefault();
                props.handleSendMessage();
              }
            }}
          />
          <SlashCommandMenu
            textareaRef={textareaRef}
            inputMessage={props.inputMessage}
            setInputMessage={props.setInputMessage}
          />

          <div
            class="flex items-center justify-between border-t pt-2"
            style={{ 'border-color': 'var(--border-dim)' }}
          >
            <div class="flex items-center gap-2">
              <ModelSelector />
              <Show when={!isChatMode()}>
                <AgentModeSelector />
              </Show>
              <ReasoningButton />
              <WebSearchButton />

              <button
                class={'p-1.5 ' + btnIcon}
                style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                title={t('chat.attachFile')}
                onClick={async () => {
                  const selected = await open({ multiple: true });
                  if (!selected) return;
                  const paths = Array.isArray(selected) ? selected : [selected];
                  for (const path of paths) {
                    await props.handleFileUpload(path, 'file');
                  }
                }}
                onMouseEnter={(e) => {
                  e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.06)';
                  e.currentTarget.style.color = 'rgba(var(--primary-rgb),0.6)';
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.background = 'transparent';
                  e.currentTarget.style.color = 'rgba(var(--text-base-rgb),0.4)';
                }}
              >
                <Icon src="/icons/app-logo/paperclip.svg" class="w-5 h-5" />
              </button>

              <button
                class={'p-1.5 ' + btnIcon}
                style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                title={t('chat.uploadImage')}
                onClick={async () => {
                  const selected = await open({
                    multiple: true,
                    filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp'] }],
                  });
                  if (!selected) return;
                  const paths = Array.isArray(selected) ? selected : [selected];
                  for (const path of paths) {
                    await props.handleFileUpload(path, 'image');
                  }
                }}
                onMouseEnter={(e) => {
                  e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.06)';
                  e.currentTarget.style.color = 'rgba(var(--primary-rgb),0.6)';
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.background = 'transparent';
                  e.currentTarget.style.color = 'rgba(var(--text-base-rgb),0.4)';
                }}
              >
                <Icon src="/icons/app-logo/image-photo.svg" class="w-5 h-5" />
              </button>

              {/* LSP problems panel toggle (extracted: DiagnosticsToggle) */}
              <DiagnosticsToggle />
            </div>

            <div class="flex items-center gap-2">
              <TokenStatsBar />
              {/* Checkpoint resume: appears left of the send button when a crash-orphaned
                  turn is detected; the label expands on hover (same animation as the
                  copy/branch action buttons in the message history). */}
              <Show when={props.checkpointAvailable && !props.isThinking}>
                <button
                  type="button"
                  class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(var(--text-base-rgb),0.08)] hover:!p-[4px_8px]"
                  style={{
                    border: '1px solid rgba(var(--primary-rgb),0.45)',
                    color: 'rgb(var(--primary-rgb))',
                  }}
                  title={t('agent.checkpoint.resume')}
                  onClick={() => props.onResumeCheckpoint()}
                >
                  <Icon name="rotate-ccw" size={13} />
                  <span class="overflow-hidden whitespace-nowrap text-[11px] max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[110px] group-hover:opacity-100 group-hover:ml-1">
                    {t('agent.checkpoint.resumeShort')}
                  </span>
                </button>
              </Show>
              <button
                class={`flex items-center justify-center border-none rounded-lg cursor-pointer h-8 w-8 transition-all duration-100 hover:opacity-90 hover:scale-105 active:scale-95 ${props.isThinking ? 'bg-[var(--color-danger)] text-white' : 'text-white'}`}
                style={!props.isThinking ? { background: 'rgba(var(--primary-rgb),0.3)' } : {}}
                onClick={() =>
                  props.isThinking ? props.handleStopGeneration() : props.handleSendMessage()
                }
              >
                <Show
                  when={props.isThinking}
                  fallback={<Icon src="/icons/app-logo/send.svg" class="w-[18px] h-[18px]" />}
                >
                  <Icon src="/icons/app-logo/stop-square.svg" class="w-[18px] h-[18px]" />
                </Show>
              </button>
            </div>
          </div>
        </div>
      </div>

      <Show when={props.isDragging}>
        <div
          class="absolute inset-0 flex items-center justify-center pointer-events-none z-[9999]"
          style={{ background: 'rgba(0,0,0,0.5)', 'backdrop-filter': 'blur(8px)' }}
        >
          <div
            class="relative flex flex-col items-center justify-center w-[420px] h-[280px] rounded-xl text-white text-center p-5"
            style={{
              background: 'rgba(var(--surface-bg), 0.7)',
              border: '1px solid var(--border-dim)',
            }}
          >
            <div class="flex items-end mb-[25px] mt-[-30px]">
              <div
                class="flex items-center justify-center w-[60px] h-20 rounded-md opacity-60 scale-[0.85] translate-y-[10px] -rotate-12 translate-x-[15px] z-[1]"
                style={{
                  background: 'rgba(var(--primary-rgb),0.06)',
                  border: '1px solid rgba(var(--primary-rgb),0.06)',
                }}
              >
                <span style={{ color: 'rgba(var(--primary-rgb),0.4)' }}>
                  <Icon src="/icons/app-logo/file-document.svg" class="w-6 h-6" />
                </span>
              </div>
              <div
                class="flex items-center justify-center w-[70px] h-[90px] rounded-md z-[3]"
                style={{
                  background: 'rgba(var(--primary-rgb),0.08)',
                  border: '1px solid rgba(var(--primary-rgb),0.1)',
                  color: 'rgba(var(--primary-rgb),0.5)',
                }}
              >
                <Icon src="/icons/app-logo/upload-arrow.svg" class="w-8 h-8" />
              </div>
              <div
                class="flex items-center justify-center w-[60px] h-20 rounded-md opacity-60 scale-[0.85] translate-y-[10px] rotate-12 -translate-x-[15px] z-[1]"
                style={{
                  background: 'rgba(var(--primary-rgb),0.06)',
                  border: '1px solid rgba(var(--primary-rgb),0.12)',
                }}
              >
                <span style={{ color: 'rgba(var(--primary-rgb),0.4)' }}>
                  <Icon src="/icons/app-logo/file-blank.svg" class="w-6 h-6" />
                </span>
              </div>
            </div>
            <h2
              style={{
                color: 'rgba(var(--primary-rgb),0.6)',
                'font-size': '22px',
                'letter-spacing': '0.1em',
                'margin-bottom': '10px',
                'z-index': '2',
              }}
            >
              {t('chat.dragUploadTitle')}
            </h2>
            <p
              style={{
                color: 'rgba(var(--text-base-rgb),0.5)',
                'font-size': '0.875rem',
                'max-width': '80%',
                'z-index': '2',
              }}
            >
              {t('chat.dragUploadFormats')}
            </p>
            <div
              class="absolute inset-3 rounded-lg pointer-events-none"
              style={{ border: '1px dashed var(--border-dim)' }}
            />
          </div>
        </div>
      </Show>

      {/* Git branch switch failure toast: portaled to body to escape the parent containing block */}
      <Show when={branchToast()}>
        <Portal>
          <div
            class="fixed bottom-5 left-5 z-[9999] max-w-[25vw] rounded-xl px-[18px] py-[10px] text-white text-[13px] font-medium shadow-[0_8px_32px_rgba(0,0,0,0.45)] select-none cursor-pointer"
            style={{
              color: 'var(--color-danger)',
              background: 'var(--popup-bg)',
              border: '1px solid rgba(248,113,113,0.2)',
              'backdrop-filter': 'var(--popup-blur)',
              animation: 'toastIn 0.3s cubic-bezier(0.175, 0.885, 0.32, 1.275) forwards',
            }}
            onClick={() => setBranchToast(null)}
          >
            {branchToast()}
          </div>
        </Portal>
      </Show>
    </div>
  );
};

export default ChatInterface;
