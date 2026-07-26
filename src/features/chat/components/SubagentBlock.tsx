/**
 * 子智能体工作过程展示块。
 *
 * 在主 Agent 的 AgentProcessBlock 时间线中，当步骤类型为 'subagent' 时，
 * 渲染此组件以展示子智能体的独立工作过程。
 *
 * 特性：
 * - Header 显示子 Agent 名称、类型标签、任务摘要、状态图标
 * - 内部时间线展示子 Agent 的工作步骤
 * - 支持折叠/展开
 * - 颜色区分不同 profile：explorer（蓝）、coder（绿）、general（紫）
 */
import { Component, createSignal, Show, For, onCleanup, createEffect } from 'solid-js';
import type { AgentStep, SubagentStep } from '../../../core/store/store';
import Icon from '../../../shared/components/Icon';
import { t } from '../../../core/i18n';

interface SubagentBlockProps {
    step: AgentStep;
    isActive: boolean;
    expanded: boolean;
    onToggle: () => void;
}

// ---- Profile 配置 ----
interface ProfileStyle {
    icon: string;
    color: string;
    bgColor: string;
    borderColor: string;
}

const PROFILE_STYLES: Record<string, ProfileStyle> = {
    explorer: {
        icon: 'search',
        color: 'rgba(100,149,237,0.85)',
        bgColor: 'rgba(100,149,237,0.08)',
        borderColor: 'rgba(100,149,237,0.25)',
    },
    coder: {
        icon: 'code',
        color: 'rgba(76,175,144,0.85)',
        bgColor: 'rgba(76,175,144,0.08)',
        borderColor: 'rgba(76,175,144,0.25)',
    },
    general: {
        icon: 'sparkles',
        color: 'rgba(186,104,200,0.85)',
        bgColor: 'rgba(186,104,200,0.08)',
        borderColor: 'rgba(186,104,200,0.25)',
    },
    requirements: {
        icon: 'lightbulb',
        color: 'rgba(255, 183, 77, 0.85)',
        bgColor: 'rgba(255, 183, 77, 0.08)',
        borderColor: 'rgba(255, 183, 77, 0.25)',
    },
};

const DEFAULT_PROFILE: ProfileStyle = {
    icon: 'sparkles',
    color: 'rgba(156,163,175,0.75)',
    bgColor: 'rgba(156,163,175,0.06)',
    borderColor: 'rgba(156,163,175,0.2)',
};

function profileLabel(profileId?: string): string {
    switch (profileId) {
        case 'explorer': return t('agent.profile.explorer');
        case 'coder': return t('agent.profile.coder');
        case 'general': return t('agent.profile.general');
        case 'requirements': return t('agent.profile.requirements');
        default: return t('agent.profile.subagent');
    }
}

function getProfileStyle(profileId?: string): ProfileStyle {
    return PROFILE_STYLES[profileId || ''] || DEFAULT_PROFILE;
}

function formatDuration(ms: number): string {
    if (ms < 50) return '0.1s';
    const seconds = ms / 1000;
    if (seconds < 60) return `${seconds.toFixed(1)}s`;
    const mins = Math.floor(seconds / 60);
    const secs = Math.floor(seconds % 60);
    return t('agent.step.duration', { mins, secs });
}

function stepTypeIcon(type: SubagentStep['type']): string {
    switch (type) {
        case 'thinking': return 'brain';
        case 'tool_call': return 'wrench';
        case 'content': return 'document';
        default: return 'document';
    }
}

function stepTypeLabel(type: SubagentStep['type']): string {
    switch (type) {
        case 'thinking': return t('agent.sub.step.thinking');
        case 'tool_call': return t('agent.sub.step.toolCall');
        case 'content': return t('agent.sub.step.content');
    }
}

// ---- 子 Agent 单步骤行 ----
const SubStepRow: Component<{ step: SubagentStep }> = (props) => {
    const icon = stepTypeIcon(props.step.type);
    const label = stepTypeLabel(props.step.type);
    const duration = props.step.duration
        ?? (props.step.status === 'running' ? Date.now() - props.step.timestamp : undefined);

    return (
        <div
            class="flex items-center gap-1.5 px-2 py-1 text-[11px]"
            classList={{
                'text-white/35': props.step.status === 'complete',
                'text-white/55': props.step.status === 'running',
                'text-red-400/60': props.step.status === 'error',
            }}
        >
            <span class="flex items-center justify-center w-3.5 h-3.5 shrink-0">
                <Icon
                    name={icon as any}
                    size={11}
                    classList={{ 'animate-spin': props.step.status === 'running' }}
                />
            </span>
            <span class="font-medium shrink-0">{label}</span>
            {props.step.toolName && (
                <span class="truncate text-white/40 min-w-0">· {props.step.toolName}</span>
            )}
            {props.step.summary && (
                <span class="truncate text-white/30 min-w-0 flex-1">— {props.step.summary}</span>
            )}
            {duration !== undefined && (
                <span class="font-mono text-white/20 text-[10px] ml-auto shrink-0">
                    {formatDuration(duration)}
                </span>
            )}
            {props.step.status === 'running' && (
                <span class="w-1.5 h-1.5 rounded-full bg-current animate-pulse shrink-0" />
            )}
        </div>
    );
};

