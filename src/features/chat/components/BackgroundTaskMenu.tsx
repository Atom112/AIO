/**
 * Background-task pull-up menu (fire-and-forget delegates).
 *
 * Mirrors SubagentMenu but lists only delegate_task(wait=false) subagents
 * (AgentStep.subagentBackground): tasks the main agent started in the background
 * and no longer waits on — they may outlive the parent turn. Rows open the
 * subagent's standalone conversation view; running rows carry an individual stop
 * button (stop_subagent cancels exactly these fire-and-forget handles; awaited
 * subagents stop together with the whole round instead). Rendered left of the
 * subagent menu in the status row; the ChatInterface right-group wrapper owns
 * the ml-auto alignment for both.
 */
import { For, Show, createMemo, createSignal, onCleanup, onMount, type Component } from 'solid-js';
import { t } from '../../../core/i18n';
import { collectSubagents, setActiveSubagentView, type Topic } from '../../../core/store/store';
import { stopSubagent } from '../../../core/api/chat';
import { btnGhost } from '../../../shared/components/buttonStyles';
import Icon from '../../../shared/components/Icon';
import { formatSubagentDuration, subagentProfileLabel } from './SubagentConversationView';
import { getSubagentProfileStyle } from '../../../core/utils/subagentProfileStyle';
import type { AgentStep } from '../../../core/store/types';

export interface BackgroundTaskMenuProps {
  /** Active topic (the background-task collection source). */
  activeTopic: () => Topic | null | undefined;
}

