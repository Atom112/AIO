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
import Markdown, { getLangIcon } from '../../../shared/components/Markdown';
import AgentProcessBlock from './AgentProcessBlock';
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
  setDatas,
  currentAssistantId,
  currentTopicId,
  isChatMode,
  gitBranch,
  gitBranches,
  switchBranch,
  collectSubagents,
  setActiveSubagentView,
  type FileChangeInfo,
  type GeneratedImageInfo,
} from '../../../core/store/store';
import {
  getSubagentProfileStyle,
  subagentProfileLabel,
  formatSubagentDuration,
} from './SubagentConversationView';
import { open } from '@tauri-apps/plugin-dialog';
import { pickSavePath } from '../../../core/api/dialog';
import { convertFileSrc } from '@tauri-apps/api/core';
import { deleteTopicMessage } from '../../../core/api/chat';
import { copyStoredImage, discardChatAttachment } from '../../../core/api/attachment';
import { revertFileChangesBatch } from '../../../core/api/project';
import { getLogo as getLogoByIds } from '../../../core/utils/modelLogo';
import { registerCommand, unregisterCommand } from '../../../core/shortcuts';
import SlashCommandMenu from '../../../shared/components/SlashCommandMenu';
import Icon from '../../../shared/components/Icon';
import { formatNumber, t } from '../../../core/i18n';
import ReasoningButton from './ReasoningButton';
import WebSearchButton from './WebSearchButton';
import ToolApprovalBubble, { type PendingApproval } from './ToolApprovalBubble';
import AgentQuestionBubble, { type PendingQuestion } from './AgentQuestionBubble';
import TokenStatsBar from './TokenStatsBar';
import {
  totalErrors,
  totalWarnings,
  problemsPanelVisible,
  setProblemsPanelVisible,
  hasDiagnostics,
} from '../../../core/store/diagnostics';
import AgentModeSelector from './AgentModeSelector';
import WelcomeScreen from './WelcomeScreen';
import DiffView from '../../../shared/components/DiffView';
import BtwResponseBox from './BtwResponseBox';
import { formatSize } from '../../../core/utils/format';
import {
  btnBadge,
  btnGhost,
  btnIcon,
  btnPrimarySm,
  btnSecondarySm,
} from '../../../shared/components/buttonStyles';

/** Aggregated file-change entry; same shape as FileChangeInfo but deduplicated */
type AggregatedFileChange = FileChangeInfo;

/** Aggregate all file changes from the message's toolCalls, deduplicated by filePath */
function aggregateFileChanges(msg: any): AggregatedFileChange[] {
  const toolCalls: any[] = msg.toolCalls || [];
  const seen = new Map<string, FileChangeInfo>();
  for (const tc of toolCalls) {
    const changes: FileChangeInfo[] = tc.fileChanges || [];
    for (const fc of changes) {
      seen.set(fc.filePath, fc);
    }
  }
  return Array.from(seen.values());
}

/** Compute added/deleted line stats from the aggregated file changes */
function aggregateDiffStats(changes: AggregatedFileChange[]): { added: number; deleted: number } {
  let added = 0;
  let deleted = 0;
  for (const c of changes) {
    const m = c.summary.match(/\+(\d+)\s*-(\d+)/);
    if (m) {
      added += parseInt(m[1], 10);
      deleted += parseInt(m[2], 10);
    }
  }
  return { added, deleted };
}

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
}

