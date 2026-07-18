import { Component, For, Show, Setter, createSignal, createEffect, createMemo, onCleanup, on } from 'solid-js';
import Markdown from '../../../shared/components/Markdown';
import AgentProcessBlock from './AgentProcessBlock';
import ModelSelector from './ModelSelector';
import { Topic, PendingAttachment, globalUserAvatar, selectedModel, isStartingLocalModel, localModelStartProgress, currentProjectId, currentProject, datas, setDatas, currentAssistantId, currentTopicId, isChatMode, mcpServerStatus, type AgentMode } from '../../../core/store/store';
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
}

const UserMessageAvatar: Component = () => {
    const [isLoaded, setIsLoaded] = createSignal(false);
    const [imgSrc, setImgSrc] = createSignal(globalUserAvatar());

    createEffect(() => {
        setImgSrc(globalUserAvatar());
        setIsLoaded(false);
    });

    return (
        <div class="relative flex flex-shrink-0 items-center justify-center w-9 h-9 rounded-full overflow-hidden"
             style="background: rgba(255,255,255,0.06); border: 1px solid rgba(255,255,255,0.04); box-shadow: 0 2px 6px rgba(0,0,0,0.15);">
            <img
                src={imgSrc()}
                alt="User"
                class="w-full h-full object-cover transition-opacity duration-300"
                classList={{ 'opacity-0': !isLoaded(), 'opacity-100': isLoaded() }}
                onLoad={() => setIsLoaded(true)}
                onError={() => setImgSrc('/icons/app-logo/user.svg')}
            />
            <div
                class="absolute inset-0 w-full h-full flex items-center justify-center transition-opacity duration-300 pointer-events-none"
                style="background: rgba(255,255,255,0.06);"
                classList={{ 'opacity-100': !isLoaded(), 'opacity-0': isLoaded() }}
            >
                <Icon src="/icons/app-logo/user.svg" class="w-5 h-5 opacity-50" />
            </div>
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
    // 流式 rAF 循环的最新 ID，始终指向最后一个排期的帧，保证能正确取消
    let streamRAFId: number | undefined;

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

    /** 当前助手的工作模式，用于控制工作目录选择器显隐 */
    const currentAgentMode = (): AgentMode => {
        const id = currentAssistantId();
        if (!id) return 'off';
        const asst = datas.assistants.find(a => a.id === id) as any;
        return asst?.agentMode || 'off';
    };

    return (
        <div class="flex flex-col flex-grow items-stretch rounded-lg box-border overflow-hidden p-[15px] pb-5 relative h-full"
             style="background: rgba(18, 22, 35, 0.12); backdrop-filter: blur(20px); border: 1px solid rgba(255, 255, 255, 0.06); box-shadow: inset 0 0 1px rgba(255,255,255,0.04);">
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
                            return (
                                <div
                                    ref={(el) => { if (msg.id) animatedMessageIds.add(msg.id); }}
                                    class={`flex flex-col mb-3 pointer-events-auto ${msg.id && !animatedMessageIds.has(msg.id) ? 'animate-message-in' : ''} ${msg.role === 'assistant' ? 'items-start' : 'items-end'}`}
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
                                                class="msg-action-btn inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200"
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
                                                <span class="action-label">复制</span>
                                            </button>

                                            {/* 最后一条用户消息：重新发送 + 编辑后重发 */}
                                            <Show when={index() === lastUserMsgIndex() && msg.role === 'user' && (msg.content || msg.displayText)}>
                                                <button
                                                    class="msg-action-btn inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200"
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
                                                        if (asstId && tId && msg.id) {
                                                            try {
                                                                await invoke('delete_topic_message', { topicId: tId, messageId: msg.id });
                                                            } catch (e) {
                                                                console.error('删除消息失败:', e);
                                                            }
                                                            setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === tId, 'history', h => h.filter((m: any) => m.id !== msg.id));
                                                        }
                                                        props.setInputMessage(text);
                                                        setTimeout(() => props.handleSendMessage(), 0);
                                                    }}
                                                >
                                                    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="w-[13px] h-[13px]">
                                                        <path stroke-linecap="round" stroke-linejoin="round" d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0l3.181 3.183a8.25 8.25 0 0013.803-3.7M4.031 9.865a8.25 8.25 0 0113.803-3.7l3.181 3.182" />
                                                    </svg>
                                                    <span class="action-label">重新发送</span>
                                                </button>
                                                <button
                                                    class="msg-action-btn inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200"
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
                                                        if (asstId && tId && msg.id) {
                                                            try {
                                                                await invoke('delete_topic_message', { topicId: tId, messageId: msg.id });
                                                            } catch (e) {
                                                                console.error('删除消息失败:', e);
                                                            }
                                                            setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === tId, 'history', h => h.filter((m: any) => m.id !== msg.id));
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
                                                    <span class="action-label">编辑</span>
                                                </button>
                                            </Show>
                                        </div>
                                    </div>

                                    <Show when={msg.role === 'user'}>
                                        <UserMessageAvatar />
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
                                        void invoke('discard_chat_attachment', { attachmentId: file.id });
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
        </div>
    );
};

export default ChatInterface;