// ---- 主组件 ----
const SubagentBlock: Component<SubagentBlockProps> = (props) => {
    const profileStyle = () => getProfileStyle(props.step.subagentProfile);
    const [elapsedMs, setElapsedMs] = createSignal(0);
    let rafId: number | null = null;

    createEffect(() => {
        if (props.isActive && props.step.status === 'running' && props.step.timestamp) {
            const tick = () => {
                if (props.step.status !== 'running') return;
                setElapsedMs(Date.now() - props.step.timestamp);
                rafId = requestAnimationFrame(tick);
            };
            rafId = requestAnimationFrame(tick);
        } else if (props.step.status !== 'running' && props.step.timestamp) {
            if (rafId) cancelAnimationFrame(rafId);
            setElapsedMs((props.step.duration ?? 0) || Date.now() - props.step.timestamp);
        } else {
            if (rafId) cancelAnimationFrame(rafId);
        }
    });

    onCleanup(() => { if (rafId) cancelAnimationFrame(rafId); });

    const style = profileStyle();
    const steps = () => props.step.subagentSteps || [];
    const displayName = props.step.subagentName || profileLabel(props.step.subagentProfile);
    const taskSummary = props.step.subagentTask || '';
    const resultText = props.step.subagentResult || '';

    return (
        <div
            class="subagent-block rounded-lg overflow-hidden border animate-expand-width"
            style={{
                '--sa-color': style.color,
                '--sa-bg': style.bgColor,
                '--sa-border': style.borderColor,
                background: style.bgColor,
                'border-color': style.borderColor,
            } as any}
            classList={{
                'opacity-90': props.step.status === 'complete',
            }}
        >
            {/* Header */}
            <button
                type="button"
                class="flex items-center w-full gap-2 px-3 py-2 cursor-pointer select-none bg-transparent border-none transition-colors duration-200 hover:bg-white/[0.03]"
                style="color: var(--sa-color); font-size: 12px;"
                onClick={props.onToggle}
            >
                <span
                    class="flex items-center justify-center w-5 h-5 rounded shrink-0"
                    style="background: rgba(255,255,255,0.04);"
                >
                    <Icon
                        name={style.icon as any}
                        size={12}
                        classList={{ 'animate-spin': props.step.status === 'running' }}
                    />
                </span>
                <span class="font-semibold shrink-0">{displayName}</span>
                <span
                    class="text-[10px] px-1.5 py-0.5 rounded font-medium shrink-0"
                    style={{
                        color: style.color,
                        background: 'rgba(255,255,255,0.05)',
                        'border': `1px solid ${style.borderColor}`,
                    }}
                >
                    {profileLabel(props.step.subagentProfile)}
                </span>
                {taskSummary && (
                    <span
                        class="truncate min-w-0 flex-1"
                        style="color: 'rgba(255,255,255,0.35)'; font-weight: 400;"
                    >
                        {taskSummary.length > 60 ? taskSummary.slice(0, 58) + '…' : taskSummary}
                    </span>
                )}
                <span class="font-mono text-white/25 text-[10px] ml-auto shrink-0">
                    {props.step.timestamp ? formatDuration(elapsedMs()) : ''}
                </span>
                <Show when={props.step.status === 'running'}>
                    <span class="w-1.5 h-1.5 rounded-full animate-pulse shrink-0" style="background: var(--sa-color);" />
                </Show>
                <Show when={props.step.status === 'error'}>
                    <Icon name="x" size={10} class="text-red-400/70 shrink-0" />
                </Show>
                <span class="flex items-center justify-center shrink-0 text-white/20">
                    <Icon
                        name="arrow-left"
                        size={10}
                        classList={{ '-rotate-90': !props.expanded, 'rotate-90': props.expanded }}
                        style="transition: transform 0.2s;"
                    />
                </span>
            </button>

            {/* Body */}
            <Show when={props.expanded}>
                <div
                    class="px-2 pb-2"
                    style="max-height: 400px; overflow-y: auto;"
                >
                    {/* 内部步骤时间线 */}
                    <Show when={steps().length > 0}>
                        <div class="flex flex-col gap-0.5 border-l-2 ml-2 pl-2" style="border-color: var(--sa-border);">
                            <For each={steps()}>
                                {(subStep) => <SubStepRow step={subStep} />}
                            </For>
                        </div>
                    </Show>

                    {/* 最终结果 */}
                    <Show when={resultText && props.step.status === 'complete'}>
                        <div
                            class="mt-2 px-2 py-2 rounded text-[12px] leading-relaxed whitespace-pre-wrap break-words"
                            style={{
                                background: 'rgba(0,0,0,0.12)',
                                color: 'rgba(255,255,255,0.55)',
                                'max-height': '300px',
                                'overflow-y': 'auto',
                            }}
                        >
                            {resultText.length > 2000
                                ? resultText.slice(0, 2000) + '\n\n' + t('agent.sub.resultTruncated')
                                : resultText}
                        </div>
                    </Show>

                    {/* 无步骤提示 */}
                    <Show when={steps().length === 0 && props.step.status === 'running'}>
                        <div class="text-[11px] text-white/25 italic px-2 py-2">
                            {t('agent.sub.working')}
                        </div>
                    </Show>

                    {/* 错误状态 */}
                    <Show when={props.step.status === 'error'}>
                        <div class="text-[12px] text-red-400/60 px-2 py-2">
                            {t('agent.sub.error')}
                        </div>
                    </Show>
                </div>
            </Show>
        </div>
    );
};

export default SubagentBlock;