const BackgroundTaskMenu: Component<BackgroundTaskMenuProps> = (props) => {
  const [menuOpen, setMenuOpen] = createSignal(false);
  const [stoppingId, setStoppingId] = createSignal<string | null>(null);
  const tasks = createMemo(() =>
    collectSubagents(props.activeTopic()).filter((s) => s.subagentBackground === true),
  );
  const hasRunningTask = () => tasks().some((s) => s.status === 'running');

  // Stop a running background task individually (idempotent no-op otherwise)
  const handleStop = async (sub: AgentStep) => {
    const id = sub.subagentId;
    if (!id || stoppingId()) return;
    setStoppingId(id);
    try {
      await stopSubagent(id);
    } finally {
      setStoppingId(null);
    }
  };

  // Close the pull-up menu on outside click (the container class is the boundary)
  const closeMenu = (e: MouseEvent) => {
    const target = e.target as HTMLElement;
    if (menuOpen() && !target.closest('.background-task-menu-container')) {
      setMenuOpen(false);
    }
  };
  onMount(() => document.addEventListener('mousedown', closeMenu));
  onCleanup(() => document.removeEventListener('mousedown', closeMenu));

  return (
    <Show when={tasks().length > 0}>
      <div class="background-task-menu-container relative inline-block">
        <button
          type="button"
          class={
            'px-1.5 py-0.5 rounded-md text-[11px] bg-[rgba(var(--surface-alt-bg),0.5)] ' + btnGhost
          }
          style={{
            color: 'rgba(var(--text-base-rgb),0.55)',
            'font-weight': '400',
          }}
          classList={{
            '!bg-[rgba(var(--surface-bg),0.7)]': menuOpen(),
            'hover:bg-[rgba(var(--surface-bg),0.6)]': !menuOpen(),
          }}
          title={t('agent.background.menuTrigger', { count: tasks().length })}
          aria-label={t('agent.background.menuTrigger', { count: tasks().length })}
          aria-haspopup="menu"
          aria-expanded={menuOpen()}
          onClick={(e) => {
            e.stopPropagation();
            setMenuOpen(!menuOpen());
          }}
        >
          {t('agent.background.menuTitle')}
          <Show when={hasRunningTask()}>
            <Icon name="spinner" size={11} class="animate-spin mx-0.5" />
          </Show>
          {` ${t('agent.background.menuCount', { count: tasks().length })} `}
          <Icon name="chevron-up" size={10} class="ml-0.5" />
        </button>
        <Show when={menuOpen()}>
          <div
            ref={(el) =>
              requestAnimationFrame(() => {
                el.classList.remove('opacity-0', 'scale-95');
                el.classList.add('opacity-100', 'scale-100');
              })
            }
            class="absolute bottom-full right-0 mb-2 z-[41] w-[320px] rounded-xl overflow-hidden opacity-0 scale-95 transition-all duration-200 ease-out origin-bottom"
            style={{
              background: 'var(--popup-bg)',
              border: '1px solid var(--border-dim)',
              'backdrop-filter': 'var(--popup-blur)',
              'box-shadow': '0 -8px 30px rgba(0,0,0,0.4)',
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <div
              class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest"
              style={{
                color: 'rgba(var(--text-base-rgb),0.35)',
                background: 'rgba(var(--text-base-rgb),0.04)',
                'border-bottom': '1px solid var(--border-dim)',
              }}
            >
              {t('agent.background.menuTitle')}
            </div>
            <div class="py-1 max-h-[280px] overflow-y-auto">
              <For each={tasks()}>
                {(sub) => {
                  const ps = getSubagentProfileStyle(sub.subagentProfile);
                  return (
                    <div
                      class="w-full flex items-center gap-2 px-3 py-2 text-left transition-colors cursor-pointer hover:bg-[rgba(var(--text-base-rgb),0.05)]"
                      style={{ color: 'rgba(var(--text-base-rgb),0.75)' }}
                      onClick={() => {
                        if (sub.subagentId) setActiveSubagentView(sub.subagentId);
                        setMenuOpen(false);
                      }}
                    >
                      <span
                        class="relative flex items-center justify-center w-5 h-5 rounded shrink-0"
                        style={{
                          background: ps.bgColor,
                          border: `1px solid ${ps.borderColor}`,
                          color: ps.color,
                          animation:
                            sub.status === 'running'
                              ? 'subagentPulse 1.6s ease-in-out infinite'
                              : 'none',
                        }}
                      >
                        <Icon name={ps.icon} size={11} />
                        <Show when={sub.status === 'running'}>
                          <span class="subagent-work-orbit" style={{ color: ps.color }} />
                        </Show>
                      </span>
                      <span class="flex items-center justify-center w-3.5 h-3.5 shrink-0">
                        <Show
                          when={sub.status === 'running'}
                          fallback={
                            <Show
                              when={sub.status === 'error'}
                              fallback={
                                <Icon name="check" size={12} class="text-white/30 shrink-0" />
                              }
                            >
                              <Icon name="x" size={12} class="text-red-400/70 shrink-0" />
                            </Show>
                          }
                        >
                          <Icon name="spinner" size={12} class="animate-spin shrink-0" />
                        </Show>
                      </span>
                      <span class="flex-1 min-w-0 text-left">
                        <span class="flex items-center gap-1.5">
                          <span class="text-[12px] font-medium truncate">
                            {sub.subagentName || subagentProfileLabel(sub.subagentProfile)}
                          </span>
                          <span class="text-[10px] text-white/35 shrink-0">
                            {subagentProfileLabel(sub.subagentProfile)}
                          </span>
                        </span>
                        <Show when={sub.subagentTask}>
                          <span class="block truncate text-[11px] text-white/35">
                            {sub.subagentTask}
                          </span>
                        </Show>
                      </span>
                      <Show
                        when={sub.status === 'running' && sub.subagentId}
                        fallback={
                          <Show when={sub.status !== 'running' && sub.duration !== undefined}>
                            <span class="font-mono text-[10px] text-white/25 shrink-0">
                              {formatSubagentDuration(sub.duration!)}
                            </span>
                          </Show>
                        }
                      >
                        <button
                          type="button"
                          class="flex items-center justify-center w-5 h-5 shrink-0 rounded bg-transparent border-none cursor-pointer text-white/40 hover:text-red-400/80 transition-colors"
                          title={t('agent.background.stop')}
                          aria-label={t('agent.background.stop')}
                          onClick={(e) => {
                            e.stopPropagation();
                            void handleStop(sub);
                          }}
                        >
                          <Icon
                            name="stop"
                            size={11}
                            classList={{ 'animate-pulse': stoppingId() === sub.subagentId }}
                          />
                        </button>
                      </Show>
                    </div>
                  );
                }}
              </For>
            </div>
          </div>
        </Show>
      </div>
    </Show>
  );
};

export default BackgroundTaskMenu;
