/**
 * WorkflowVisualization - workflow step visualization pipeline
 *
 * While a workflow runs, renders a horizontal step-card pipeline above TopicSidebar,
 * showing each step's status (pending/running/completed/failed) and duration in real time.
 *
 * Data source: the workflowState signal in the store (driven by backend Tauri events).
 */
import { Component, createSignal, Show, For } from 'solid-js';
import { workflowState, type WorkflowStepState } from '../../../core/store/store';
import Icon from '../../../shared/components/Icon';
import { t, type TranslationKey } from '../../../core/i18n';

/** Step status → Tailwind dot color */
const STATUS_DOT: Record<string, string> = {
  pending: 'bg-gray-400',
  running: 'bg-blue-500 animate-pulse',
  completed: 'bg-green-500',
  failed: 'bg-red-500',
  skipped: 'bg-gray-600',
};

/** Step status → Tailwind text color */
const STATUS_TEXT: Record<string, string> = {
  pending: 'text-gray-400',
  running: 'text-blue-400',
  completed: 'text-green-400',
  failed: 'text-red-400',
  skipped: 'text-gray-500',
};

const STATUS_LABEL: Record<string, TranslationKey> = {
  pending: 'agent.workflow.pending',
  running: 'agent.workflow.running',
  completed: 'agent.status.completed',
  failed: 'agent.status.failed',
  skipped: 'agent.workflow.skipped',
};

/** Milliseconds → "Xs" format (seconds rounded) */
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
          {/* Header bar */}
          <button
            type="button"
            class="flex items-center justify-between w-full px-3 py-1.5 text-left hover:bg-white/[0.03] transition-colors"
            onClick={() => setCollapsed((p) => !p)}
          >
            <div class="flex items-center gap-2 text-xs text-gray-300 min-w-0">
              <Icon name="sparkles" size={12} class="text-yellow-400 shrink-0" />
              <span class="font-medium truncate">{ws().title}</span>
              <span class="text-gray-600 shrink-0">
                {ws().steps.filter((s: WorkflowStepState) => s.status === 'completed').length}/
                {ws().steps.length}
              </span>
            </div>
            <Icon
              name="arrow-left"
              class={`text-gray-500 shrink-0 transition-transform duration-200 ${collapsed() ? '-rotate-90' : 'rotate-90'}`}
            />
          </button>

          {/* Step-card pipeline */}
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
                    {/* Step index */}
                    <span class="text-gray-600 tabular-nums w-3 text-right">{index() + 1}</span>

                    {/* Status dot */}
                    <span
                      class={`w-2 h-2 rounded-full shrink-0 ${STATUS_DOT[step.status] || 'bg-gray-400'}`}
                    />

                    {/* Step name */}
                    <span class="text-gray-300 truncate max-w-[80px]">{step.name}</span>

                    {/* Status text */}
                    <span class={`${STATUS_TEXT[step.status] || 'text-gray-400'} shrink-0`}>
                      {STATUS_LABEL[step.status] ? t(STATUS_LABEL[step.status]) : step.status}
                    </span>

                    {/* Duration */}
                    {step.duration != null && step.duration > 0 && (
                      <span class="text-gray-600 shrink-0">{formatDuration(step.duration)}</span>
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