const ChatInterface: Component<ChatInterfaceProps> = (props) => {
  let textareaRef: HTMLTextAreaElement | undefined;
  let scrollContainerRef: HTMLDivElement | undefined;
  // Message content wrapper (all children of the scroll container): observed by the streaming
  // follow-bottom ResizeObserver. The scroll container is a fixed-height flex-grow box, so
  // content growth changes scrollHeight but not its box size (RO on it only fires once, at
  // observe()); the wrapper grows with content, so each streaming expansion triggers RO.
  let streamContentRef: HTMLDivElement | undefined;
  const [autoScroll, setAutoScroll] = createSignal(true);
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
  // Pending programmatic scrolls: +1 before each programmatic scrollTop assignment,
  // consumed by one scroll event; events run real logic (at-bottom check) only when the
  // count is 0, so multiple scroll events per assignment cannot spuriously disable auto-follow.
  let pendingSnaps = 0;
  // Expected value of the last programmatic scrollTop assignment; scroll events compare it
  // with the current scrollTop: match = programmatic echo (consume the count); mismatch =
  // the user dragged the bar mid-programmatic-scroll, treated as user scrolling even when
  // count > 0 (snaps fire per 50ms content batch; count-masking alone would swallow them).
  let lastProgrammaticTop = -1;
  // rAF id of the smooth-scroll animation
  let smoothScrollRAF: number | undefined;
  // Last observed history length, used to detect newly added messages
  let lastHistoryLen = 0;
  // Latest id of the streaming rAF loop; always points at the last scheduled frame so it can be cancelled
  // Manual user scroll-up flag: a plain mutable variable bypasses SolidJS reactivity latency so rAF callbacks see it synchronously
  let userScrolledUp = false;
  // Skip message animations on resend/edit (no exit + no entrance animation)
  const [skipMessageAnimation, setSkipMessageAnimation] = createSignal(false);
  // Git branch dropdown state
  const [branchOpen, setBranchOpen] = createSignal(false);
  const [branchToast, setBranchToast] = createSignal<string | null>(null);
  // Subagent pull-up menu state
  const [subagentMenuOpen, setSubagentMenuOpen] = createSignal(false);
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

  // All subagent steps of the current topic (including past turns, persisted on messages
  // via agentSteps). The status-bar pull-up menu entry shows when any subagent exists.
  const subagents = createMemo(() => collectSubagents(props.activeTopic));
  const hasRunningSubagent = () => subagents().some((s) => s.status === 'running');

  // Close the Git branch dropdown / subagent pull-up menu on outside click
  const closeOverlays = (e: MouseEvent) => {
    const target = e.target as HTMLElement;
    if (branchOpen() && !target.closest('.branch-dropdown-container')) {
      setBranchOpen(false);
    }
    if (subagentMenuOpen() && !target.closest('.subagent-menu-container')) {
      setSubagentMenuOpen(false);
    }
  };
  onMount(() => document.addEventListener('mousedown', closeOverlays));
  onCleanup(() => document.removeEventListener('mousedown', closeOverlays));

  // ---- Register shortcut commands ----
  const cmdFocusInput = registerCommand({
    id: 'focus-input',
    label: '聚焦输入框',
    description: '将光标聚焦到消息输入框',
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
    label: '上传文件',
    description: '打开文件选择对话框上传文件',
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

  const getModelLogo = (modelName: string) => {
    return getLogoByIds(null, modelName);
  };

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

  /** Whether the container is scrolled to the bottom (50px threshold) */
  const isAtBottom = () => {
    if (!scrollContainerRef) return true;
    const el = scrollContainerRef;
    return el.scrollHeight - el.scrollTop - el.clientHeight < 50;
  };

  /** Wheel scroll up: stop auto-scroll immediately and cancel any in-flight smooth scroll */
  const handleWheel = (e: WheelEvent) => {
    if (e.deltaY < 0) {
      // Plain mutable flag: the next rAF frame sees it immediately, stopping follow-bottom with zero latency
      userScrolledUp = true;
      if (smoothScrollRAF !== undefined) {
        cancelAnimationFrame(smoothScrollRAF);
        smoothScrollRAF = undefined;
      }
      pendingSnaps = 0;
      setAutoScroll(false);
    }
  };

  /** Scroll event: distinguish user scrolling from programmatic scrolling (position-echo comparison; see lastProgrammaticTop) */
  const handleScroll = () => {
    if (pendingSnaps > 0) {
      pendingSnaps -= 1;
      if (scrollContainerRef && Math.abs(scrollContainerRef.scrollTop - lastProgrammaticTop) < 2) {
        return;
      }
      // Position mismatch: user drag between programmatic scrolls; falls through to the user logic below (count already decremented)
    }
    const atBottom = isAtBottom();
    setAutoScroll(atBottom);
    // User manually scrolled back to the bottom: clear the scroll-up flag and resume auto-follow
    if (atBottom) {
      userScrolledUp = false;
    }
  };

  /** Snap to the bottom instantly (used during streaming; scrolls down only) */
  const snapToBottom = () => {
    if (!scrollContainerRef) return;
    const el = scrollContainerRef;
    const target = Math.max(0, el.scrollHeight - el.clientHeight);
    if (el.scrollTop < target) {
      pendingSnaps += 1;
      el.scrollTop = target;
      lastProgrammaticTop = target;
    }
  };

  /** Smooth-scroll to the bottom (rAF animation, for non-streaming cases) */
  const smoothScrollToBottom = () => {
    if (!scrollContainerRef) return;
    const el = scrollContainerRef;
    const target = Math.max(0, el.scrollHeight - el.clientHeight);
    const start = el.scrollTop;
    const distance = target - start;
    // Target not above the current position: never scroll upward (prevents a brief content collapse from snapping the view back to the top)
    if (target <= start) {
      el.scrollTop = target;
      lastProgrammaticTop = target;
      return;
    }
    if (Math.abs(distance) < 2) {
      el.scrollTop = target;
      lastProgrammaticTop = target;
      return;
    }
    if (smoothScrollRAF !== undefined) cancelAnimationFrame(smoothScrollRAF);
    const duration = 300;
    const startTime = performance.now();
    const animate = (now: number) => {
      if (!scrollContainerRef || !autoScroll()) {
        smoothScrollRAF = undefined;
        return;
      }
      const elapsed = now - startTime;
      const progress = Math.min(elapsed / duration, 1);
      const eased = 1 - Math.pow(1 - progress, 3);
      pendingSnaps += 1;
      scrollContainerRef.scrollTop = start + distance * eased;
      lastProgrammaticTop = scrollContainerRef.scrollTop;
      if (progress < 1) {
        smoothScrollRAF = requestAnimationFrame(animate);
      } else {
        smoothScrollRAF = undefined;
      }
    };
    smoothScrollRAF = requestAnimationFrame(animate);
  };

  /** Snap to the latest conversation after a topic switch or component remount.
   *  Message rows use content-visibility: auto (contain-intrinsic-size: auto 100px); unrendered
   *  rows lay out as 100px placeholders, so one scrollTop assignment only reaches the placeholder
   *  bottom. Progressive rendering expands about one viewport per frame, so correction continues
   *  over multiple frames until the position settles (cap 40 ≈ 200+ messages; short ones settle
   *  in 2-3). Aborts on user scroll-up; the first snap runs in a microtask (DOM committed, not
   *  yet painted), avoiding a one-frame flash at the wrong position. */
  const snapToLatestOnTopicSwitch = () => {
    queueMicrotask(() => {
      if (!scrollContainerRef || userScrolledUp) return;
      snapToBottom();
      let lastTop = -1;
      let guard = 0;
      const step = () => {
        if (!scrollContainerRef || userScrolledUp || guard >= 40) return;
        snapToBottom();
        const cur = scrollContainerRef.scrollTop;
        if (cur !== lastTop) {
          lastTop = cur;
          guard += 1;
          requestAnimationFrame(step);
        }
      };
      requestAnimationFrame(step);
    });
  };

  /** On topic switch: reset auto-scroll and the history-length baseline, then snap to the latest conversation */
  createEffect(
    on(
      () => props.activeTopic?.id,
      (id) => {
        setAutoScroll(true);
        userScrolledUp = false;
        // Clear animation memory (module-level set) only on real topic switches; remounting on
        // return from a subagent view re-runs this effect, and unconditional clearing replays all animations.
        if (id !== lastAnimatedTopicId) {
          animatedMessageIds.clear();
          lastAnimatedTopicId = id;
        }
        lastHistoryLen = props.activeTopic?.history?.length ?? 0;
        // Show the latest conversation right after a topic switch / remount (return from a
        // subagent view): the length-watching effect only scrolls when len > lastHistoryLen,
        // never on the switch path; without a snap the view stays clamped at the topic's start.
        snapToLatestOnTopicSwitch();
      },
    ),
  );

  /** Generation state change: reset auto-scroll on start; do one final smooth scroll on end */
  createEffect(
    on(
      () => props.isThinking,
      (thinking, prev) => {
        if (thinking && !prev) {
          // User sent a new message; generation starting
          setAutoScroll(true);
          userScrolledUp = false;
        } else if (!thinking && prev) {
          // Generation just ended: content may have just finished rendering; one final scroll, then stop
          if (autoScroll() && scrollContainerRef) {
            requestAnimationFrame(() => {
              if (autoScroll() && scrollContainerRef && !props.isThinking) {
                smoothScrollToBottom();
              }
            });
          }
        }
      },
    ),
  );

  /** Non-streaming case: smooth-scroll only when the message count grows (new messages), avoiding repeated re-snapping from post-generation reflow */
  createEffect(() => {
    const history = props.activeTopic?.history;
    const len = history?.length ?? 0;
    const tid = props.activeTopic?.id;
    void tid;

    if (props.isThinking) {
      lastHistoryLen = len;
      return;
    }
    if (!autoScroll() || !scrollContainerRef) {
      lastHistoryLen = len;
      return;
    }
    if (len > lastHistoryLen) {
      requestAnimationFrame(() => {
        if (autoScroll() && !props.isThinking && scrollContainerRef) {
          smoothScrollToBottom();
        }
      });
    }
    lastHistoryLen = len;
  });

  /** While streaming: ResizeObserver follows content growth to keep the view at the bottom.
   *  Observes the content wrapper (streamContentRef) because the scroll container is a
   *  fixed-height flex-grow box: content growth changes scrollHeight but not its box size, so
   *  RO on the container never fires past the initial observe() callback and streaming growth
   *  goes unfollowed (one snap at start and end only). The wrapper grows with content, firing
   *  on every expansion; the container is observed too (window resize / input-area height). */
  createEffect(() => {
    if (!props.isThinking || !streamContentRef) return;
    const observer = new ResizeObserver(() => {
      if (!userScrolledUp && autoScroll()) {
        snapToBottom();
      }
    });
    observer.observe(streamContentRef);
    if (scrollContainerRef) observer.observe(scrollContainerRef);
    onCleanup(() => observer.disconnect());
  });
  /** Cancel the smooth-scroll animation on component teardown */
  onCleanup(() => {
    if (smoothScrollRAF !== undefined) {
      cancelAnimationFrame(smoothScrollRAF);
    }
  });

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
              class={btnPrimarySm}
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
            <For each={props.activeTopic?.history}>
              {(msg: any, index) => {
                if (msg.role === 'tool') return null;
                const isActiveRound = createMemo(
                  () => index() === props.typingIndex && props.isThinking,
                );
                const fileChanges = msg.role === 'assistant' ? aggregateFileChanges(msg) : [];
                return (
                  <div class="flex items-start gap-2 mb-3">
                    {/* Selection mode: checkbox */}
                    <Show when={props.isSelectingMessages && msg.id}>
                      <div
                        class="flex-shrink-0 mt-1 cursor-pointer select-none"
                        onClick={() => props.onToggleMessage(msg.id!)}
                      >
                        <div
                          class="w-5 h-5 rounded border-2 flex items-center justify-center transition-all duration-150 hover:scale-110 active:scale-95"
                          classList={{
                            'bg-[rgba(var(--primary-rgb),0.25)] border-[rgba(var(--primary-rgb),0.5)]':
                              props.selectedMessageIds.has(msg.id!),
                            'bg-transparent border-white/[0.15] hover:border-white/[0.35]':
                              !props.selectedMessageIds.has(msg.id!),
                          }}
                        >
                          <Show when={props.selectedMessageIds.has(msg.id!)}>
                            <Icon
                              name="check"
                              class="w-3 h-3"
                              style={{ color: 'rgba(var(--primary-rgb),0.9)' }}
                            />
                          </Show>
                        </div>
                      </div>
                    </Show>
                    <div
                      ref={(el) => {
                        if (msg.id) {
                          animatedMessageIds.add(msg.id);
                          messageEls.set(msg.id, el);
                        }
                      }}
                      class={`flex flex-col flex-1 pointer-events-auto min-w-0 ${msg.id && !animatedMessageIds.has(msg.id) && !skipMessageAnimation() ? 'animate-message-in' : ''} ${msg.role === 'assistant' ? 'items-start' : 'items-end'}`}
                      style={{
                        // content-visibility skips offscreen rendering for performance; contain-intrinsic-size
                        // `auto <len>` remembers the element's last rendered height so it does not collapse
                        // to the 100px placeholder when skipped; otherwise total height shrinks and the
                        // browser clamps scrollTop upward (view jumps back toward the top on new output).
                        'content-visibility': 'auto',
                        'contain-intrinsic-size': 'auto 100px',
                      }}
                    >
                      <div
                        class={`flex gap-3 w-full ${msg.role === 'assistant' ? 'justify-start items-start' : 'justify-end items-start pr-[10px]'}`}
                      >
                        <Show when={msg.role === 'assistant'}>
                          <div
                            class="flex flex-shrink-0 items-center justify-center w-9 h-9 rounded-full overflow-hidden"
                            style={{
                              background: 'rgb(var(--text-base-rgb))',
                              border: '1px solid var(--border-dim)',
                              'box-shadow': '0 2px 6px rgba(0,0,0,0.25)',
                            }}
                          >
                            <Show
                              when={getModelLogo(msg.modelId || selectedModel()?.model_id || '')}
                              fallback={
                                <span
                                  class="text-[14px] font-bold select-none"
                                  style={{ color: 'var(--color-dark-700)' }}
                                >
                                  {(msg.modelId || selectedModel()?.model_id || 'AI')
                                    .charAt(0)
                                    .toUpperCase()}
                                </span>
                              }
                            >
                              <img
                                src={getModelLogo(msg.modelId || selectedModel()?.model_id || '')!}
                                alt={t('common.ai')}
                                class="w-[25px] h-[25px] rounded-full"
                              />
                            </Show>
                          </div>
                        </Show>

                        <div
                          class={`flex flex-col max-w-[75%] ${msg.role === 'assistant' ? 'items-start' : 'items-end'}`}
                        >
                          <div
                            class={`rounded-[10px] leading-relaxed max-w-full min-h-[1.5em] px-[14px] py-[10px] transition-[height] duration-200 break-words group relative ${
                              msg.role === 'assistant'
                                ? 'rounded-tl-[2px] text-white'
                                : 'rounded-tr-[2px] text-white'
                            } ${isActiveRound() && msg.content ? 'message-streaming' : ''}`}
                            style={{
                              background:
                                msg.role === 'assistant'
                                  ? 'rgba(var(--text-base-rgb),0.04)'
                                  : 'rgba(var(--primary-rgb),0.08)',
                              border:
                                msg.role === 'assistant'
                                  ? '1px solid rgba(var(--text-base-rgb),0.13)'
                                  : '1px solid rgba(var(--primary-rgb),0.16)',
                              'backdrop-filter': 'blur(6px)',
                            }}
                          >
                            <Show
                              when={
                                msg.role === 'user' &&
                                msg.displayFiles &&
                                msg.displayFiles.length > 0
                              }
                            >
                              <div class="flex flex-col gap-2 mb-2 first:mt-3">
                                <For each={msg.displayFiles}>
                                  {(file: any) => {
                                    const isImage = (file.mimeType || '').startsWith('image/');
                                    const ext = (file.name.split('.').pop() || '').toLowerCase();
                                    return (
                                      <div
                                        class="flex items-center cursor-default gap-3 max-w-[300px] px-[14px] py-[5px] rounded-lg transition-all duration-200"
                                        style={{
                                          background: 'rgba(var(--primary-rgb),0.06)',
                                          border: '1px solid rgba(var(--primary-rgb),0.12)',
                                        }}
                                      >
                                        {/* Unified thumbnail/icon box: images and attachments share the same size */}
                                        <Show
                                          when={isImage && file.storagePath}
                                          fallback={
                                            <div
                                              class="flex flex-shrink-0 items-center justify-center w-7 h-7 rounded-md [&_svg]:w-4 [&_svg]:h-4"
                                              style={{
                                                background: 'rgba(var(--primary-rgb),0.08)',
                                                color: 'rgba(var(--primary-rgb),0.7)',
                                              }}
                                            >
                                              <span
                                                ref={(node) => {
                                                  node.innerHTML = getLangIcon(ext);
                                                }}
                                              />
                                            </div>
                                          }
                                        >
                                          <img
                                            src={convertFileSrc(file.storagePath)}
                                            loading="lazy"
                                            class="w-7 h-7 flex-shrink-0 object-cover rounded-md border border-white/[0.08]"
                                          />
                                        </Show>
                                        <div class="flex-grow overflow-hidden">
                                          <div class="text-white text-[0.9rem] font-medium overflow-hidden text-ellipsis whitespace-nowrap">
                                            {file.name}
                                          </div>
                                        </div>
                                      </div>
                                    );
                                  }}
                                </For>
                              </div>
                            </Show>

                            <div class="mt-1">
                              {/* Agent process: reasoning + toolCalls + agentSteps in one collapsed block */}
                              <Show
                                when={
                                  msg.role === 'assistant' &&
                                  (msg.reasoning ||
                                    msg.interimContent ||
                                    (msg as any).toolCalls?.length > 0 ||
                                    (msg as any).agentSteps?.length > 0)
                                }
                              >
                                <AgentProcessBlock
                                  reasoning={msg.reasoning}
                                  toolCalls={(msg as any).toolCalls}
                                  interimContent={msg.interimContent}
                                  isActive={isActiveRound()}
                                  startTime={msg.agentStartTime}
                                  agentSteps={(msg as any).agentSteps}
                                />
                              </Show>
                              <Show
                                when={
                                  msg.role === 'assistant' &&
                                  !msg.content &&
                                  !(msg as any).toolCalls?.length &&
                                  !msg.reasoning
                                }
                                fallback={
                                  <Show when={msg.role !== 'tool'}>
                                    <div
                                      class={
                                        isActiveRound() && msg.content
                                          ? 'animate-stream-fade-in'
                                          : ''
                                      }
                                    >
                                      <Markdown
                                        content={
                                          msg.role === 'user' && msg.displayText !== undefined
                                            ? msg.displayText
                                            : msg.content
                                        }
                                      />
                                    </div>
                                  </Show>
                                }
                              >
                                <div class="flex items-center gap-2 py-1 text-white/50 italic text-[14px] select-none">
                                  <Icon
                                    src="/icons/app-logo/loading.svg"
                                    class="w-4 h-4 animate-spin opacity-50"
                                  />
                                  <span class="animate-pulse">{t('chat.thinking')}</span>
                                </div>
                              </Show>
                            </div>
                          </div>

                          <Show when={msg.images && msg.images.length > 0}>
                            <div class="flex flex-wrap gap-1.5 mt-2">
                              <For each={msg.images}>
                                {(img) => (
                                  <button
                                    class={btnBadge}
                                    onClick={() => downloadGeneratedImage(img)}
                                    title={t('common.download')}
                                  >
                                    <Icon name="download" class="w-3 h-3" />
                                    <span class="max-w-[160px] truncate">{img.name}</span>
                                    <span class="opacity-60">{formatSize(img.size)}</span>
                                  </button>
                                )}
                              </For>
                            </div>
                          </Show>

                          <Show
                            when={
                              msg.role === 'assistant' &&
                              (msg.modelId ||
                                selectedModel()?.model_id ||
                                msg.inputTokens ||
                                msg.outputTokens)
                            }
                          >
                            <div
                              style={{
                                color: 'rgba(var(--text-base-rgb),0.3)',
                                'font-family': 'monospace',
                                'font-size': '11px',
                                'margin-left': '4px',
                                'margin-top': '4px',
                                opacity: '0.7',
                                'user-select': 'none',
                                'text-align': 'left',
                                display: 'flex',
                                'align-items': 'center',
                                gap: '8px',
                              }}
                            >
                              <Show when={msg.modelId || selectedModel()?.model_id}>
                                <span>{msg.modelId || selectedModel()?.model_id}</span>
                              </Show>
                              <Show when={msg.inputTokens || msg.outputTokens}>
                                <span style={{ color: 'rgba(var(--text-base-rgb),0.2)' }}>·</span>
                                <Show when={msg.inputTokens}>
                                  <span title={t('chat.inputTokens')}>
                                    ↗{' '}
                                    {msg.inputTokens! >= 1000
                                      ? `${(msg.inputTokens! / 1000).toFixed(1)}K`
                                      : msg.inputTokens}
                                  </span>
                                </Show>
                                <Show when={msg.outputTokens}>
                                  <span title={t('chat.outputTokens')}>
                                    ↘{' '}
                                    {msg.outputTokens! >= 1000
                                      ? `${(msg.outputTokens! / 1000).toFixed(1)}K`
                                      : msg.outputTokens}
                                  </span>
                                </Show>
                              </Show>
                            </div>
                          </Show>

                          {/* Message action buttons: always visible */}
                          <div
                            class={`flex mt-1.5 px-[4px] z-[5] gap-1.5 flex-wrap ${msg.role === 'assistant' ? 'justify-start' : 'justify-end pr-[10px]'}`}
                          >
                            {/* Copy button (on every message) */}
                            <button
                              class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(var(--text-base-rgb),0.08)] hover:!p-[4px_8px]"
                              style={{
                                border: '1px solid rgba(var(--text-base-rgb),0.25)',
                                color: 'rgba(var(--text-base-rgb),0.6)',
                              }}
                              onClick={(e) => {
                                const currentBtn = e.currentTarget;
                                const text =
                                  msg.role === 'user' && msg.displayText !== undefined
                                    ? msg.displayText
                                    : msg.content;
                                if (!text) return;
                                navigator.clipboard.writeText(text).then(() => {
                                  const label = currentBtn.querySelector(
                                    '.action-label',
                                  ) as HTMLElement | null;
                                  if (label) {
                                    const originalText = label.innerText;
                                    currentBtn.style.color = 'var(--color-success)';
                                    currentBtn.style.borderColor = 'var(--color-success)';
                                    label.style.maxWidth = '60px';
                                    label.style.opacity = '1';
                                    label.innerText = t('common.copied');
                                    setTimeout(() => {
                                      currentBtn.style.color = '';
                                      currentBtn.style.borderColor = '';
                                      label.style.maxWidth = '';
                                      label.style.opacity = '';
                                      label.innerText = originalText;
                                    }, 2000);
                                  }
                                });
                              }}
                            >
                              <Icon
                                src="/icons/app-logo/clipboard-copy.svg"
                                class="w-[13px] h-[13px]"
                              />
                              <span class="action-label overflow-hidden whitespace-nowrap text-[11px] text-white max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[80px] group-hover:opacity-100">
                                {t('common.copy')}
                              </span>
                            </button>

                            {/* Branch button (assistant messages only) */}
                            <Show when={msg.role === 'assistant' && !!msg.id}>
                              <button
                                class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(var(--text-base-rgb),0.08)] hover:!p-[4px_8px]"
                                style={{
                                  border: '1px solid rgba(var(--text-base-rgb),0.25)',
                                  color: 'rgba(var(--text-base-rgb),0.6)',
                                }}
                                onClick={() => {
                                  if (msg.id) props.onBranchFromMessage(msg.id);
                                }}
                                title={t('chat.branchFromHere')}
                              >
                                <Icon
                                  name="git-branch"
                                  size={13}
                                  style={{ display: 'inline', 'vertical-align': 'middle' }}
                                />
                                <span class="action-label overflow-hidden whitespace-nowrap text-[11px] text-white max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[80px] group-hover:opacity-100">
                                  {t('chat.branch')}
                                </span>
                              </button>
                            </Show>

                            {/* Last user message: resend + edit-and-resend */}
                            <Show
                              when={
                                index() === lastUserMsgIndex() &&
                                msg.role === 'user' &&
                                (msg.content || msg.displayText)
                              }
                            >
                              <button
                                class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(var(--text-base-rgb),0.08)] hover:!p-[4px_8px]"
                                style={{
                                  border: '1px solid rgba(var(--text-base-rgb),0.25)',
                                  color: 'rgba(var(--text-base-rgb),0.6)',
                                }}
                                onClick={async () => {
                                  const text =
                                    msg.displayText !== undefined ? msg.displayText : msg.content;
                                  if (!text) return;
                                  if (props.isThinking) {
                                    // CHAT-11: wait for the stream to actually stop
                                    // (isThinking flips in the done handler) instead of
                                    // sleeping a magic 150ms and racing the epilogue.
                                    await props.handleStopGeneration();
                                    await awaitGenerationStopped();
                                  }
                                  const asstId = currentAssistantId();
                                  const tId = currentTopicId();
                                  if (asstId && tId) {
                                    const reply = lastAssistantReply();
                                    // Skip the exit animation and delete directly; also suppress the entrance animation of the new message
                                    setSkipMessageAnimation(true);
                                    setTimeout(() => setSkipMessageAnimation(false), 100);
                                    if (reply?.id) {
                                      try {
                                        await deleteTopicMessage(tId, reply.id!);
                                      } catch (e) {
                                        console.error('Failed to delete assistant reply:', e);
                                      }
                                      setDatas(
                                        'assistants',
                                        (a) => a.id === asstId,
                                        'topics',
                                        (t) => t.id === tId,
                                        'history',
                                        (h) => h.filter((m: any) => m.id !== reply.id),
                                      );
                                    }
                                    if (msg.id) {
                                      try {
                                        await deleteTopicMessage(tId, msg.id!);
                                      } catch (e) {
                                        console.error('Failed to delete message:', e);
                                      }
                                      setDatas(
                                        'assistants',
                                        (a) => a.id === asstId,
                                        'topics',
                                        (t) => t.id === tId,
                                        'history',
                                        (h) => h.filter((m: any) => m.id !== msg.id),
                                      );
                                    }
                                  }
                                  props.setInputMessage(text);
                                  setTimeout(() => props.handleSendMessage(), 0);
                                }}
                              >
                                <svg
                                  xmlns="http://www.w3.org/2000/svg"
                                  fill="none"
                                  viewBox="0 0 24 24"
                                  stroke-width="1.5"
                                  stroke="currentColor"
                                  class="w-[13px] h-[13px]"
                                >
                                  <path
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                    d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0l3.181 3.183a8.25 8.25 0 0013.803-3.7M4.031 9.865a8.25 8.25 0 0113.803-3.7l3.181 3.182"
                                  />
                                </svg>
                                <span class="action-label overflow-hidden whitespace-nowrap text-[11px] text-white max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[80px] group-hover:opacity-100">
                                  {t('chat.resendMessage')}
                                </span>
                              </button>
                              <button
                                class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(var(--text-base-rgb),0.08)] hover:!p-[4px_8px]"
                                style={{
                                  border: '1px solid rgba(var(--text-base-rgb),0.25)',
                                  color: 'rgba(var(--text-base-rgb),0.6)',
                                }}
                                onClick={async () => {
                                  const text =
                                    msg.displayText !== undefined ? msg.displayText : msg.content;
                                  if (!text) return;
                                  if (props.isThinking) {
                                    // CHAT-11: wait for the stream to actually stop
                                    // (isThinking flips in the done handler) instead of
                                    // sleeping a magic 150ms and racing the epilogue.
                                    await props.handleStopGeneration();
                                    await awaitGenerationStopped();
                                  }
                                  const asstId = currentAssistantId();
                                  const tId = currentTopicId();
                                  if (asstId && tId) {
                                    const reply = lastAssistantReply();
                                    // Skip the exit animation and delete directly; also suppress the entrance animation of the new message
                                    setSkipMessageAnimation(true);
                                    setTimeout(() => setSkipMessageAnimation(false), 100);
                                    if (reply?.id) {
                                      try {
                                        await deleteTopicMessage(tId, reply.id!);
                                      } catch (e) {
                                        console.error('Failed to delete assistant reply:', e);
                                      }
                                      setDatas(
                                        'assistants',
                                        (a) => a.id === asstId,
                                        'topics',
                                        (t) => t.id === tId,
                                        'history',
                                        (h) => h.filter((m: any) => m.id !== reply.id),
                                      );
                                    }
                                    if (msg.id) {
                                      try {
                                        await deleteTopicMessage(tId, msg.id!);
                                      } catch (e) {
                                        console.error('Failed to delete message:', e);
                                      }
                                      setDatas(
                                        'assistants',
                                        (a) => a.id === asstId,
                                        'topics',
                                        (t) => t.id === tId,
                                        'history',
                                        (h) => h.filter((m: any) => m.id !== msg.id),
                                      );
                                    }
                                  }
                                  props.setInputMessage(text);
                                  if (textareaRef) {
                                    textareaRef.focus();
                                    textareaRef.scrollIntoView({
                                      behavior: 'smooth',
                                      block: 'center',
                                    });
                                  }
                                }}
                              >
                                <svg
                                  xmlns="http://www.w3.org/2000/svg"
                                  fill="none"
                                  viewBox="0 0 24 24"
                                  stroke-width="1.5"
                                  stroke="currentColor"
                                  class="w-[13px] h-[13px]"
                                >
                                  <path
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                    d="M16.862 4.487l1.687-1.688a1.875 1.875 0 112.652 2.652L10.582 16.07a4.5 4.5 0 01-1.897 1.13L6 18l.8-2.685a4.5 4.5 0 011.13-1.897l8.932-8.931zm0 0L19.5 7.125M18 14v4.75A2.25 2.25 0 0115.75 21H5.25A2.25 2.25 0 013 18.75V8.25A2.25 2.25 0 015.25 6H10"
                                  />
                                </svg>
                                <span class="action-label overflow-hidden whitespace-nowrap text-[11px] text-white max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[80px] group-hover:opacity-100">
                                  {t('common.edit')}
                                </span>
                              </button>
                            </Show>

                            {/* Delete button (on every message, placed rightmost) */}
                            <button
                              class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(248,113,113,0.1)] hover:!p-[4px_8px]"
                              style={{
                                border: '1px solid rgba(248,113,113,0.35)',
                                color: 'rgba(248,113,113,0.75)',
                              }}
                              onClick={async () => {
                                const asstId = currentAssistantId();
                                const tId = currentTopicId();
                                if (!asstId || !tId || !msg.id) return;
                                // Play the exit animation directly on the DOM, without triggering a reactive re-render
                                const el = messageEls.get(msg.id);
                                if (el) el.classList.add('animate-message-out');
                                await new Promise((r) => setTimeout(r, 260));
                                try {
                                  await deleteTopicMessage(tId, msg.id!);
                                } catch (e) {
                                  console.error('Failed to delete message:', e);
                                }
                                setDatas(
                                  'assistants',
                                  (a) => a.id === asstId,
                                  'topics',
                                  (t) => t.id === tId,
                                  'history',
                                  (h) => h.filter((m: any) => m.id !== msg.id),
                                );
                              }}
                            >
                              <svg
                                xmlns="http://www.w3.org/2000/svg"
                                fill="none"
                                viewBox="0 0 24 24"
                                stroke-width="1.5"
                                stroke="currentColor"
                                class="w-[13px] h-[13px]"
                              >
                                <path
                                  stroke-linecap="round"
                                  stroke-linejoin="round"
                                  d="M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 01-2.244 2.077H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0"
                                />
                              </svg>
                              <span class="action-label overflow-hidden whitespace-nowrap text-[11px] max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[60px] group-hover:opacity-100">
                                {t('chat.deleteMessage')}
                              </span>
                            </button>
                          </div>
                        </div>
                      </div>

                      {/* File Changes box (assistant messages only, shown when there are file changes) */}
                      <Show when={fileChanges.length > 0}>
                        {(() => {
                          const isReverted = () =>
                            revertedMessages().has(msg.id ?? String(index()));
                          const diffStats = () => aggregateDiffStats(fileChanges);
                          const showUndo = () =>
                            !isReverted() && !!currentProject()?.path && !!gitBranch();
                          const handleUndoAll = async () => {
                            const projectPath = currentProject()?.path;
                            if (!projectPath) return;
                            try {
                              await revertFileChangesBatch(
                                projectPath,
                                fileChanges.map((fc) => fc.filePath),
                              );
                              setRevertedMessages((prev) => {
                                const next = new Set(prev);
                                next.add(msg.id ?? String(index()));
                                return next;
                              });
                            } catch (err) {
                              console.error('Failed to batch-revert:', err);
                              alert(t('agent.diff.revertFailed'));
                            }
                          };
                          return (
                            <div
                              class="rounded-lg overflow-hidden transition-opacity duration-300 w-[80%] self-center mt-3"
                              classList={{ 'opacity-40': isReverted() }}
                              style={{
                                border: '1px solid var(--border-dim)',
                                background: 'rgba(var(--text-base-rgb),0.015)',
                              }}
                            >
                              <div
                                class="flex items-center gap-2 px-3 py-1.5"
                                style={{
                                  'border-bottom': '1px solid var(--border-dim)',
                                  color: 'rgba(var(--text-base-rgb),0.4)',
                                  'font-size': '14px',
                                }}
                              >
                                <svg
                                  xmlns="http://www.w3.org/2000/svg"
                                  fill="none"
                                  viewBox="0 0 24 24"
                                  stroke-width="1.5"
                                  stroke="currentColor"
                                  class="w-4 h-4"
                                  style={{ opacity: '0.6' }}
                                >
                                  <path
                                    stroke-linecap="round"
                                    stroke-linejoin="round"
                                    d="M19.5 14.25v-2.625a3.375 3.375 0 00-3.375-3.375h-1.5A1.125 1.125 0 0113.5 7.125v-1.5a3.375 3.375 0 00-3.375-3.375H8.25m0 12.75h7.5m-7.5 3H12M10.5 2.25H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 00-9-9z"
                                  />
                                </svg>
                                <Show
                                  when={isReverted()}
                                  fallback={
                                    <span class="font-medium">{t('agent.diff.title')}</span>
                                  }
                                >
                                  <span
                                    class="font-medium"
                                    style={{ 'text-decoration': 'line-through' }}
                                  >
                                    {t('agent.diff.title')}
                                  </span>
                                </Show>
                                <span style={{ color: 'rgba(var(--text-base-rgb),0.25)' }}>
                                  {t('agent.diff.files', {
                                    count: formatNumber(fileChanges.length),
                                  })}
                                </span>
                                <Show when={diffStats().added > 0 || diffStats().deleted > 0}>
                                  <span class="font-mono">
                                    {diffStats().added > 0 && (
                                      <span style={{ color: 'var(--color-success)' }}>
                                        +{diffStats().added}
                                      </span>
                                    )}
                                    {diffStats().added > 0 && diffStats().deleted > 0 && (
                                      <span style={{ color: 'rgba(var(--text-base-rgb),0.18)' }}>
                                        {' '}
                                      </span>
                                    )}
                                    {diffStats().deleted > 0 && (
                                      <span style={{ color: 'var(--color-danger)' }}>
                                        -{diffStats().deleted}
                                      </span>
                                    )}
                                  </span>
                                </Show>
                                <div class="flex-1" />
                                <Show when={isReverted()}>
                                  <span
                                    class="px-1.5 py-px rounded text-[12px] font-medium"
                                    style={{
                                      color: 'rgba(80, 220, 100, 0.7)',
                                      background: 'rgba(80, 220, 100, 0.08)',
                                      border: '1px solid rgba(80, 220, 100, 0.15)',
                                    }}
                                  >
                                    {t('agent.diff.reverted')}
                                  </span>
                                </Show>
                                <Show when={showUndo()}>
                                  <button
                                    type="button"
                                    class={'px-1.5 py-0.5 text-[12px] font-medium ' + btnGhost}
                                    onClick={(e) => {
                                      e.stopPropagation();
                                      handleUndoAll();
                                    }}
                                    title={t('agent.diff.revertAllTitle')}
                                  >
                                    <svg
                                      xmlns="http://www.w3.org/2000/svg"
                                      fill="none"
                                      viewBox="0 0 24 24"
                                      stroke-width="2"
                                      stroke="currentColor"
                                      class="w-3.5 h-3.5"
                                      style={{ opacity: '0.6' }}
                                    >
                                      <path
                                        stroke-linecap="round"
                                        stroke-linejoin="round"
                                        d="M9 15L3 9m0 0l6-6M3 9h12a6 6 0 010 12h-3"
                                      />
                                    </svg>
                                    {t('agent.diff.revertAll')}
                                  </button>
                                </Show>
                              </div>
                              <div class="p-2">
                                <DiffView changes={fileChanges} allReverted={isReverted()} />
                              </div>
                            </div>
                          );
                        })()}
                      </Show>
                    </div>
                  </div>
                );
              }}
            </For>

            {/* /btw overlay Q&A boxes */}
            <Show when={props.btwOverlays.length > 0}>
              <div class="flex flex-col items-center mb-3 space-y-2">
                <For each={props.btwOverlays}>
                  {(item) => <BtwResponseBox item={item} onDismiss={props.onDismissBtw} />}
                </For>
              </div>
            </Show>
          </Show>

          {/* Tool-call approval bubbles */}
          <Show when={props.pendingApprovals.length > 0}>
            <div class="mb-3 space-y-2">
              <For each={props.pendingApprovals}>
                {(approval) => (
                  <ToolApprovalBubble approval={approval} onResolved={props.onResolveApproval} />
                )}
              </For>
            </div>
          </Show>

          {/* Agent question bubbles (ask_user): options + custom input + skip */}
          <Show when={props.pendingQuestions.length > 0}>
            <div class="mb-3 space-y-2">
              <For each={props.pendingQuestions}>
                {(question) => (
                  <AgentQuestionBubble question={question} onResolved={props.onResolveQuestion} />
                )}
              </For>
            </div>
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
          onClick={() => {
            userScrolledUp = false;
            setAutoScroll(true);
            smoothScrollToBottom();
          }}
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

      <div class="flex flex-wrap gap-[3px] bg-transparent pt-[3px] relative z-0">
        <For each={props.pendingFiles}>
          {(file, i) => (
            <div
              class="flex items-center rounded-[16px] text-[12px] px-[10px] py-1 transition-all duration-200"
              style={{
                background: 'rgba(var(--primary-rgb),0.08)',
                border: '1px solid rgba(var(--primary-rgb),0.12)',
                color: 'rgba(var(--primary-rgb),0.6)',
              }}
            >
              <Show
                when={file.type === 'image'}
                fallback={
                  <span class="mr-1 inline-flex">
                    <Icon name="file" size={12} class="opacity-60" />
                  </span>
                }
              >
                <img
                  src={file.previewUrl}
                  loading="lazy"
                  class="w-5 h-5 object-cover mr-1 rounded-[2px]"
                />
              </Show>
              {file.name}
              <button
                class="flex items-center bg-none border-none text-[rgba(var(--text-base-rgb),0.5)] cursor-pointer text-lg leading-none ml-2 transition-colors duration-200 hover:text-[var(--color-danger)]"
                onClick={() => {
                  const currentIndex = i();
                  const duplicatePending = props.pendingFiles.some(
                    (pending, index) => index !== currentIndex && pending.id === file.id,
                  );
                  if (!duplicatePending) {
                    void discardChatAttachment(file.id).catch(console.error);
                  }
                  props.setPendingFiles((p) => p.filter((_, idx) => idx !== currentIndex));
                }}
              >
                ×
              </button>
            </div>
          )}
        </For>
      </div>

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
                plan: t('agent.mode.plan'),
                workflow: t('agent.mode.workflow'),
              }[mode as 'normal' | 'auto' | 'plan' | 'workflow'] || mode;
            return (
              <div
                class="flex items-center gap-2 px-3 h-8 rounded-2xl text-xs"
                style={{
                  background: 'rgba(var(--surface-bg), 0.5)',
                  border: '1px solid rgba(var(--border-rgb), 0.15)',
                }}
              >
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
                          background: 'rgba(var(--surface-bg),0.96)',
                          border: '1px solid var(--border-dim)',
                          'backdrop-filter': 'blur(12px)',
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

                {/* Subagent pull-up menu: right side of the status bar, shown when any subagent exists */}
                <Show when={subagents().length > 0}>
                  <div class="subagent-menu-container relative inline-block ml-auto">
                    <button
                      type="button"
                      class={
                        'px-1.5 py-0.5 rounded-md text-[11px] bg-[rgba(var(--surface-alt-bg),0.5)] ' +
                        btnGhost
                      }
                      style={{
                        color: 'rgba(var(--text-base-rgb),0.55)',
                        'font-weight': '400',
                      }}
                      classList={{
                        // 与分支触发器同款全响应式方案（见上方注释）
                        '!bg-[rgba(var(--surface-bg),0.7)]': subagentMenuOpen(),
                        'hover:bg-[rgba(var(--surface-bg),0.6)]': !subagentMenuOpen(),
                      }}
                      title={t('agent.subagent.menuTrigger', { count: subagents().length })}
                      aria-label={t('agent.subagent.menuTrigger', { count: subagents().length })}
                      aria-haspopup="menu"
                      aria-expanded={subagentMenuOpen()}
                      onClick={(e) => {
                        e.stopPropagation();
                        setSubagentMenuOpen(!subagentMenuOpen());
                      }}
                    >
                      {t('agent.subagent.menuTitle')}
                      <Show when={hasRunningSubagent()}>
                        <Icon name="spinner" size={11} class="animate-spin mx-0.5" />
                      </Show>
                      {` ${t('agent.subagent.menuCount', { count: subagents().length })} `}
                      <Icon name="chevron-up" size={10} class="ml-0.5" />
                    </button>
                    <Show when={subagentMenuOpen()}>
                      <div
                        ref={(el) =>
                          requestAnimationFrame(() => {
                            el.classList.remove('opacity-0', 'scale-95');
                            el.classList.add('opacity-100', 'scale-100');
                          })
                        }
                        class="absolute bottom-full right-0 mb-2 z-[41] w-[320px] rounded-xl overflow-hidden opacity-0 scale-95 transition-all duration-200 ease-out origin-bottom"
                        style={{
                          background: 'rgba(var(--surface-bg),0.96)',
                          border: '1px solid var(--border-dim)',
                          'backdrop-filter': 'blur(12px)',
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
                          {t('agent.subagent.menuTitle')}
                        </div>
                        <div class="py-1 max-h-[280px] overflow-y-auto">
                          <For each={subagents()}>
                            {(sub) => {
                              const ps = getSubagentProfileStyle(sub.subagentProfile);
                              return (
                                <button
                                  type="button"
                                  class="w-full flex items-center gap-2 px-3 py-2 text-left transition-colors cursor-pointer border-none bg-transparent hover:bg-[rgba(var(--text-base-rgb),0.05)]"
                                  style={{ color: 'rgba(var(--text-base-rgb),0.75)' }}
                                  onClick={() => {
                                    if (sub.subagentId) setActiveSubagentView(sub.subagentId);
                                    setSubagentMenuOpen(false);
                                  }}
                                >
                                  <span class="flex items-center justify-center w-4 h-4 shrink-0">
                                    <Show
                                      when={sub.status === 'running'}
                                      fallback={
                                        <Show
                                          when={sub.status === 'error'}
                                          fallback={
                                            <Icon
                                              name="check"
                                              size={12}
                                              class="text-white/30 shrink-0"
                                            />
                                          }
                                        >
                                          <Icon
                                            name="x"
                                            size={12}
                                            class="text-red-400/70 shrink-0"
                                          />
                                        </Show>
                                      }
                                    >
                                      <Icon
                                        name="spinner"
                                        size={12}
                                        class="animate-spin shrink-0"
                                      />
                                    </Show>
                                  </span>
                                  <span
                                    class="flex items-center justify-center w-5 h-5 rounded shrink-0"
                                    style={{
                                      background: ps.bgColor,
                                      border: `1px solid ${ps.borderColor}`,
                                      color: ps.color,
                                    }}
                                  >
                                    <Icon name={ps.icon} size={11} />
                                  </span>
                                  <span class="flex-1 min-w-0 text-left">
                                    <span class="flex items-center gap-1.5">
                                      <span class="text-[12px] font-medium truncate">
                                        {sub.subagentName ||
                                          subagentProfileLabel(sub.subagentProfile)}
                                      </span>
                                      <span class="text-[10px] text-white/35 shrink-0">
                                        {subagentProfileLabel(sub.subagentProfile)}
                                      </span>
                                    </span>
                                    <Show when={sub.subagentTask}>
                                      <span class="block truncate text-[11px] text-white/35">
                                        {sub.subagentTask}
                                      </span>
                                    </Show>
                                  </span>
                                  <Show
                                    when={sub.status !== 'running' && sub.duration !== undefined}
                                  >
                                    <span class="font-mono text-[10px] text-white/25 shrink-0">
                                      {formatSubagentDuration(sub.duration!)}
                                    </span>
                                  </Show>
                                </button>
                              );
                            }}
                          </For>
                        </div>
                      </div>
                    </Show>
                  </div>
                </Show>
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
            onInput={(e) => {
              props.setInputMessage(e.currentTarget.value);
              e.currentTarget.style.height = 'auto';
              const newHeight = e.currentTarget.scrollHeight;
              e.currentTarget.style.height = `${newHeight}px`;
              if (newHeight > 200) {
                e.currentTarget.style.overflowY = 'auto';
                e.currentTarget.style.height = '200px';
              } else {
                e.currentTarget.style.overflowY = 'hidden';
              }
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.ctrlKey && !e.shiftKey) {
                e.preventDefault();
                props.handleSendMessage();
                if (textareaRef) textareaRef.style.height = '40px';
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

              {/* LSP problems panel toggle */}
              <Show when={hasDiagnostics()}>
                <button
                  class={'relative p-1.5 ' + btnIcon}
                  style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                  title={t('chat.diagnostics', {
                    errors: formatNumber(totalErrors()),
                    warnings: formatNumber(totalWarnings()),
                  })}
                  onClick={() => setProblemsPanelVisible(!problemsPanelVisible())}
                  onMouseEnter={(e) => {
                    e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.06)';
                    e.currentTarget.style.color = 'rgba(var(--text-base-rgb),0.6)';
                  }}
                  onMouseLeave={(e) => {
                    e.currentTarget.style.background = 'transparent';
                    e.currentTarget.style.color = 'rgba(var(--text-base-rgb),0.4)';
                  }}
                >
                  <Icon name="clipboard" size={14} />
                  <Show when={totalErrors() > 0}>
                    <span class="absolute -top-1 -right-1 text-[9px] px-1 rounded-full bg-red-500 text-white font-bold leading-tight">
                      {totalErrors()}
                    </span>
                  </Show>
                </button>
              </Show>
            </div>

            <div class="flex items-center gap-2">
              <TokenStatsBar />
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
              background: 'rgba(var(--surface-bg), 0.92)',
              border: '1px solid rgba(248,113,113,0.2)',
              'backdrop-filter': 'blur(30px) saturate(180%)',
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
