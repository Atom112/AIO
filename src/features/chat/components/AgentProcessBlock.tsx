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
import { Component, createSignal, Show, For, onCleanup, createEffect } from 'solid-js';
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

    return (
        <div
            class="agent-step-card"
            classList={{
                'is-running': props.step.status === 'running',
                'is-error': props.step.status === 'error',
                'is-expanded': props.expanded,
            }}
            style={{ '--step-border-color': getStepBorderColor(props.step) }}
        >
            {/* Header */}
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
            <div class="agent-step-body">
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
                            <For each={timelineSteps()}>
                                {(step) => (
                                    <StepCard
                                        step={step}
                                        isActive={props.isActive}
                                        expanded={isStepExpanded(step)}
                                        onToggle={() => toggleStep(step.id)}
                                    />
                                )}
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
