/**
 * Subagent pull-up menu (阶段4 第2项拆分切片：ChatInterface <900 目标).
 *
 * Verbatim extraction of the subagent menu from ChatInterface's status bar: the
 * trigger button (count + running spinner) and the dropdown listing every subagent
 * (status icon, profile style, name/task, duration). Owns the open state and the
 * outside-click-to-close handler; the parent passes only the active topic
 * (subagent collection source). Selection navigates via the module-level
 * setActiveSubagentView store signal.
 */
import { For, Show, createMemo, createSignal, onCleanup, onMount, type Component } from 'solid-js';
import { t } from '../../../core/i18n';
import { collectSubagents, setActiveSubagentView, type Topic } from '../../../core/store/store';
import { btnGhost } from '../../../shared/components/buttonStyles';
import Icon from '../../../shared/components/Icon';
import {
  formatSubagentDuration,
  getSubagentProfileStyle,
  subagentProfileLabel,
} from './SubagentConversationView';

export interface SubagentMenuProps {
  /** Active topic (the subagent collection source). */
  activeTopic: () => Topic | null | undefined;
}

const SubagentMenu: Component<SubagentMenuProps> = (props) => {
  const [subagentMenuOpen, setSubagentMenuOpen] = createSignal(false);
  const subagents = createMemo(() => collectSubagents(props.activeTopic()));
  const hasRunningSubagent = () => subagents().some((s) => s.status === 'running');

  // Close the pull-up menu on outside click (the container class is the boundary)
  const closeMenu = (e: MouseEvent) => {
    const target = e.target as HTMLElement;
    if (subagentMenuOpen() && !target.closest('.subagent-menu-container')) {
      setSubagentMenuOpen(false);
    }
  };
  onMount(() => document.addEventListener('mousedown', closeMenu));
  onCleanup(() => document.removeEventListener('mousedown', closeMenu));

  return (
    <Show when={subagents().length > 0}>
      <div class="subagent-menu-container relative inline-block ml-auto">
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
            '!bg-[rgba(var(--surface-bg),0.7)]': subagentMenuOpen(),
            'hover:bg-[rgba(var(--surface-bg),0.6)]': !subagentMenuOpen(),
          }}
          title={t('agent.subagent.menuTrigger', { count: subagents().length })}
          aria-label={t('agent.subagent.menuTrigger', { count: subagents().length })}
          aria-haspopup="menu"
          aria-expanded={subagentMenuOpen()}
          onClick={(e) => {
            e.stopPropagation();
            setSubagentMenuOpen(!subagentMenuOpen());
          }}
        >
          {t('agent.subagent.menuTitle')}
          <Show when={hasRunningSubagent()}>
            <Icon name="spinner" size={11} class="animate-spin mx-0.5" />
          </Show>
          {` ${t('agent.subagent.menuCount', { count: subagents().length })} `}
          <Icon name="chevron-up" size={10} class="ml-0.5" />
        </button>
        <Show when={subagentMenuOpen()}>
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
              {t('agent.subagent.menuTitle')}
            </div>
            <div class="py-1 max-h-[280px] overflow-y-auto">
              <For each={subagents()}>
                {(sub) => {
                  const ps = getSubagentProfileStyle(sub.subagentProfile);
                  return (
                    <button
                      type="button"
                      class="w-full flex items-center gap-2 px-3 py-2 text-left transition-colors cursor-pointer border-none bg-transparent hover:bg-[rgba(var(--text-base-rgb),0.05)]"
                      style={{ color: 'rgba(var(--text-base-rgb),0.75)' }}
                      onClick={() => {
                        if (sub.subagentId) setActiveSubagentView(sub.subagentId);
                        setSubagentMenuOpen(false);
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
                      <Show when={sub.status !== 'running' && sub.duration !== undefined}>
                        <span class="font-mono text-[10px] text-white/25 shrink-0">
                          {formatSubagentDuration(sub.duration!)}
                        </span>
                      </Show>
                    </button>
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

export default SubagentMenu;
