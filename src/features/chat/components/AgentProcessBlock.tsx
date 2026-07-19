/**
 * Agent 工作过程折叠块
 *
 * 两种渲染模式：
 * - 新模式（有 agentSteps）：按时间线卡片展示每个步骤（思考 / 工具调用 / 阶段性总结）
 * - 旧模式（无 agentSteps）：兼容旧数据，显示 reasoning + interimContent + toolCalls 三块
 *
 * 交互：
 * - 工作中步骤：自动展开，左边框脉动
 * - 已完成步骤：默认折叠（仅显示 header），可单独点击展开
 * - 外层折叠按钮：整体收起 / 展开所有步骤
 */
import { Component, createSignal, createMemo, Show, For, onCleanup, createEffect, onMount } from 'solid-js';
import type { AgentStep, ToolCallDisplay } from '../../../core/store/store';
import Icon from '../../../shared/components/Icon';
import ToolCallBubble from './ToolCallBubble';
import Markdown from '../../../shared/components/Markdown';
import SubagentBlock from './SubagentBlock';

interface AgentProcessBlockProps {
    /** 模型原生思维链文本（旧模式） */
    reasoning?: string;
    /** 该轮次产生的工具调用列表（旧模式） */
    toolCalls?: ToolCallDisplay[];
    /** 中间轮次的阶段性总结文本累积（旧模式） */
    interimContent?: string;
    /** 是否仍在工作中 */
    isActive: boolean;
    /** Agent 轮次开始时间戳（ms） */
    startTime?: number;
    /** Agent 工作过程时间线（新模式） */
    agentSteps?: AgentStep[];
}

// ---- 步骤类型配置 ----
const STEP_CONFIG: Record<AgentStep['type'], { icon: string; label: string; borderColor: string }> = {
    thinking: { icon: 'brain', label: '思考过程', borderColor: 'rgba(124,154,191,0.55)' },
    tool_call: { icon: 'wrench', label: '工具调用', borderColor: 'rgba(240,160,64,0.55)' },
    content: { icon: 'document', label: '阶段性总结', borderColor: 'rgba(156,163,175,0.35)' },
    subagent: { icon: 'sparkles', label: '子智能体', borderColor: 'rgba(140,120,220,0.55)' },
};

// 已播放入场动画的步骤 ID 集合，避免 agentSteps 数组重建时重复触发动画
const animatedStepIds = new Set<string>();

// 缓冲池状态（按会话轮次 startTime 索引）。
// store 流式更新 agentSteps 时会产生新的消息对象引用，父组件 <For> 按引用比较会销毁并重建
// AgentProcessBlock —— 每个工具事件触发一次重挂载。因此渲染进度、待出队队列、出队定时器
// 都必须持久化在模块级，否则重挂载时队列状态丢失，新步骤会被立即渲染（缓冲区被旁路）。
interface RoundRenderQueue {
    /** 已出队（已渲染）的步骤 id */
    renderedIds: Set<string>;
    /** 等待出队的步骤 id（保持到达顺序） */
    pendingIds: string[];
    /** 出队定时器；非 null 表示队列正在逐个出队中 */
    dequeueTimer: number | null;
}
const perRoundQueue = new Map<string, RoundRenderQueue>();

// 出队通知订阅者（按轮次索引）：出队定时器存活期间组件可能已重挂载，
// 定时器通过此 Map 只通知当前活跃实例，避免向已销毁实例的信号写入。
const queueSubscribers = new Map<string, (id: string) => void>();

function getRoundQueue(roundKey: string): RoundRenderQueue {
    let q = perRoundQueue.get(roundKey);
    if (!q) {
        q = { renderedIds: new Set(), pendingIds: [], dequeueTimer: null };
        perRoundQueue.set(roundKey, q);
    }
    return q;
}

function getStepBorderColor(step: AgentStep): string {
    if (step.type === 'subagent') return STEP_CONFIG[step.type].borderColor;
    if (step.type !== 'tool_call') return STEP_CONFIG[step.type].borderColor;
    if (step.status === 'running') return 'rgba(240,160,64,0.55)';
    if (step.status === 'error') return 'rgba(224,85,85,0.55)';
    return 'rgba(76,175,144,0.55)';
}

