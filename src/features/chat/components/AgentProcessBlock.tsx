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
};

function getStepBorderColor(step: AgentStep): string {
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
    const name = step.toolCall?.function?.name || 'unknown';
    // 截断过长的工具名
    return name.length > 28 ? name.slice(0, 26) + '…' : name;
}

function stepStatusIcon(step: AgentStep): string {
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
                class="agent-step-header"
                onClick={props.onToggle}
                disabled={props.step.status === 'running'}
            >
                <span class="agent-step-icon-wrap">
                    <Icon
                        name={cfg.icon}
                        size={12}
                        class="agent-step-icon"
                        classList={{ 'is-spinning': props.step.status === 'running' }}
                    />
                </span>
                <span class="agent-step-title">{cfg.label}</span>
                {props.step.type === 'tool_call' && (
                    <span class="agent-step-tool-name">· {stepToolLabel(props.step)}</span>
                )}
                {props.step.type === 'tool_call' && stepStatusIcon(props.step) && (
                    <span class="agent-step-status-icon">
                        <Icon
                            name={stepStatusIcon(props.step) as any}
                            size={10}
                            classList={{ 'animate-spin': props.step.status === 'running' }}
                        />
                    </span>
                )}
                {durationMs !== undefined && (
                    <span class="agent-step-duration">{formatDuration(durationMs)}</span>
                )}
                <span class="agent-step-chevron" aria-hidden="true">
                    <Icon name="arrow-left" size={10} class="agent-step-chevron-icon" />
                </span>
            </button>

            {/* Body */}
            <div class="agent-step-body">
                {/* 思考步骤：原始文本 */}
                {props.step.type === 'thinking' && props.step.thinkingText && (
                    <div class="agent-step-thinking-content">
                        {props.step.thinkingText}
                    </div>
                )}

                {/* 工具调用步骤：复用 ToolCallBubble */}
                {props.step.type === 'tool_call' && props.step.toolCall && (
                    <div class="agent-step-tool-content">
                        <ToolCallBubble
                            toolCall={props.step.toolCall}
                            state={props.step.toolCall.state ?? props.step.status}
                            result={props.step.toolCall.result}
                            error={props.step.toolCall.error}
                        />
                    </div>
                )}

                {/* Content 步骤：Markdown 渲染的阶段性总结 */}
                {props.step.type === 'content' && props.step.contentText && (
                    <div class="agent-step-content-text">
                        <Markdown content={props.step.contentText} />
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

    // 工作中强制展开整体，完成后允许用户控制
    const isExpanded = () => {
        if (props.isActive) return true;
        return userExpanded() ?? false;
    };

    // 实时计时器（整体耗时）
    const [elapsedMs, setElapsedMs] = createSignal(0);
    let rafId: number | null = null;

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
            setElapsedMs(Date.now() - props.startTime);
        } else {
            if (rafId) cancelAnimationFrame(rafId);
        }
    });

    onCleanup(() => { if (rafId) cancelAnimationFrame(rafId); });

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

    return (
        <Show when={hasContent()}>
            <div
                class="agent-process-block"
                classList={{ 'is-open': isExpanded(), 'is-active': props.isActive }}
            >
                {/* 外层 header */}
                <button
                    type="button"
                    class="agent-process-header"
                    onClick={toggleOuter}
                    aria-expanded={isExpanded()}
                >
                    <span class="agent-process-icon-wrap">
                        <Icon
                            name={props.isActive ? 'sparkles' : 'brain'}
                            size={13}
                            class="agent-process-icon"
                        />
                    </span>
                    <span class="agent-process-title">
                        {props.isActive ? '正在工作' : '已工作'}
                    </span>
                    <span class="agent-process-duration">
                        {props.startTime ? formatDuration(elapsedMs()) : ''}
                    </span>
                    <span class="agent-process-chevron" aria-hidden="true">
                        <Icon name="arrow-left" size={11} class="agent-process-chevron-icon" />
                    </span>
                </button>

                {/* Body */}
                <div class="agent-process-body">
                    {/* === 新模式：时间线卡片 === */}
                    <Show when={hasSteps()}>
                        <div class="agent-steps-timeline">
                            <For each={props.agentSteps}>
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
                            <div class="agent-process-reasoning">
                                <div class="agent-process-reasoning-label">
                                    <Icon name="brain" size={11} class="mr-1.5 opacity-60" />
                                    思考过程
                                </div>
                                <div class="agent-process-reasoning-content">
                                    {props.reasoning}
                                </div>
                            </div>
                        </Show>

                        <Show when={props.interimContent && props.interimContent.trim()}>
                            <div class="agent-process-interim">
                                <div class="agent-process-interim-label">
                                    <Icon name="document" size={11} class="mr-1.5 opacity-60" />
                                    工作过程
                                </div>
                                <div class="agent-process-interim-content">
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
        </Show>
    );
};

export default AgentProcessBlock;
