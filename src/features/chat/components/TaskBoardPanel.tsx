/**
 * TaskBoardPanel - agent task list panel
 *
 * While the main agent works through a task list (create_tasks / update_task tools), this
 * collapsible panel above the chat area shows the board for the current topic with live
 * statuses. Data source: the agentTaskBoard signal (mirrored via agent-tasks-changed).
 * Mounting slot and visual language follow WorkflowVisualization.
 */
import { Component, createSignal, For, Show } from 'solid-js';
import { agentTaskBoard } from '../../../core/store/store';
import Icon from '../../../shared/components/Icon';
import { t } from '../../../core/i18n';

interface TaskBoardPanelProps {
  /** Current topic id; the panel only renders for the matching board. */
  topicId: string;
}

const TaskBoardPanel: Component<TaskBoardPanelProps> = (props) => {
  const [collapsed, setCollapsed] = createSignal(false);

  const board = () => {
    const b = agentTaskBoard();
    return b && b.topicId === props.topicId && b.tasks.length > 0 ? b : null;
  };

  const doneCount = () => board()!.tasks.filter((tk) => tk.status === 'completed').length;

  return (
    <Show when={board()}>
      <div class="bg-gray-800/60 border-b border-gray-700 shrink-0">
        {/* Header bar */}
        <button
          type="button"
          class="flex items-center justify-between w-full px-3 py-1.5 text-left hover:bg-white/[0.03] transition-colors"
          onClick={() => setCollapsed((p) => !p)}
        >
          <div class="flex items-center gap-2 text-xs text-gray-300 min-w-0">
            <Icon name="clipboard" size={12} class="text-yellow-400 shrink-0" />
            <span class="font-medium truncate">{t('agent.tasks.title')}</span>
            <span class="text-gray-600 shrink-0 tabular-nums">
              {doneCount()}/{board()!.tasks.length}
            </span>
          </div>
          <Icon
            name="arrow-left"
            class={`text-gray-500 shrink-0 transition-transform duration-200 ${collapsed() ? '-rotate-90' : 'rotate-90'}`}
          />
        </button>

        {/* Task rows */}
        <Show when={!collapsed()}>
          <div class="px-3 pb-2 space-y-1 max-h-[240px] overflow-y-auto scrollbar-thin">
            <For each={board()!.tasks}>
              {(task) => (
                <div
                  class="flex items-center gap-2 rounded-lg bg-gray-800/50 border border-gray-700/50 px-2 py-1 text-xs select-none"
                  classList={{ 'opacity-50': task.status === 'pending' }}
                >
                  {/* Status mark */}
                  <Show
                    when={task.status === 'completed'}
                    fallback={
                      <span
                        class="w-2 h-2 rounded-full shrink-0"
                        classList={{
                          'border border-gray-500': task.status === 'pending',
                          'bg-blue-500 animate-pulse': task.status === 'in_progress',
                        }}
                      />
                    }
                  >
                    <Icon name="check" size={12} class="text-green-500 shrink-0" />
                  </Show>
                  <span
                    class="text-gray-300 truncate"
                    classList={{ 'line-through text-gray-500': task.status === 'completed' }}
                  >
                    {task.title}
                  </span>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>
    </Show>
  );
};

export default TaskBoardPanel;