function formatDuration(ms: number): string {
    if (ms < 50) return '0.1s';
    const seconds = ms / 1000;
    if (seconds < 60) return `${seconds.toFixed(1)}s`;
    const mins = Math.floor(seconds / 60);
    const secs = Math.floor(seconds % 60);
    return `${mins}分 ${secs}秒`;
}

function stepToolLabel(step: AgentStep): string {
    if (step.type === 'subagent') return step.subagentProfile || 'subagent';
    const name = step.toolCall?.function?.name || 'unknown';
    // 截断过长的工具名
    return name.length > 28 ? name.slice(0, 26) + '…' : name;
}

function stepStatusIcon(step: AgentStep): string {
    if (step.type === 'subagent') return step.status === 'running' ? 'spinner' : step.status === 'error' ? 'x' : 'check';
    if (step.type !== 'tool_call') return '';
    if (step.status === 'running') return 'spinner';
    if (step.status === 'error') return 'x';
    return 'check';
}

// ---- 单步骤卡片组件 ----
const StepCard: Component<{
    step: AgentStep;
    isActive: boolean;
    expanded: boolean;
    onToggle: () => void;
}> = (props) => {
    const cfg = STEP_CONFIG[props.step.type];
    const durationMs = props.step.duration ?? (props.step.status === 'running' ? Date.now() - props.step.timestamp : undefined);

    // 入场动画去重：仅首次渲染该步骤时应用动画类
    const isNew = !animatedStepIds.has(props.step.id);
    if (isNew) {
        animatedStepIds.add(props.step.id);
    }

    // 入场动画类首帧就渲染在元素上（animation backwards 从初始帧开始），
    // 动画结束后移除该类，让 is-running 的 stepCardPulse 脉冲动画恢复（animation 简写会互相覆盖）
    let entranceRef: HTMLDivElement | undefined;

    // 入场期间锁定展开状态：避免 running → complete 快速翻转时 body 高度塌缩与入场动画冲突，
    // 导致视觉上动画被"压扁推走"。动画结束后才交还真实 expanded 状态。
    const [entranceDone, setEntranceDone] = createSignal(!isNew);
    const effectiveExpanded = () => !entranceDone() || props.expanded;

    onMount(() => {
        if (!isNew || !entranceRef) return;
        // 入场动画总时长 ~0.9s（0.1s 延迟 + 0.8s expand-width），结束后移除类并解锁 expanded
        const timer = setTimeout(() => {
            setEntranceDone(true);
        }, 920);
        onCleanup(() => clearTimeout(timer));
    });

    return (
        <div
            ref={entranceRef}
            class="rounded-md overflow-hidden bg-white/[0.015] border border-white/[0.04] hover:bg-white/[0.03]"
            classList={{
                'is-running': props.step.status === 'running',
                'is-error': props.step.status === 'error',
                'is-expanded': effectiveExpanded(),
            }}
            style={{
                '--step-border-color': getStepBorderColor(props.step),
                'border-left': `2px solid var(--step-border-color, rgba(255, 255, 255, 0.12))`,
                transition: 'border-color 0.3s, background 0.2s',
                ...(props.step.status === 'running' ? { background: 'rgba(var(--primary-rgb), 0.03)', animation: 'stepCardPulse 2s ease-in-out infinite' } : {}),
                ...(props.step.status === 'error' ? { 'border-left-color': 'rgba(224, 85, 85, 0.5)' } : {}),
                ...(isNew && !entranceDone() ? { animation: 'stepSlideIn 0.6s cubic-bezier(0.22, 0.61, 0.36, 1) backwards, expand-width 0.8s cubic-bezier(0.25, 0.46, 0.45, 0.94) 0.1s backwards' } : {})
            }}
        >
            <button
                type="button"
                class="flex items-center w-full gap-1.5 px-2.5 py-1.5 cursor-pointer select-none bg-transparent border-none text-white/45 text-[11px] transition-colors duration-200 hover:text-white/70"
                onClick={props.onToggle}
                disabled={props.step.status === 'running'}
            >
                <span class="flex items-center justify-center w-4 h-4 rounded bg-white/[0.05] shrink-0">
                    <Icon
                        name={cfg.icon as any}
                        size={12}
                        class="w-3 h-3"
                        classList={{ 'is-spinning': props.step.status === 'running' }}
                    />
                </span>
                <span class="font-medium flex-none">{cfg.label}</span>
                {props.step.type === 'tool_call' && (
                    <span class="truncate text-white/55 flex-initial min-w-0">· {stepToolLabel(props.step)}</span>
                )}
                {props.step.type === 'tool_call' && stepStatusIcon(props.step) && (
                    <span class="flex items-center justify-center shrink-0">
                        <Icon
                            name={stepStatusIcon(props.step) as any}
                            size={10}
                            classList={{ 'animate-spin': props.step.status === 'running' }}
                        />
                    </span>
                )}
                {durationMs !== undefined && (
                    <span class="font-mono text-white/25 text-[10px] flex-none ml-auto">{formatDuration(durationMs)}</span>
                )}
                <span class="flex items-center justify-center shrink-0 text-white/20 transition-[transform,color] duration-200" aria-hidden="true">
                    <Icon name="arrow-left" size={10} class="-rotate-90 transition-transform duration-200" />
                </span>
            </button>

            {/* Body */}
            <div class="overflow-hidden opacity-0 max-h-0 px-2.5" style={(() => {
                const expanded = effectiveExpanded();
                const running = props.step.status === 'running';
                const open = expanded || running;
                return {
                    transition: 'max-height 0.25s cubic-bezier(0.4, 0, 0.2, 1), opacity 0.18s, padding 0.25s',
                    ...(open ? { 'max-height': '800px', opacity: 1, 'padding-top': '2px', 'padding-bottom': '8px' } : {})
                };
            })()}>
                {/* 思考步骤：原始文本 */}
                {props.step.type === 'thinking' && props.step.thinkingText && (
                    <div class="text-[12px] leading-relaxed italic whitespace-pre-wrap break-words text-white/50 px-2 py-1.5 rounded bg-black/[0.12] max-h-[300px] overflow-y-auto">
                        {props.step.thinkingText}
                    </div>
                )}

                {/* 工具调用步骤：复用 ToolCallBubble */}
                {props.step.type === 'tool_call' && props.step.toolCall && (
                    <div class="text-[12px]">
                        <ToolCallBubble
                            toolCall={props.step.toolCall}
                            state={(props.step.toolCall.state || (props.step.status === 'running' ? 'calling' : props.step.status === 'error' ? 'error' : 'success')) as 'calling' | 'success' | 'error'}
                            result={props.step.toolCall.result}
                            error={props.step.toolCall.error}
                        />
                    </div>
                )}

                {/* Content 步骤：Markdown 渲染的阶段性总结 */}
                {props.step.type === 'content' && props.step.contentText && (
                    <div class="text-[13px] leading-relaxed break-words text-white/65 p-1">
                        <Markdown content={props.step.contentText} />
                    </div>
                )}

                {/* 子智能体步骤：嵌套 SubagentBlock */}
                {props.step.type === 'subagent' && (
                    <div class="px-2 pb-2">
                        <SubagentBlock
                            step={props.step}
                            isActive={props.isActive}
                            expanded={props.expanded}
                            onToggle={props.onToggle}
                        />
                    </div>
                )}
            </div>
        </div>
    );
};

