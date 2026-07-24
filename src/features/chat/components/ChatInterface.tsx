import { Component, For, Show, Setter, createSignal, createEffect, createMemo, onCleanup, onMount, on } from 'solid-js';
import { Portal } from 'solid-js/web';
import Markdown from '../../../shared/components/Markdown';
import AgentProcessBlock from './AgentProcessBlock';
import ModelSelector from './ModelSelector';
import { Topic, PendingAttachment, globalUserAvatar, selectedModel, isStartingLocalModel, localModelStartProgress, currentProjectId, currentProject, datas, setDatas, currentAssistantId, currentTopicId, isChatMode, mcpServerStatus, gitBranch, gitBranches, switchBranch, type AgentMode, type FileChangeInfo } from '../../../core/store/store';
import { open } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import { getLogo as getLogoByIds } from '../../../core/utils/modelLogo';
import { registerCommand, unregisterCommand } from '../../../core/shortcuts';
import SlashCommandMenu from '../../../shared/components/SlashCommandMenu';
import Icon from '../../../shared/components/Icon';
import ReasoningButton from './ReasoningButton';
import WebSearchButton from './WebSearchButton';
import ToolCallBubble from './ToolCallBubble';
import ToolApprovalBubble, { type PendingApproval } from './ToolApprovalBubble';
import TokenStatsBar from './TokenStatsBar';
import { totalErrors, totalWarnings, problemsPanelVisible, setProblemsPanelVisible, hasDiagnostics } from '../../../core/store/diagnostics';
import AgentModeSelector from './AgentModeSelector';
import WelcomeScreen from './WelcomeScreen';
import DiffView from '../../../shared/components/DiffView';


/** 聚合后的文件变更条目，与 FileChangeInfo 同形但条目唯一 */
type AggregatedFileChange = FileChangeInfo;

/** 从消息的 toolCalls 中聚合所有文件变更，按 filePath 去重 */
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

/** 从聚合的文件变更中计算增减行数统计 */
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
    /** 是否显示分享按钮（仅纯对话模式） */
    canShare: boolean;
    /** 打开分享弹窗的回调（进入消息选择模式） */
    onOpenShare: () => void;
    /** 消息选择模式 */
    isSelectingMessages: boolean;
    /** 当前选中的消息 ID 集合 */
    selectedMessageIds: Set<string>;
    /** 切换单条消息选中 */
    onToggleMessage: (msgId: string) => void;
    /** 全选 */
    onSelectAll: () => void;
    /** 取消选择 */
    onCancelSelection: () => void;
    /** 确认选择 */
    onConfirmSelection: () => void;
    /** 从消息处分叉 */
    onBranchFromMessage: (messageId: string) => void;
}

const UserMessageAvatar: Component = () => {
    const avatarSrc = () => globalUserAvatar();

    return (
        <div class="relative flex flex-shrink-0 items-center justify-center w-9 h-9 rounded-full overflow-hidden"
             style="background: rgba(255,255,255,0.06); border: 1px solid rgba(255,255,255,0.04); box-shadow: 0 2px 6px rgba(0,0,0,0.15);">
            <img
                src={avatarSrc()}
                alt="User"
                class="w-full h-full object-cover"
                onError={(e) => { e.currentTarget.src = '/icons/app-logo/user.svg'; }}
            />
            <div
                class="absolute inset-0 w-full h-full flex items-center justify-center pointer-events-none"
                style="background: rgba(255,255,255,0.02);"
            />
        </div>
    );
};

