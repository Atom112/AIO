/**
 * WorkflowVisualization — 工作流步骤可视化管线
 *
 * 在工作流执行期间，于 TopicSidebar 上方渲染一条水平步骤卡片管线，
 * 实时显示各步骤的状态（pending/running/completed/failed）和耗时。
 *
 * 数据来源：store 中 workflowState 信号（由后端 Tauri 事件驱动更新）。
 */
import { Component, createSignal, Show, For } from 'solid-js';
import { workflowState, type WorkflowState, type WorkflowStepState } from '../../../core/store/store';
import Icon from '../../../shared/components/Icon';
import { t, type TranslationKey } from '../../../core/i18n';


/** 步骤状态 → Tailwind 圆点颜色 */
const STATUS_DOT: Record<string, string> = {
    pending:   'bg-gray-400',
    running:   'bg-blue-500 animate-pulse',
    completed: 'bg-green-500',
    failed:    'bg-red-500',
};

/** 步骤状态 → Tailwind 文字颜色 */
const STATUS_TEXT: Record<string, string> = {
    pending:   'text-gray-400',
    running:   'text-blue-400',
    completed: 'text-green-400',
    failed:    'text-red-400',
};

const STATUS_LABEL: Record<string, TranslationKey> = {
    pending:   'agent.workflow.pending',
    running:   'agent.workflow.running',
    completed: 'agent.status.completed',
    failed:    'agent.status.failed',
};

/** 毫秒 → "Xs" 格式（秒取整） */
function formatDuration(ms?: number): string {
    if (!ms || ms <= 0) return '';
    return `${Math.round(ms / 1000)}s`;
}

const WorkflowVisualization: Component = () => {
    const [collapsed, setCollapsed] = createSignal(false);

    return (
        <Show when={workflowState()}>
            {(ws) => (
                <div class="bg-gray-800/60 border-b border-gray-700 shrink-0">
                    {/* 标题栏 */}
                    <button
                        type="button"
                        class="flex items-center justify-between w-full px-3 py-1.5 text-left hover:bg-white/[0.03] transition-colors"
                        onClick={() => setCollapsed((p) => !p)}
                    >
                        <div class="flex items-center gap-2 text-xs text-gray-300 min-w-0">
                            <Icon name="sparkles" size={12} class="text-yellow-400 shrink-0" />
                            <span class="font-medium truncate">{ws().title}</span>
                            <span class="text-gray-600 shrink-0">
                                {ws().steps.filter((s: WorkflowStepState) => s.status === 'completed').length}/{ws().steps.length}
                            </span>
                        </div>
                        <Icon
                            name="arrow-left"
                            class={`text-gray-500 shrink-0 transition-transform duration-200 ${collapsed() ? '-rotate-90' : 'rotate-90'}`}
                        />
                    </button>

                    {/* 步骤卡片管线 */}
                    <Show when={!collapsed()}>
                        <div class="flex gap-2 px-3 pb-2 overflow-x-auto scrollbar-thin">
                            <For each={ws().steps}>
                                {(step, index) => (
                                    <div
                                        class="flex items-center gap-1.5 rounded-lg bg-gray-800/50 border border-gray-700/50 px-2 py-1 text-xs shrink-0 select-none"
                                        classList={{
                                            'opacity-50': step.status === 'pending',
                                        }}
                                    >
                                        {/* 步骤序号 */}
                                        <span class="text-gray-600 tabular-nums w-3 text-right">
                                            {index() + 1}
                                        </span>

                                        {/* 状态圆点 */}
                                        <span
                                            class={`w-2 h-2 rounded-full shrink-0 ${STATUS_DOT[step.status] || 'bg-gray-400'}`}
                                        />

                                        {/* 步骤名 */}
                                        <span class="text-gray-300 truncate max-w-[80px]">
                                            {step.name}
                                        </span>

                                        {/* 状态文字 */}
                                        <span
                                            class={`${STATUS_TEXT[step.status] || 'text-gray-400'} shrink-0`}
                                        >
                                            {STATUS_LABEL[step.status] ? t(STATUS_LABEL[step.status]) : step.status}
                                        </span>

                                        {/* 耗时 */}
                                        {step.duration != null && step.duration > 0 && (
                                            <span class="text-gray-600 shrink-0">
                                                {formatDuration(step.duration)}
                                            </span>
                                        )}
                                    </div>
                                )}
                            </For>
                        </div>
                    </Show>
                </div>
            )}
        </Show>
    );
};

export default WorkflowVisualization;