// ---- 主组件 ----
const AgentProcessBlock: Component<AgentProcessBlockProps> = (props) => {
    const [userExpanded, setUserExpanded] = createSignal<boolean | null>(null);
    const [expandedSteps, setExpandedSteps] = createSignal<Set<string>>(new Set());

    // ---- 时间线自动滚动（复用 ChatInterface 模式） ----
    let stepsContainerRef: HTMLDivElement | undefined;
    const [autoScrollSteps, setAutoScrollSteps] = createSignal(true);
    let userScrolledUpSteps = false;     // 纯变量：rAF 回调需同步感知
    let suppressScrollSteps = false;     // 纯变量：抑制 programmatic scroll 事件
    let streamScrollRafId: number | null = null;

    const isStepsAtBottom = () => {
        if (!stepsContainerRef) return true;
        const el = stepsContainerRef;
        return el.scrollHeight - el.scrollTop - el.clientHeight < 50;
    };

    const snapStepsToBottom = () => {
        if (!stepsContainerRef) return;
        suppressScrollSteps = true;
        stepsContainerRef.scrollTop = stepsContainerRef.scrollHeight;
    };

    const handleStepsWheel = (e: WheelEvent) => {
        if (e.deltaY < 0) {
            userScrolledUpSteps = true;
            if (streamScrollRafId) { cancelAnimationFrame(streamScrollRafId); streamScrollRafId = null; }
            suppressScrollSteps = false;
            setAutoScrollSteps(false);
        }
    };

    const handleStepsScroll = () => {
        if (suppressScrollSteps) { suppressScrollSteps = false; return; }
        const atBottom = isStepsAtBottom();
        setAutoScrollSteps(atBottom);
        if (atBottom) userScrolledUpSteps = false;
    };

    // 工作中强制展开整体，完成后允许用户控制
    const isExpanded = () => {
        if (props.isActive) return true;
        return userExpanded() ?? false;
    };

    // 实时计时器（整体耗时）
    const [elapsedMs, setElapsedMs] = createSignal(0);
    let rafId: number | null = null;

    // 从 agentSteps 数据推导冻结耗时（不受 remount 影响，避免 Date.now() 漂移）
    const frozenElapsed = (): number => {
        if (!props.startTime) return 0;
        const steps = props.agentSteps;
        if (steps && steps.length > 0) {
            const last = steps[steps.length - 1];
            if (last.status === 'complete' && last.duration !== undefined) {
                return last.timestamp + last.duration - props.startTime;
            }
            if (last.status !== 'running') {
                return last.timestamp - props.startTime;
            }
        }
        return 0;
    };

    createEffect(() => {
        if (props.isActive && props.startTime) {
            const tick = () => {
                if (!props.isActive) return;
                setElapsedMs(Date.now() - props.startTime!);
                rafId = requestAnimationFrame(tick);
            };
            rafId = requestAnimationFrame(tick);
        } else if (!props.isActive && props.startTime) {
            if (rafId) cancelAnimationFrame(rafId);
            setElapsedMs(frozenElapsed());
        } else {
            if (rafId) cancelAnimationFrame(rafId);
        }
    });

    onCleanup(() => { if (rafId) cancelAnimationFrame(rafId); if (streamScrollRafId) cancelAnimationFrame(streamScrollRafId); });

    // 流式滚动：工作中每帧贴底
    createEffect(() => {
        if (props.isActive) {
            const tick = () => {
                if (!props.isActive || userScrolledUpSteps || !autoScrollSteps()) return;
                snapStepsToBottom();
                streamScrollRafId = requestAnimationFrame(tick);
            };
            streamScrollRafId = requestAnimationFrame(tick);
        }
    });

    const toggleOuter = () => {
        if (props.isActive) return;
        setUserExpanded(p => !(p ?? false));
    };

    const toggleStep = (stepId: string) => {
        setExpandedSteps(prev => {
            const next = new Set(prev);
            if (next.has(stepId)) next.delete(stepId);
            else next.add(stepId);
            return next;
        });
    };

    const isStepExpanded = (step: AgentStep) => {
        if (step.status === 'running') return true;
        return expandedSteps().has(step.id);
    };

    // 判断使用哪种模式
    const hasSteps = () => !!(props.agentSteps && props.agentSteps.length > 0);
    const hasOldContent = () => !!(props.reasoning?.trim()) || !!(props.interimContent?.trim()) || !!(props.toolCalls && props.toolCalls.length > 0);
    const hasContent = () => hasSteps() || hasOldContent();

    // 时间线步骤：排除最后一个 content 步骤（它是最终回复，已在下方 Markdown 气泡中显示）
    const timelineSteps = () => {
        const steps = props.agentSteps;
        if (!steps || steps.length === 0) return [];
        const last = steps[steps.length - 1];
        if (last.type === 'content') return steps.slice(0, -1);
        return steps;
    };

    // ---- 缓冲池：agent 可能几乎同时创建 7-8 个工具调用步骤，直接渲染会导致动画互相推挤、视觉混乱。
    // 新步骤先进入队列，按固定间隔逐个出队渲染，形成有序的流式进入效果。
    //
    // 关键设计：store 流式更新 agentSteps 时产生新的消息对象引用，父组件 <For> 按引用比较会销毁
    // 并重建 AgentProcessBlock —— 每个工具事件都触发一次重挂载。因此队列三要素（renderedIds /
    // pendingIds / dequeueTimer）全部存放在模块级 perRoundQueue 中，重挂载后原样恢复：
    // 已渲染的直接恢复 visibleIds（避免"从头渲染闪现"），待出队的继续排队，进行中的定时器
    // 不被重挂载打断（onCleanup 不能清除它）。
    //
    // 注意：定时器回调存活期间组件可能已重挂载多次，因此 setVisibleIds 不直接捕获组件实例的
    // setter，而是记录到模块级 subscribers，由当前活跃实例注册/注销，定时器只通知最新实例。
    const DEQUEUE_INTERVAL = 150;
    const roundKey = String(props.startTime ?? 0);
    const queue = getRoundQueue(roundKey);

    const [visibleIds, setVisibleIds] = createSignal<string[]>([...queue.renderedIds]);

    const stepById = (id: string): AgentStep | undefined => timelineSteps().find(s => s.id === id);

    // 出队渲染的订阅者：重挂载后只有最新实例接收出队通知，旧实例的 DOM 已被 <For> 销毁。
    const subscriber = (id: string) => setVisibleIds(prev => prev.includes(id) ? prev : [...prev, id]);
    queueSubscribers.set(roundKey, subscriber);
    onCleanup(() => {
        // 仅当当前实例仍是订阅者时注销（重挂载后新实例已覆盖注册）
        if (queueSubscribers.get(roundKey) === subscriber) queueSubscribers.delete(roundKey);
    });

    const flushQueue = () => {
        if (queue.pendingIds.length === 0) { queue.dequeueTimer = null; return; }
        const id = queue.pendingIds.shift()!;
        queue.renderedIds.add(id);
        queueSubscribers.get(roundKey)?.(id);
        queue.dequeueTimer = window.setTimeout(flushQueue, DEQUEUE_INTERVAL);
    };

    // 监听源步骤变化：工作中新步骤进队列逐个渲染；非工作中直接全量同步
    createEffect(() => {
        const steps = timelineSteps();
        if (!props.isActive) {
            // 非工作中（历史数据 / 已完成）：直接全量渲染，清空队列并释放该轮次的队列状态
            if (queue.dequeueTimer) { clearTimeout(queue.dequeueTimer); queue.dequeueTimer = null; }
            queue.pendingIds = [];
            queue.renderedIds.clear();
            perRoundQueue.delete(roundKey);
            steps.forEach(s => queue.renderedIds.add(s.id));
            setVisibleIds(steps.map(s => s.id));
            return;
        }
        // 工作中：找出未渲染且未排队的新步骤 id 加入队列（重挂载恢复后 pendingIds 仍有效，
        // 不会因实例重置而重复入队或绕过排队直接渲染）。
        // 首个元素同样延迟一个出队间隔再渲染：入队即启动定时器，由定时器统一驱动出队节奏，
        // 保证所有元素（包括首个）的入场时机一致、动画完整。
        const newIds = steps
            .filter(s => !queue.renderedIds.has(s.id) && !queue.pendingIds.includes(s.id))
            .map(s => s.id);
        if (newIds.length > 0) {
            queue.pendingIds.push(...newIds);
            if (queue.dequeueTimer === null) {
                queue.dequeueTimer = window.setTimeout(flushQueue, DEQUEUE_INTERVAL);
            }
        }
    });

    return (
        <Show when={hasContent()}>
            <div
                class="my-2"
                classList={{ 'is-open': isExpanded(), 'is-active': props.isActive }}
            >
                {/* 外层 header */}
                <button
                    type="button"
                    class="flex items-center w-full gap-2 px-3 py-2 cursor-pointer select-none bg-transparent border-none text-white/50 text-xs transition-colors duration-200 hover:text-white/75"
                    onClick={toggleOuter}
                    aria-expanded={isExpanded()}
                >
                    <span class="font-medium flex-none">
                        {props.isActive ? '正在工作' : '已工作'}
                    </span>
                    <span class="font-mono text-white/30 text-[11px] flex-none ml-0.5">
                        {props.startTime ? formatDuration(elapsedMs()) : ''}
                    </span>
                    <span class="flex items-center justify-center ml-auto w-5 h-5 rounded-full bg-white/[0.08] text-white/40 transition-colors duration-200" aria-hidden="true">
                        <Icon
                            name="arrow-left"
                            size={16}
                            classList={{ '-rotate-90': !isExpanded(), 'rotate-90': isExpanded() }}
                            style="transition: transform 0.2s;"
                        />
                    </span>
                </button>

                {/* Body：滚动放在外层，内部正常排列不压缩 */}
                <div class="overflow-hidden transition-[max-height] duration-300 ease-in-out" style={isExpanded() ? 'max-height: 620px' : 'max-height: 0'}>
                <div
                    ref={stepsContainerRef}
                    class="overflow-y-auto"
                    style="max-height: 620px;"
                    onScroll={handleStepsScroll}
                    onWheel={handleStepsWheel}
                >
                    {/* === 新模式：时间线卡片 === */}
                    <Show when={hasSteps()}>
                        <div class="flex flex-col gap-2 px-3">
                            <For each={visibleIds()}>
                                {(id) => {
                                    // 用 memo 从源数据取最新步骤对象：状态更新（running→complete）实时反映，
                                    // 但 <For> 的 each 是稳定的 id 列表，DOM 不会因状态更新而重建
                                    const step = createMemo(() => stepById(id));
                                    return (
                                        <Show when={step()}>
                                            <StepCard
                                                step={step()!}
                                                isActive={props.isActive}
                                                expanded={isStepExpanded(step()!)}
                                                onToggle={() => toggleStep(id)}
                                            />
                                        </Show>
                                    );
                                }}
                            </For>
                        </div>
                    </Show>

                    {/* === 旧模式（回退）：扁平三块 === */}
                    <Show when={!hasSteps() && hasOldContent()}>
                        <Show when={props.reasoning && props.reasoning.trim()}>
                            <div class="mb-2">
                                <div class="flex items-center text-[11px] font-medium mb-1 text-white/40">
                                    <Icon name="brain" size={11} class="mr-1.5 opacity-60" />
                                    思考过程
                                </div>
                                <div class="text-[12.5px] leading-relaxed italic whitespace-pre-wrap break-words text-white/55 px-2.5 py-2 rounded-md bg-black/15 border-l-2 border-l-white/[0.08] max-h-[360px] overflow-y-auto">
                                    {props.reasoning}
                                </div>
                            </div>
                        </Show>

                        <Show when={props.interimContent && props.interimContent.trim()}>
                            <div class="mb-2">
                                <div class="flex items-center text-[11px] font-medium mb-1 text-white/40">
                                    <Icon name="document" size={11} class="mr-1.5 opacity-60" />
                                    工作过程
                                </div>
                                <div class="text-[12.5px] leading-relaxed italic whitespace-pre-wrap break-words text-white/55 px-2.5 py-2 rounded-md bg-black/15 border-l-2 border-l-white/[0.08] max-h-[360px] overflow-y-auto">
                                    <Markdown content={props.interimContent!} />
                                </div>
                            </div>
                        </Show>

                        <Show when={props.toolCalls && props.toolCalls.length > 0}>
                            <div class="agent-process-toolcalls">
                                <For each={props.toolCalls}>
                                    {(tc) => (
                                        <ToolCallBubble
                                            toolCall={tc}
                                            state={tc.state ?? 'calling'}
                                            result={tc.result}
                                            error={tc.error}
                                        />
                                    )}
                                </For>
                            </div>
                        </Show>
                    </Show>
                </div>
                </div>
            </div>
        </Show>
    );
};

export default AgentProcessBlock;
