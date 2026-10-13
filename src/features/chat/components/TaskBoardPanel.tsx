/**
 * TaskBoardPanel - agent task list section inside the agent status bar.
 *
 * Round-11 redesign (user feedback): the board used to be a standalone gray-themed
 * panel above the message area, visually detached from the app theme. It now renders
 * as a section of the status bar strip (above the status row, inside the strip's
 * border): created tasks expand the strip to show the detailed rows; the collapse
 * button reduces it to a single summary line ("Tasks: N, done M, pending K").
 * Colors are CSS-variable based and follow the active theme.
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
  const pendingCount = () => board()!.tasks.filter((tk) => tk.status === 'pending').length;

  return (
    <Show when={board()}>
      <div class="border-b" style={{ 'border-color': 'rgba(var(--border-rgb), 0.15)' }}>
        {/* Summary / toggle row: collapsed state shows only this line.
            rounded-t matches the strip's rounded-2xl so the hover background
            follows the frame corner (the strip does not clip children). */}
        <button
          type="button"
          class="flex items-center gap-2 w-full px-3 py-1.5 text-left cursor-pointer rounded-t-2xl bg-transparent border-none hover:bg-[rgba(var(--text-base-rgb),0.04)] transition-colors"
          onClick={() => setCollapsed((p) => !p)}
          title={collapsed() ? t('agent.tasks.expand') : t('agent.tasks.collapse')}
        >
          <Icon
            name="clipboard"
            size={12}
            class="shrink-0"
            style={{ color: 'rgba(var(--primary-rgb),0.7)' }}
          />
          <span class="font-medium truncate" style={{ color: 'rgba(var(--text-base-rgb),0.7)' }}>
            {t('agent.tasks.statusLine', {
              total: board()!.tasks.length,
              done: doneCount(),
              pending: pendingCount(),
            })}
          </span>
          <Icon
            name="arrow-left"
            size={11}
            class="shrink-0 ml-auto transition-transform duration-200"
            classList={{ '-rotate-90': collapsed(), 'rotate-90': !collapsed() }}
            style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
          />
        </button>

        {/* Detailed task rows */}
        <Show when={!collapsed()}>
          <div class="px-2 pb-2 space-y-1 max-h-[200px] overflow-y-auto scrollbar-thin">
            <For each={board()!.tasks}>
              {(task) => (
                <div
                  class="flex items-center gap-2 rounded-md px-2 py-1 text-[11px] select-none"
                  style={{
                    background: 'rgba(var(--text-base-rgb),0.03)',
                    'border-left': `2px solid ${
                      task.status === 'in_progress'
                        ? 'rgba(var(--primary-rgb),0.55)'
                        : 'transparent'
                    }`,
                  }}
                  classList={{ 'opacity-55': task.status === 'pending' }}
                >
                  {/* Status mark */}
                  <Show
                    when={task.status === 'completed'}
                    fallback={
                      <span
                        class="w-2 h-2 rounded-full shrink-0"
                        classList={{
                          'animate-pulse': task.status === 'in_progress',
                        }}
                        style={{
                          background:
                            task.status === 'in_progress'
                              ? 'rgba(var(--primary-rgb),0.8)'
                              : 'transparent',
                          border:
                            task.status === 'pending'
                              ? '1px solid rgba(var(--text-base-rgb),0.35)'
                              : 'none',
                        }}
                      />
                    }
                  >
                    <Icon
                      name="check"
                      size={12}
                      class="shrink-0"
                      style={{ color: 'var(--color-success)' }}
                    />
                  </Show>
                  <span
                    class="truncate"
                    classList={{ 'line-through': task.status === 'completed' }}
                    style={{
                      color:
                        task.status === 'completed'
                          ? 'rgba(var(--text-base-rgb),0.35)'
                          : 'rgba(var(--text-base-rgb),0.75)',
                    }}
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