const ChatInterface: Component<ChatInterfaceProps> = (props) => {
    let textareaRef: HTMLTextAreaElement | undefined;
    let scrollContainerRef: HTMLDivElement | undefined;
    const [autoScroll, setAutoScroll] = createSignal(true);
    // 抑制程序滚动触发的 scroll 事件，避免误判为用户滚动
    let suppressScroll = false;
    // 平滑滚动动画的 rAF ID
    let smoothScrollRAF: number | undefined;
    // 上一次记录到的历史消息数量，用于判断是否新增了消息
    let lastHistoryLen = 0;
    // 流式 rAF 循环的最新 ID，始终指向最后一个排期的帧，保证能正确取消
    // 用户手动上滚标志：直接可变变量，绕过 SolidJS 响应式延迟，确保 rAF 回调能同步感知
    let userScrolledUp = false;
    // 已播放入场动画的消息 ID 集合，避免 agentSteps 更新时重复触发动画
    let animatedMessageIds = new Set<string>();
    // 重新发送/编辑时跳过消息动画（不播退场 + 不播入场）
    const [skipMessageAnimation, setSkipMessageAnimation] = createSignal(false);
    // 流式 rAF 循环的最新 ID，始终指向最后一个排期的帧，保证能正确取消
    let streamRAFId: number | undefined;
    // Git 分支下拉状态
    const [branchOpen, setBranchOpen] = createSignal(false);
    const [branchToast, setBranchToast] = createSignal<string | null>(null);
    // 消息 DOM 元素映射（id → HTMLElement），用于退场动画
    const messageEls = new Map<string, HTMLElement>();
    // 已撤销的消息 ID 集合（本地信号，不持久化）
    const [revertedMessages, setRevertedMessages] = createSignal<Set<string>>(new Set());

    // 点击外部关闭 Git 分支下拉
    const closeBranch = (e: MouseEvent) => {
        if (branchOpen()) {
            const target = e.target as HTMLElement;
            if (!target.closest('.branch-dropdown-container')) {
                setBranchOpen(false);
            }
        }
    };
    onMount(() => document.addEventListener('mousedown', closeBranch));
    onCleanup(() => document.removeEventListener('mousedown', closeBranch));

    // ---- 注册快捷键命令 ----
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
                    filters: [{ name: '所有文件', extensions: ['*'] }],
                });
                if (selected) {
                    const path = typeof selected === 'string'
                        ? selected
                        : 'path' in selected
                            ? (selected as any).path
                            : String(selected);
                    await props.handleFileUpload(path, 'file');
                }
            } catch (e) {
                console.warn('[shortcut] 上传文件取消或失败:', e);
            }
        },
    });

    // 组件销毁时清理命令注册
    onCleanup(() => {
        unregisterCommand(cmdFocusInput);
        unregisterCommand(cmdUploadFile);
    });

    const getModelLogo = (modelName: string) => {
        return getLogoByIds(null, modelName);
    };

    /** 检测是否已滚动到底部（阈值 50px） */
    const isAtBottom = () => {
        if (!scrollContainerRef) return true;
        const el = scrollContainerRef;
        return el.scrollHeight - el.scrollTop - el.clientHeight < 50;
    };

    /** 鼠标滚轮向上：立即停止自动滚动并取消进行中的平滑滚动 */
    const handleWheel = (e: WheelEvent) => {
        if (e.deltaY < 0) {
            // 直接可变标志：下个 rAF 帧立刻感知，零延迟停止跟底
            userScrolledUp = true;
            // 取消流式 rAF 循环的最新帧
            if (streamRAFId !== undefined) {
                cancelAnimationFrame(streamRAFId);
                streamRAFId = undefined;
            }
            if (smoothScrollRAF !== undefined) {
                cancelAnimationFrame(smoothScrollRAF);
                smoothScrollRAF = undefined;
            }
            suppressScroll = false;
            setAutoScroll(false);
        }
    };

    /** 滚动事件：区分用户滚动与程序滚动 */
    const handleScroll = () => {
        if (suppressScroll) {
            suppressScroll = false;
            return;
        }
        const atBottom = isAtBottom();
        setAutoScroll(atBottom);
        // 用户手动滚回底部：清除上滚标志，恢复自动跟底
        if (atBottom) {
            userScrolledUp = false;
        }
    };

    /** 瞬时滚动到底部（流式期间使用，抑制 scroll 事件） */
    const snapToBottom = () => {
        if (!scrollContainerRef) return;
        const el = scrollContainerRef;
        const target = el.scrollHeight - el.clientHeight;
        if (el.scrollTop < target) {
            suppressScroll = true;
            el.scrollTop = el.scrollHeight;
        }
    };

    /** 平滑滚动到底部（rAF 动画，用于非流式场景） */
    const smoothScrollToBottom = () => {
        if (!scrollContainerRef) return;
        const el = scrollContainerRef;
        const target = el.scrollHeight - el.clientHeight;
        const start = el.scrollTop;
        const distance = target - start;
        if (Math.abs(distance) < 2) {
            el.scrollTop = target;
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
            suppressScroll = true;
            scrollContainerRef.scrollTop = start + distance * eased;
            if (progress < 1) {
                smoothScrollRAF = requestAnimationFrame(animate);
            } else {
                smoothScrollRAF = undefined;
            }
        };
        smoothScrollRAF = requestAnimationFrame(animate);
    };

    /** 切换话题时重置自动滚动与历史长度基线 */
    createEffect(on(() => props.activeTopic?.id, () => {
        setAutoScroll(true);
        userScrolledUp = false;
        animatedMessageIds = new Set();
        lastHistoryLen = props.activeTopic?.history?.length ?? 0;
    }));

    /** 生成状态变化：开始时重置自动滚动，结束时做一次收尾平滑滚动 */
    createEffect(on(() => props.isThinking, (thinking, prev) => {
        if (thinking && !prev) {
            // 用户发送新消息，开始生成
            setAutoScroll(true);
            userScrolledUp = false;
        } else if (!thinking && prev) {
            // 生成刚结束：内容可能刚刚完成渲染，做一次最终滚动后即停止
            if (autoScroll() && scrollContainerRef) {
                requestAnimationFrame(() => {
                    if (autoScroll() && scrollContainerRef && !props.isThinking) {
                        smoothScrollToBottom();
                    }
                });
            }
        }
    }));

    /** 非流式场景：仅在消息数量增加（新增消息）时平滑滚动，避免生成结束后内容重排反复吸附 */
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

    /** 流式输出期间：rAF 循环跟随内容增长平滑滚动 */
    createEffect(() => {
        if (!props.isThinking || !scrollContainerRef) return;

        let running = true;
        const scroll = () => {
            // userScrolledUp 为直接可变变量，确保滚轮事件后最速响应
            if (!running || userScrolledUp || !autoScroll()) return;
            snapToBottom();
            streamRAFId = requestAnimationFrame(scroll);
        };
        streamRAFId = requestAnimationFrame(scroll);
        onCleanup(() => {
            running = false;
            if (streamRAFId !== undefined) {
                cancelAnimationFrame(streamRAFId);
                streamRAFId = undefined;
            }
        });
    });

    /** 组件销毁时清理平滑滚动动画 */
    onCleanup(() => {
        if (smoothScrollRAF !== undefined) {
            cancelAnimationFrame(smoothScrollRAF);
        }
    });

    /** 计算最后一条用户消息的索引，用于显示"重新发送"等按钮 */
    const lastUserMsgIndex = createMemo(() => {
        const history = props.activeTopic?.history;
        if (!history || !Array.isArray(history)) return -1;
        let lastIdx = -1;
        for (let i = 0; i < history.length; i++) {
            if (history[i]?.role === 'user') lastIdx = i;
        }
        return lastIdx;
    });

    /** 最后一条助手回复（紧接在最后一条用户消息之后），用于重发/编辑时一并删除 */
    const lastAssistantReply = createMemo(() => {
        const history = props.activeTopic?.history;
        if (!history || !Array.isArray(history)) return null;
        const lastUserIdx = lastUserMsgIndex();
        if (lastUserIdx < 0 || lastUserIdx >= history.length - 1) return null;
        const next = history[lastUserIdx + 1];
        return next?.role === 'assistant' ? next : null;
    });

    /** 当前助手的工作模式，用于控制工作目录选择器显隐 */
    const currentAgentMode = (): AgentMode => {
        const id = currentAssistantId();
        if (!id) return 'off';
        const asst = datas.assistants.find(a => a.id === id) as any;
        return asst?.agentMode || 'off';
    };

    return (
        <div class="flex flex-col flex-grow items-stretch rounded-[12px] box-border overflow-hidden p-[15px] pb-5 relative h-full"
             style="background: rgba(18, 22, 35, 0.12); backdrop-filter: blur(20px); border: 1px solid rgba(255, 255, 255, 0.06); box-shadow: inset 0 0 1px rgba(255,255,255,0.04);">
            {/* 顶部操作栏 */}
            <Show
              when={props.isSelectingMessages}
              fallback={
                <div class="flex items-center justify-between px-1 pb-3 shrink-0">
                  <span class="text-sm font-medium truncate" style="color: rgba(255,255,255,0.7);">
                    {props.activeTopic?.name ?? ''}
                  </span>
                  <Show when={props.canShare && props.activeTopic}>
                    <button
                      class="group inline-flex items-center justify-center bg-transparent rounded-lg cursor-pointer w-8 h-8 hover:w-[68px] transition-all duration-200 hover:px-2 hover:bg-[rgba(255,255,255,0.06)]"
                      style="border: 1px solid rgba(255,255,255,0.12); color: rgba(255,255,255,0.45);"
                      onClick={() => props.onOpenShare()}
                    >
                      <Icon src="/icons/app-logo/share.svg" class="w-[15px] h-[15px] shrink-0" />
                      <span class="overflow-hidden whitespace-nowrap text-[11px] max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[80px] group-hover:opacity-100 group-hover:ml-1">分享</span>
                    </button>
                  </Show>
                </div>
              }
            >
              {/* 选择模式工具栏 */}
              <div class="flex items-center justify-between px-1 pb-3 shrink-0 animate-fade-in">
                <div class="flex items-center gap-2">
                  <button
                    class="px-3 py-1.5 rounded-lg text-xs font-medium transition-all duration-200 bg-white/[0.04] border border-white/[0.08] text-white/60 hover:bg-white/[0.08] hover:text-white/80"
                    onClick={props.onSelectAll}
                  >
                    全选
                  </button>
                  <button
                    class="px-3 py-1.5 rounded-lg text-xs font-medium transition-all duration-200 bg-white/[0.04] border border-white/[0.08] text-white/60 hover:bg-white/[0.08] hover:text-white/80"
                    onClick={props.onCancelSelection}
                  >
                    取消
                  </button>
                </div>
                <div class="flex items-center gap-3">
                  <span class="text-xs text-white/50">
                    已选 {props.selectedMessageIds.size} 条
                  </span>
                  <button
                    class="px-4 py-1.5 rounded-lg text-xs font-semibold transition-all duration-200"
                    classList={{
                      'bg-[rgba(124,154,191,0.12)] border border-[rgba(124,154,191,0.3)] text-[rgba(124,154,191,0.9)] hover:bg-[rgba(124,154,191,0.2)]': props.selectedMessageIds.size > 0,
                      'bg-white/[0.02] border border-white/[0.04] text-white/25 cursor-not-allowed': props.selectedMessageIds.size === 0,
                    }}
                    disabled={props.selectedMessageIds.size === 0}
                    onClick={props.onConfirmSelection}
                  >
                    确认
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
                <Show when={isStartingLocalModel()}>
                    <div class="w-full mb-4 p-4 rounded-lg" style="background: rgba(255,255,255,0.04); border: 1px solid rgba(255,255,255,0.06);">
                        <div class="flex items-center gap-3 mb-2">
                            <span style="color: rgba(124,154,191,0.6);"><Icon src="/icons/app-logo/loading.svg" class="w-5 h-5 animate-spin" /></span>
                            <span style="color: rgba(255,255,255,0.85); font-size: 0.875rem;">正在启动本地推理引擎...</span>
                        </div>
                        <div class="w-full h-2 rounded-full" style="background: rgba(255,255,255,0.06);">
                            <div class="h-2 rounded-full transition-all duration-300" style={{ width: `${localModelStartProgress()}%`, background: 'rgba(124,154,191,0.4)' }}></div>
                        </div>
                        <div class="text-right text-xs mt-1" style="color: rgba(124,154,191,0.4);">
                            {Math.round(localModelStartProgress())}%
                        </div>
                    </div>
                </Show>

                <Show when={!props.activeTopic || (props.activeTopic?.history?.length ?? 0) === 0}>
                    <div class="min-h-full flex items-center justify-center">
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
                            const isStreaming = createMemo(() => index() === props.typingIndex && props.isThinking && !msg.content);
                            const isActiveRound = createMemo(() => index() === props.typingIndex && props.isThinking);
                            const fileChanges = (msg.role === 'assistant') ? aggregateFileChanges(msg) : [];
                            return (
                                <div class="flex items-start gap-2 mb-3">
                                    {/* 选择模式：复选框 */}
                                    <Show when={props.isSelectingMessages && msg.id}>
                                      <div
                                        class="flex-shrink-0 mt-1 cursor-pointer select-none"
                                        onClick={() => props.onToggleMessage(msg.id!)}
                                      >
                                        <div
                                          class="w-5 h-5 rounded border-2 flex items-center justify-center transition-all duration-150 hover:scale-110 active:scale-95"
                                          classList={{
                                            'bg-[rgba(124,154,191,0.25)] border-[rgba(124,154,191,0.5)]': props.selectedMessageIds.has(msg.id!),
                                            'bg-transparent border-white/[0.15] hover:border-white/[0.35]': !props.selectedMessageIds.has(msg.id!),
                                          }}
                                        >
                                          <Show when={props.selectedMessageIds.has(msg.id!)}>
                                            <Icon name="check" class="w-3 h-3" style="color: rgba(124,154,191,0.9);" />
                                          </Show>
                                        </div>
                                      </div>
                                    </Show>
                                    <div
                                      ref={(el) => { if (msg.id) { animatedMessageIds.add(msg.id); messageEls.set(msg.id, el); } }}
                                      class={`flex flex-col flex-1 pointer-events-auto min-w-0 ${msg.id && !animatedMessageIds.has(msg.id) && !skipMessageAnimation() ? 'animate-message-in' : ''} ${msg.role === 'assistant' ? 'items-start' : 'items-end'}`}
                                    >
                                <div class={`flex gap-3 w-full ${msg.role === 'assistant' ? 'justify-start items-start' : 'justify-end items-start'}`}>
                                    <Show when={msg.role === 'assistant'}>
                                        <div class="flex flex-shrink-0 items-center justify-center w-9 h-9 rounded-full overflow-hidden"
                                             style="background: #ffffff; border: 1px solid rgba(255,255,255,0.85); box-shadow: 0 2px 6px rgba(0,0,0,0.25);">
                                            <Show
                                                when={getModelLogo(msg.modelId || selectedModel()?.model_id || '')}
                                                fallback={
                                                    <span class="text-[14px] font-bold select-none" style="color: #1a1e2c;">
                                                        {(msg.modelId || selectedModel()?.model_id || 'AI').charAt(0).toUpperCase()}
                                                    </span>
                                                }
                                            >
                                                <img
                                                    src={getModelLogo(msg.modelId || selectedModel()?.model_id || "")!}
                                                    alt="AI"
                                                    class="w-[25px] h-[25px] rounded-full"
                                                />
                                            </Show>
                                        </div>
                                    </Show>

                                    <div class={`flex flex-col max-w-[75%] ${msg.role === 'assistant' ? 'items-start' : 'items-end'}`}>
                                        <div
                                            class={`rounded-[10px] leading-relaxed max-w-full min-h-[1.5em] px-[14px] py-[10px] transition-[height] duration-200 break-words group relative ${
                                                msg.role === 'assistant'
                                                    ? 'rounded-tl-[2px] text-white'
                                                    : 'rounded-tr-[2px] text-white'
                                            } ${isActiveRound() && msg.content ? 'message-streaming' : ''}`}
                                            style={`background: ${msg.role === 'assistant' ? 'rgba(255,255,255,0.04)' : 'rgba(124,154,191,0.08)'}; border: ${msg.role === 'assistant' ? '1px solid rgba(255,255,255,0.04)' : '1px solid rgba(124,154,191,0.06)'}; backdrop-filter: blur(8px);`}
                                        >
                                            <Show when={msg.role === 'user' && msg.displayFiles && msg.displayFiles.length > 0}>
                                                <For each={msg.displayFiles}>
                                                    {(file: any) => (
                                                        <div class="flex items-center rounded-lg cursor-default mb-2 max-w-[300px] px-[14px] py-[10px] transition-all duration-200 first:mt-3"
                                                             style="background: rgba(124,154,191,0.06); border: 1px solid rgba(124,154,191,0.04);">
                                                            <div class="flex flex-shrink-0 items-center justify-center w-10 h-10 rounded-md mr-3"
                                                                 style="background: rgba(124,154,191,0.08); color: rgba(124,154,191,0.6);">
                                                                <Icon src="/icons/app-logo/file-document.svg" class="w-6 h-6" />
                                                            </div>
                                                            <div class="flex-grow overflow-hidden">
                                                                <div class="text-white text-[0.9rem] font-medium overflow-hidden text-ellipsis whitespace-nowrap">{file.name}</div>
                                                                <div style="color: rgba(124,154,191,0.4); font-size: 0.75rem; margin-top: 2px;">已解析</div>
                                                            </div>
                                                        </div>
                                                    )}
                                                </For>
                                            </Show>

                                            <div class="mt-1">
                                                {/* Agent 工作过程：reasoning + toolCalls + agentSteps 统一折叠 */}
                                                <Show when={msg.role === 'assistant' && (msg.reasoning || msg.interimContent || (msg as any).toolCalls?.length > 0 || (msg as any).agentSteps?.length > 0)}>
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
                                                    when={msg.role === 'assistant' && !msg.content && !(msg as any).toolCalls?.length && !msg.reasoning}
                                                    fallback={
                                                        <Show when={msg.role !== 'tool'}>
                                                            <div class={isActiveRound() && msg.content ? 'animate-stream-fade-in' : ''}>
                                                                <Markdown content={msg.role === 'user' && msg.displayText !== undefined ? msg.displayText : msg.content} />
                                                            </div>
                                                        </Show>
                                                    }
                                                >
                                                    <div class="flex items-center gap-2 py-1 text-white/50 italic text-[14px] select-none">
                                                        <Icon src="/icons/app-logo/loading.svg" class="w-4 h-4 animate-spin opacity-50" />
                                                        <span class="animate-pulse">AI 正在思考中...</span>
                                                    </div>
                                                </Show>
                                            </div>
                                        </div>

                                        <Show when={msg.role === 'assistant' && (msg.modelId || selectedModel()?.model_id || msg.inputTokens || msg.outputTokens)}>
                                            <div style="color: rgba(255,255,255,0.3); font-family: monospace; font-size: 11px; margin-left: 4px; margin-top: 4px; opacity: 0.7; user-select: none; text-align: left; display: flex; align-items: center; gap: 8px;">
                                                <Show when={msg.modelId || selectedModel()?.model_id}>
                                                    <span>{msg.modelId || selectedModel()?.model_id}</span>
                                                </Show>
                                                <Show when={msg.inputTokens || msg.outputTokens}>
                                                    <span style="color: rgba(255,255,255,0.2);">·</span>
                                                    <Show when={msg.inputTokens}>
                                                        <span title="输入 tokens">↗ {(msg.inputTokens! >= 1000) ? `${(msg.inputTokens! / 1000).toFixed(1)}K` : msg.inputTokens}</span>
                                                    </Show>
                                                    <Show when={msg.outputTokens}>
                                                        <span title="输出 tokens">↘ {(msg.outputTokens! >= 1000) ? `${(msg.outputTokens! / 1000).toFixed(1)}K` : msg.outputTokens}</span>
                                                    </Show>
                                                </Show>
                                            </div>
                                        </Show>

                                        {/* 消息操作按钮组：始终可见 */}
                                        <div class={`flex mt-1.5 px-[4px] z-[5] gap-1.5 flex-wrap ${msg.role === 'assistant' ? 'justify-start' : 'justify-end'}`}>
                                            {/* 复制按钮（每条消息都有） */}
                                            <button
                                                class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(124,154,191,0.12)] hover:!p-[4px_8px]"
                                                style="border: 1px solid rgba(124,154,191,0.5); color: rgba(124,154,191,0.9);"
                                                onClick={(e) => {
                                                    const currentBtn = e.currentTarget;
                                                    const text = msg.role === 'user' && msg.displayText !== undefined ? msg.displayText : msg.content;
                                                    if (!text) return;
                                                    navigator.clipboard.writeText(text).then(() => {
                                                        const label = currentBtn.querySelector('.action-label') as HTMLElement | null;
                                                        if (label) {
                                                            const originalText = label.innerText;
                                                            currentBtn.style.color = '#4af908';
                                                            currentBtn.style.borderColor = '#4af908';
                                                            label.style.maxWidth = '60px';
                                                            label.style.opacity = '1';
                                                            label.innerText = '已复制';
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
                                                <Icon src="/icons/app-logo/clipboard-copy.svg" class="w-[13px] h-[13px]" />
                                                <span class="action-label overflow-hidden whitespace-nowrap text-[11px] max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[80px] group-hover:opacity-100">复制</span>
                                            </button>

                                            {/* 分支按钮（仅 assistant 消息） */}
                                            <Show when={msg.role === 'assistant' && !!msg.id}>
                                                <button
                                                    class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(124,154,191,0.12)] hover:!p-[4px_8px]"
                                                    style="border: 1px solid rgba(124,154,191,0.5); color: rgba(124,154,191,0.9);"
                                                    onClick={() => {
                                                        if (msg.id) props.onBranchFromMessage(msg.id);
                                                    }}
                                                    title="从此消息分叉出新话题"
                                                >
                                                    <Icon name="git-branch" size={13} style="display: inline; vertical-align: middle;" />
                                                    <span class="action-label overflow-hidden whitespace-nowrap text-[11px] max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[80px] group-hover:opacity-100">分支</span>
                                                </button>
                                            </Show>


                                            {/* 最后一条用户消息：重新发送 + 编辑后重发 */}
                                            <Show when={index() === lastUserMsgIndex() && msg.role === 'user' && (msg.content || msg.displayText)}>
                                                <button
                                                    class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(124,154,191,0.12)] hover:!p-[4px_8px]"
                                                    style="border: 1px solid rgba(124,154,191,0.5); color: rgba(124,154,191,0.9);"
                                                    onClick={async () => {
                                                        const text = msg.displayText !== undefined ? msg.displayText : msg.content;
                                                        if (!text) return;
                                                        if (props.isThinking) {
                                                            props.handleStopGeneration();
                                                            await new Promise(r => setTimeout(r, 150));
                                                        }
                                                        const asstId = currentAssistantId();
                                                        const tId = currentTopicId();
                                                        if (asstId && tId) {
                                                            const reply = lastAssistantReply();
                                                            // 跳过退场动画，直接删除；同时抑制新消息的入场动画
                                                            setSkipMessageAnimation(true);
                                                            setTimeout(() => setSkipMessageAnimation(false), 100);
                                                            if (reply?.id) {
                                                                try {
                                                                    await invoke('delete_topic_message', { topicId: tId, messageId: reply.id });
                                                                } catch (e) {
                                                                    console.error('删除助手回复失败:', e);
                                                                }
                                                                setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === tId, 'history', h => h.filter((m: any) => m.id !== reply.id));
                                                            }
                                                            if (msg.id) {
                                                                try {
                                                                    await invoke('delete_topic_message', { topicId: tId, messageId: msg.id });
                                                                } catch (e) {
                                                                    console.error('删除消息失败:', e);
                                                                }
                                                                setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === tId, 'history', h => h.filter((m: any) => m.id !== msg.id));
                                                            }
                                                        }
                                                        props.setInputMessage(text);
                                                        setTimeout(() => props.handleSendMessage(), 0);
                                                    }}
                                                >
                                                    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="w-[13px] h-[13px]">
                                                        <path stroke-linecap="round" stroke-linejoin="round" d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0l3.181 3.183a8.25 8.25 0 0013.803-3.7M4.031 9.865a8.25 8.25 0 0113.803-3.7l3.181 3.182" />
                                                    </svg>
                                                    <span class="action-label overflow-hidden whitespace-nowrap text-[11px] max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[80px] group-hover:opacity-100">重新发送</span>
                                                </button>
                                                <button
                                                    class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(124,154,191,0.12)] hover:!p-[4px_8px]"
                                                    style="border: 1px solid rgba(124,154,191,0.5); color: rgba(124,154,191,0.9);"
                                                    onClick={async () => {
                                                        const text = msg.displayText !== undefined ? msg.displayText : msg.content;
                                                        if (!text) return;
                                                        if (props.isThinking) {
                                                            props.handleStopGeneration();
                                                            await new Promise(r => setTimeout(r, 150));
                                                        }
                                                        const asstId = currentAssistantId();
                                                        const tId = currentTopicId();
                                                        if (asstId && tId) {
                                                            const reply = lastAssistantReply();
                                                            // 跳过退场动画，直接删除；同时抑制新消息的入场动画
                                                            setSkipMessageAnimation(true);
                                                            setTimeout(() => setSkipMessageAnimation(false), 100);
                                                            if (reply?.id) {
                                                                try {
                                                                    await invoke('delete_topic_message', { topicId: tId, messageId: reply.id });
                                                                } catch (e) {
                                                                    console.error('删除助手回复失败:', e);
                                                                }
                                                                setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === tId, 'history', h => h.filter((m: any) => m.id !== reply.id));
                                                            }
                                                            if (msg.id) {
                                                                try {
                                                                    await invoke('delete_topic_message', { topicId: tId, messageId: msg.id });
                                                                } catch (e) {
                                                                    console.error('删除消息失败:', e);
                                                                }
                                                                setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === tId, 'history', h => h.filter((m: any) => m.id !== msg.id));
                                                            }
                                                        }
                                                        props.setInputMessage(text);
                                                        if (textareaRef) {
                                                            textareaRef.focus();
                                                            textareaRef.scrollIntoView({ behavior: 'smooth', block: 'center' });
                                                        }
                                                    }}
                                                >
                                                    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="w-[13px] h-[13px]">
                                                        <path stroke-linecap="round" stroke-linejoin="round" d="M16.862 4.487l1.687-1.688a1.875 1.875 0 112.652 2.652L10.582 16.07a4.5 4.5 0 01-1.897 1.13L6 18l.8-2.685a4.5 4.5 0 011.13-1.897l8.932-8.931zm0 0L19.5 7.125M18 14v4.75A2.25 2.25 0 0115.75 21H5.25A2.25 2.25 0 013 18.75V8.25A2.25 2.25 0 015.25 6H10" />
                                                    </svg>
                                                    <span class="action-label overflow-hidden whitespace-nowrap text-[11px] max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[80px] group-hover:opacity-100">编辑</span>
                                                </button>
                                            </Show>

                                            {/* 删除按钮（每条消息都有，置于最右侧） */}
                                            <button
                                                class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(248,113,113,0.1)] hover:!p-[4px_8px]"
                                                style="border: 1px solid rgba(248,113,113,0.35); color: rgba(248,113,113,0.75);"
                                                onClick={async () => {
                                                    const asstId = currentAssistantId();
                                                    const tId = currentTopicId();
                                                    if (!asstId || !tId || !msg.id) return;
                                                    // 直接在 DOM 上播退场动画，不触发响应式重渲染
                                                    const el = messageEls.get(msg.id);
                                                    if (el) el.classList.add('animate-message-out');
                                                    await new Promise(r => setTimeout(r, 260));
                                                    try {
                                                        await invoke('delete_topic_message', { topicId: tId, messageId: msg.id });
                                                    } catch (e) {
                                                        console.error('删除消息失败:', e);
                                                    }
                                                    setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === tId, 'history', h => h.filter((m: any) => m.id !== msg.id));
                                                }}
                                            >
                                                <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="w-[13px] h-[13px]">
                                                    <path stroke-linecap="round" stroke-linejoin="round" d="M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 01-2.244 2.077H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0" />
                                                </svg>
                                                <span class="action-label overflow-hidden whitespace-nowrap text-[11px] max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[60px] group-hover:opacity-100">删除</span>
                                            </button>
                                        </div>
                                    </div>

                                    <Show when={msg.role === 'user'}>
                                        <UserMessageAvatar />
                                    </Show>
                                </div>

                                {/* File Changes box（仅 assistant 消息，有文件变更时显示）*/}
                                <Show when={fileChanges.length > 0}>
                                    {(() => {
                                        const isReverted = () => revertedMessages().has(msg.id ?? String(index()));
                                        const diffStats = () => aggregateDiffStats(fileChanges);
                                        const showUndo = () => !isReverted() && !!currentProject()?.path && !!gitBranch();
                                        const handleUndoAll = async () => {
                                            const projectPath = currentProject()?.path;
                                            if (!projectPath) return;
                                            try {
                                                await invoke('revert_file_changes_batch', {
                                                    projectPath,
                                                    filePaths: fileChanges.map((fc) => fc.filePath),
                                                });
                                                setRevertedMessages(prev => {
                                                    const next = new Set(prev);
                                                    next.add(msg.id ?? String(index()));
                                                    return next;
                                                });
                                            } catch (err) {
                                                console.error('批量撤销失败:', err);
                                                alert('批量撤销失败，请确认项目是否为 Git 仓库且文件未被提交。');
                                            }
                                        };
                                        return (
                                            <div
                                                class="rounded-lg overflow-hidden transition-opacity duration-300 w-[80%] self-center mt-3"
                                                classList={{ 'opacity-40': isReverted() }}
                                                style={{
                                                    border: '1px solid rgba(255,255,255,0.06)',
                                                    background: 'rgba(255,255,255,0.015)',
                                                }}
                                            >
                                                <div
                                                    class="flex items-center gap-2 px-3 py-1.5"
                                                    style={{
                                                        'border-bottom': '1px solid rgba(255,255,255,0.04)',
                                                        color: 'rgba(255,255,255,0.4)',
                                                        'font-size': '14px',
                                                    }}
                                                >
                                                    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="w-4 h-4" style="opacity: 0.6;">
                                                        <path stroke-linecap="round" stroke-linejoin="round" d="M19.5 14.25v-2.625a3.375 3.375 0 00-3.375-3.375h-1.5A1.125 1.125 0 0113.5 7.125v-1.5a3.375 3.375 0 00-3.375-3.375H8.25m0 12.75h7.5m-7.5 3H12M10.5 2.25H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 00-9-9z" />
                                                    </svg>
                                                    <Show
                                                        when={isReverted()}
                                                        fallback={<span class="font-medium">文件更改</span>}
                                                    >
                                                        <span class="font-medium" style="text-decoration: line-through;">文件更改</span>
                                                    </Show>
                                                    <span style="color: rgba(255,255,255,0.25);">{fileChanges.length} 个文件</span>
                                                    <Show when={diffStats().added > 0 || diffStats().deleted > 0}>
                                                        <span class="font-mono">
                                                            {diffStats().added > 0 && <span style="color: #50dc64;">+{diffStats().added}</span>}
                                                            {diffStats().added > 0 && diffStats().deleted > 0 && <span style="color: rgba(255,255,255,0.18);"> </span>}
                                                            {diffStats().deleted > 0 && <span style="color: #ff5050;">-{diffStats().deleted}</span>}
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
                                                            已撤销
                                                        </span>
                                                    </Show>
                                                    <Show when={showUndo()}>
                                                        <button
                                                            type="button"
                                                            class="flex items-center gap-1 px-1.5 py-0.5 rounded text-[12px] font-medium transition-colors hover:bg-white/[0.06] border-none cursor-pointer"
                                                            style="color: rgba(255,255,255,0.35); background: transparent;"
                                                            onClick={(e) => { e.stopPropagation(); handleUndoAll(); }}
                                                            title="撤销此轮所有文件更改"
                                                        >
                                                            <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="2" stroke="currentColor" class="w-3.5 h-3.5" style="opacity: 0.6;">
                                                                <path stroke-linecap="round" stroke-linejoin="round" d="M9 15L3 9m0 0l6-6M3 9h12a6 6 0 010 12h-3" />
                                                            </svg>
                                                            撤销全部
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
                </Show>

                {/* 工具调用审批气泡 */}
                <Show when={props.pendingApprovals.length > 0}>
                    <div class="mb-3 space-y-2">
                        <For each={props.pendingApprovals}>
                            {(approval) => (
                                <ToolApprovalBubble
                                    approval={approval}
                                    onResolved={props.onResolveApproval}
                                />
                            )}
                        </For>
                    </div>
                </Show>
            </div>

            {/* 滚动到底部按钮：用户上滚浏览历史后显示，点击回到最新消息 */}
            <Show when={!autoScroll()}>
                <button
                    class="absolute bottom-[130px] right-[30px] z-[50] flex items-center justify-center w-9 h-9 rounded-full cursor-pointer
                           animate-fade-in transition-all duration-200 hover:scale-110 active:scale-95"
                    style="background: rgba(124,154,191,0.15); border: 1px solid rgba(124,154,191,0.25); color: rgba(124,154,191,0.7); box-shadow: 0 2px 8px rgba(0,0,0,0.3);"
                    onClick={() => {
                        userScrolledUp = false;
                        setAutoScroll(true);
                        smoothScrollToBottom();
                    }}
                    title="滚动到最新消息"
                >
                    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="2" stroke="currentColor" class="w-4 h-4">
                        <path stroke-linecap="round" stroke-linejoin="round" d="M19.5 8.25l-7.5 7.5-7.5-7.5" />
                    </svg>
                </button>
            </Show>

            <Show when={props.isProcessing}>
                <div class="absolute inset-0 flex items-center justify-center z-[100]"
                     style="background: rgba(0,0,0,0.6); color: rgba(124,154,191,0.6); font-size: 0.875rem; backdrop-filter: blur(4px);">
                    正在解析文件内容...
                </div>
            </Show>

            <div class="flex flex-wrap gap-[3px] bg-transparent pt-[3px] relative z-0">
                <For each={props.pendingFiles}>
                    {(file, i) => (
                        <div class="flex items-center rounded-[16px] text-[12px] px-[10px] py-1 transition-all duration-200"
                             style="background: rgba(124,154,191,0.08); border: 1px solid rgba(124,154,191,0.04); color: rgba(124,154,191,0.6);">
                            <Show when={file.type === 'image'} fallback={<span class="mr-1 inline-flex"><Icon name="file" size={12} class="opacity-60" /></span>}>
                                <img src={file.previewUrl} class="w-5 h-5 object-cover mr-1 rounded-[2px]" />
                            </Show>
                            {file.name}
                            <button
                                class="flex items-center bg-none border-none text-[rgba(255,255,255,0.5)] cursor-pointer text-lg leading-none ml-2 transition-colors duration-200 hover:text-[#ff4d4d]"
                                onClick={() => {
                                    const duplicatePending = props.pendingFiles.some(
                                        (pending, index) => index !== i() && pending.id === file.id
                                    );
                                    if (!duplicatePending) {
                                        void invoke('discard_chat_attachment', { attachmentId: file.id }).catch(console.error);
                                    }
                                    props.setPendingFiles(p => p.filter((_, idx) => idx !== i()));
                                }}
                            >
                                ×
                            </button>
                        </div>
                    )}
                </For>
            </div>

            <div class="bg-transparent flex flex-col relative w-full z-10">
                {/* Agent 模式状态栏 */}
                <Show when={(() => {
                  const asst = datas.assistants.find(a => a.id === currentAssistantId());
                  return !isChatMode();
                })()}>
                  {(() => {
                    const asst = datas.assistants.find((a: any) => a.id === currentAssistantId());
                    const mode = asst?.agentMode || 'off';
                    const project = currentProject();
                    const modeLabel: string = { normal: '普通', auto: '自动', plan: 'Plan', workflow: '工作流' }[mode as 'normal'|'auto'|'plan'|'workflow'] || mode;
                    return (
                      <div class="flex items-center gap-2 px-3 h-8 rounded-2xl text-xs"
                           style="background: rgba(124,154,191,0.12); border: 1px solid rgba(124,154,191,0.2);">
                        <Icon name="wrench" size={14} />
                        <span style="color: rgba(255,255,255,0.7);">
                          Agent · {modeLabel} · 项目: {project?.name ?? ''}
                        </span>
                        <Show when={gitBranch()}>
                          <div class="branch-dropdown-container relative inline-block">
                            <button
                              type="button"
                              class="flex items-center gap-0.5 cursor-pointer border-none rounded-md px-1.5 py-0.5 transition-colors duration-150 bg-[rgba(124,154,191,0.08)]"
                              style="color: rgba(255,255,255,0.5); font-weight: 400;"
                              classList={{ '!bg-[rgba(124,154,191,0.18)]': branchOpen() }}
                              onClick={(e) => { e.stopPropagation(); setBranchOpen(!branchOpen()); }}
                              onMouseEnter={(e) => { if (!branchOpen()) e.currentTarget.style.background = 'rgba(124,154,191,0.15)'; }}
                              onMouseLeave={(e) => { if (!branchOpen()) e.currentTarget.style.background = 'rgba(124,154,191,0.08)'; }}
                            >
                              · <Icon name="git-branch" size={12} style="display: inline; vertical-align: middle;" /> {gitBranch()}
                            </button>
                            <Show when={branchOpen()}>
                              <div
                                ref={(el) => requestAnimationFrame(() => { el.classList.remove('opacity-0', 'scale-95'); el.classList.add('opacity-100', 'scale-100'); })}
                                class="absolute bottom-full left-0 mb-2 z-[41] w-[200px] rounded-xl overflow-hidden opacity-0 scale-95 transition-all duration-200 ease-out origin-bottom"
                                style="background: rgba(18,22,35,0.96); border: 1px solid rgba(255,255,255,0.08); backdrop-filter: blur(12px); box-shadow: 0 -8px 30px rgba(0,0,0,0.4);"
                                onClick={(e) => e.stopPropagation()}
                              >
                                <div class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest"
                                     style="color: rgba(255,255,255,0.35); background: rgba(255,255,255,0.04); border-bottom: 1px solid rgba(255,255,255,0.04);">
                                  切换分支
                                </div>
                                <div class="py-1 max-h-[200px] overflow-y-auto">
                                  <For each={gitBranches()}>
                                    {(branch) => (
                                      <button
                                        type="button"
                                        class="w-full flex items-center gap-2 px-3 py-2 text-left transition-colors cursor-pointer border-none text-[13px]"
                                        style="color: rgba(255,255,255,0.75);"
                                        classList={{ '!bg-[rgba(124,154,191,0.12)]': branch === gitBranch() }}
                                        onClick={async () => {
                                          if (branch === gitBranch()) { setBranchOpen(false); return; }
                                          try {
                                            await switchBranch(branch);
                                            setBranchOpen(false);
                                          } catch (e) {
                                            const msg = typeof e === 'string' ? e : '切换分支失败，请检查是否有未提交的更改';
                                            setBranchToast(msg);
                                            setTimeout(() => setBranchToast(null), 3000);
                                          }
                                        }}
                                        onMouseEnter={(e) => {
                                          if (branch !== gitBranch()) e.currentTarget.style.background = 'rgba(255,255,255,0.05)';
                                        }}
                                        onMouseLeave={(e) => {
                                          if (branch !== gitBranch()) e.currentTarget.style.background = 'transparent';
                                        }}
                                      >
                                        <span class="flex-1 truncate">{branch}</span>
                                        <Show when={branch === gitBranch()}>
                                          <Icon name="check" size={13} class="shrink-0" style="color: rgba(124,154,191,0.8);" />
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
                          <span class="w-2 h-2 rounded-full shrink-0" style={{ background: (() => { const s = mcpServerStatus()['__aio-filesystem__']?.status; return s === 'connected' ? '#4ade80' : s === 'connecting' ? '#facc15' : '#f87171'; })() }} title={(() => { const s = mcpServerStatus()['__aio-filesystem__']?.status; return s === 'connected' ? '文件系统正常' : s === 'connecting' ? '文件系统启动中' : '文件系统异常'; })()} />
                        </Show>
                      </div>
                    );
                  })()}
                </Show>
                <div class="rounded-xl box-border flex flex-col gap-[10px] mt-[3px] p-[10px] transition-all duration-200 w-full"
                     style="background: rgba(0,0,0,0.25); border: 1px solid rgba(255,255,255,0.06);">
                    <textarea
                        ref={textareaRef}
                        class="bg-transparent border-none text-white font-inherit text-base leading-relaxed min-h-[40px] max-h-[20vh] outline-none overflow-y-hidden px-[5px] pb-[5px] resize-none w-full focus:overflow-y-auto"
                        placeholder="输入消息... (Ctrl + Enter 换行)"
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

                    <div class="flex items-center justify-between border-t pt-2" style="border-color: rgba(255,255,255,0.04);">
                        <div class="flex items-center gap-2">
                            <ModelSelector />
                            <Show when={!isChatMode()}>
                              <AgentModeSelector />
                            </Show>
                            <ReasoningButton />
                            <WebSearchButton />

                            <button
                                class="flex items-center justify-center bg-transparent border-none rounded-md cursor-pointer p-1.5 transition-all duration-200"
                                style="color: rgba(255,255,255,0.4);"
                                title="上传文件"
                                onClick={async () => {
                                    const selected = await open({ multiple: true });
                                    if (!selected) return;
                                    const paths = Array.isArray(selected) ? selected : [selected];
                                    for (const path of paths) {
                                        await props.handleFileUpload(path, 'file');
                                    }
                                }}
                                onMouseEnter={(e) => { e.currentTarget.style.background = 'rgba(255,255,255,0.06)'; e.currentTarget.style.color = 'rgba(124,154,191,0.6)'; }}
                                onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = 'rgba(255,255,255,0.4)'; }}
                            >
                                <Icon src="/icons/app-logo/paperclip.svg" class="w-5 h-5" />
                            </button>

                            <button
                                class="flex items-center justify-center bg-transparent border-none rounded-md cursor-pointer p-1.5 transition-all duration-200"
                                style="color: rgba(255,255,255,0.4);"
                                title="上传图片"
                                onClick={async () => {
                                    const selected = await open({
                                        multiple: true,
                                        filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp'] }]
                                    });
                                    if (!selected) return;
                                    const paths = Array.isArray(selected) ? selected : [selected];
                                    for (const path of paths) {
                                        await props.handleFileUpload(path, 'image');
                                    }
                                }}
                                onMouseEnter={(e) => { e.currentTarget.style.background = 'rgba(255,255,255,0.06)'; e.currentTarget.style.color = 'rgba(124,154,191,0.6)'; }}
                                onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = 'rgba(255,255,255,0.4)'; }}
                            >
                                <Icon src="/icons/app-logo/image-photo.svg" class="w-5 h-5" />
                            </button>

                            {/* LSP 问题面板切换 */}
                            <Show when={hasDiagnostics()}>
                                <button
                                    class="relative flex items-center justify-center bg-transparent border-none rounded-md cursor-pointer p-1.5 transition-all duration-200"
                                    style="color: rgba(255,255,255,0.4);"
                                    title={`${totalErrors()} 错误, ${totalWarnings()} 警告 — 点击切换问题面板`}
                                    onClick={() => setProblemsPanelVisible(!problemsPanelVisible())}
                                    onMouseEnter={(e) => { e.currentTarget.style.background = 'rgba(255,255,255,0.06)'; e.currentTarget.style.color = 'rgba(255,255,255,0.6)'; }}
                                    onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = 'rgba(255,255,255,0.4)'; }}
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
                                class={`flex items-center justify-center border-none rounded-lg cursor-pointer h-8 w-8 transition-all duration-100 hover:opacity-90 hover:scale-105 active:scale-95 ${props.isThinking ? 'bg-[#ff4d4d] text-white' : 'text-white'}`}
                                style={!props.isThinking ? { background: 'rgba(124,154,191,0.3)' } : {}}
                                onClick={() => props.isThinking
                                    ? props.handleStopGeneration()
                                    : props.handleSendMessage()
                                }
                            >
                                <Show when={props.isThinking} fallback={
                                    <Icon src="/icons/app-logo/send.svg" class="w-[18px] h-[18px]" />
                                }>
                                    <Icon src="/icons/app-logo/stop-square.svg" class="w-[18px] h-[18px]" />
                                </Show>
                            </button>
                        </div>
                    </div>
                </div>
            </div>

            <Show when={props.isDragging}>
                <div class="absolute inset-0 flex items-center justify-center pointer-events-none z-[9999]"
                     style="background: rgba(0,0,0,0.5); backdrop-filter: blur(8px);">
                    <div class="relative flex flex-col items-center justify-center w-[420px] h-[280px] rounded-xl text-white text-center p-5"
                         style="background: rgba(18, 22, 35, 0.7); border: 1px solid rgba(255, 255, 255, 0.06);">
                        <div class="flex items-end mb-[25px] mt-[-30px]">
                            <div class="flex items-center justify-center w-[60px] h-20 rounded-md opacity-60 scale-[0.85] translate-y-[10px] -rotate-12 translate-x-[15px] z-[1]"
                                 style="background: rgba(124,154,191,0.06); border: 1px solid rgba(124,154,191,0.06);">
                                <span style="color: rgba(124,154,191,0.4);"><Icon src="/icons/app-logo/file-document.svg" class="w-6 h-6" /></span>
                            </div>
                            <div class="flex items-center justify-center w-[70px] h-[90px] rounded-md z-[3]"
                                 style="background: rgba(124,154,191,0.08); border: 1px solid rgba(124,154,191,0.1); color: rgba(124,154,191,0.5);">
                                <Icon src="/icons/app-logo/upload-arrow.svg" class="w-8 h-8" />
                            </div>
                            <div class="flex items-center justify-center w-[60px] h-20 rounded-md opacity-60 scale-[0.85] translate-y-[10px] rotate-12 -translate-x-[15px] z-[1]"
                                 style="background: rgba(124,154,191,0.06); border: 1px solid rgba(124,154,191,0.04);">
                                <span style="color: rgba(124,154,191,0.4);"><Icon src="/icons/app-logo/file-blank.svg" class="w-6 h-6" /></span>
                            </div>
                        </div>
                        <h2 style="color: rgba(124,154,191,0.6); font-size: 22px; letter-spacing: 0.1em; margin-bottom: 10px; z-index: 2;">上传文件</h2>
                        <p style="color: rgba(255,255,255,0.5); font-size: 0.875rem; max-width: 80%; z-index: 2;">支持 PDF、Docx、pptx 和图片解析</p>
                        <div class="absolute inset-3 rounded-lg pointer-events-none" style="border: 1px dashed rgba(255,255,255,0.1);"></div>
                    </div>
                </div>
            </Show>

            {/* Git 分支切换失败 Toast — Portal 到 body 避免被父容器包含块限制 */}
            <Show when={branchToast()}>
              <Portal>
                <div
                  class="fixed bottom-5 left-5 z-[9999] max-w-[25vw] rounded-xl px-[18px] py-[10px] text-white text-[13px] font-medium shadow-[0_8px_32px_rgba(0,0,0,0.45)] select-none cursor-pointer"
                  style="color: #fca5a5; background: rgba(18, 22, 35, 0.92); border: 1px solid rgba(248,113,113,0.2); backdrop-filter: blur(30px) saturate(180%); animation: toastIn 0.3s cubic-bezier(0.175, 0.885, 0.32, 1.275) forwards;"
                  onClick={() => setBranchToast(null)}
                >{branchToast()}</div>
              </Portal>
            </Show>
        </div>
    );
};

export default ChatInterface;
