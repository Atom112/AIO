/**
 * Harness plan strip — the CLI agent's execution plan (ACP `plan` session
 * updates) rendered as a collapsible section under the status bar, following
 * the chat TaskBoardPanel's status-bar visual language.
 */
import { Component, createSignal, For, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import { currentHarnessTopicId, harnessPlan } from '../../../core/store/harness';
import { t } from '../../../core/i18n';

const HarnessPlanStrip: Component = () => {
  const [collapsed, setCollapsed] = createSignal(false);

  const entries = () => harnessPlan()[currentHarnessTopicId() ?? ''] ?? null;
  const doneCount = () =>
    entries()!.filter((e) => e.status === 'completed' || e.status === 'cancelled').length;
  const pendingCount = () => entries()!.filter((e) => e.status === 'pending').length;

  const statusDot = (status: string) =>
    status === 'completed'
      ? 'var(--color-success)'
      : status === 'in_progress'
        ? 'rgba(var(--primary-rgb),0.9)'
        : status === 'cancelled'
          ? 'rgba(var(--text-base-rgb),0.3)'
          : 'rgba(var(--text-base-rgb),0.35)';

  return (
    <Show when={entries() && entries()!.length > 0}>
      <div class="border-b shrink-0" style={{ 'border-color': 'rgba(var(--border-rgb), 0.15)' }}>
        {/* Summary / toggle row */}
        <button
          type="button"
          class="flex items-center gap-2 w-full px-3 py-1.5 text-left cursor-pointer bg-transparent border-none hover:bg-[rgba(var(--text-base-rgb),0.04)] transition-colors"
          onClick={() => setCollapsed((p) => !p)}
          title={collapsed() ? t('agent.tasks.expand') : t('agent.tasks.collapse')}
        >
          <Icon
            name="clipboard"
            size={12}
            class="shrink-0"
            style={{ color: 'rgba(var(--primary-rgb),0.7)' }}
          />
          <span
            class="font-medium truncate text-[11px]"
            style={{ color: 'rgba(var(--text-base-rgb),0.7)' }}
          >
            {t('agent.tasks.statusLine', {
              total: entries()!.length,
              done: doneCount(),
              pending: pendingCount(),
            })}
          </span>
          <Icon
            name="triangle-down"
            size={11}
            class="shrink-0 ml-auto transition-transform duration-200"
            classList={{ '-rotate-90': collapsed() }}
            style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
          />
        </button>

        {/* Plan rows (grid-rows height animation; rows stay mounted) */}
        <div
          class="grid transition-[grid-template-rows] duration-200 ease-out"
          classList={{ 'grid-rows-[0fr]': collapsed(), 'grid-rows-[1fr]': !collapsed() }}
        >
          <div class="overflow-hidden min-h-0">
            <div
              class="px-2 pb-2 space-y-1 max-h-[200px] overflow-y-auto scrollbar-thin transition-opacity duration-200"
              classList={{ 'opacity-0': collapsed(), 'opacity-100': !collapsed() }}
            >
              <For each={entries()}>
                {(entry) => (
                  <div
                    class="flex items-center gap-2 rounded-md px-2 py-1 text-[11px] select-none"
                    style={{
                      background: 'rgba(var(--text-base-rgb),0.035)',
                      'border-left': `2px solid ${statusDot(entry.status)}`,
                      color: 'rgba(var(--text-base-rgb),0.72)',
                    }}
                    classList={{ 'line-through opacity-55': entry.status === 'completed' }}
                  >
                    <span class="truncate flex-1">{entry.content}</span>
                    <Show when={entry.status === 'in_progress'}>
                      <Icon
                        name="spinner"
                        size={10}
                        class="animate-spin shrink-0"
                        style={{ color: 'rgba(var(--primary-rgb),0.8)' }}
                      />
                    </Show>
                  </div>
                )}
              </For>
            </div>
          </div>
        </div>
      </div>
    </Show>
  );
};

export default HarnessPlanStrip;
