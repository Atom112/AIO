/**
 * Background-task pull-up menu: live view of the app's running background jobs
 * (memory fact extraction / consolidation, index rebuilds, code indexing, model
 * downloads), fed by the global registry (core/store/background-tasks) which
 * backgroundTaskListeners keep current. Rows are informational — the jobs are
 * controlled from their own surfaces (memory panel, model settings). Terminal
 * entries expire on their own, so the menu hides again once everything settles.
 *
 * Rendered LEFT of the subagent menu in the status row; the ChatInterface
 * right-group wrapper owns the ml-auto alignment for both.
 */
import { For, Show, createMemo, createSignal, onCleanup, onMount, type Component } from 'solid-js';
import { t } from '../../../core/i18n';
import { projects, type Topic } from '../../../core/store/store';
import {
  backgroundTasks,
  type BackgroundTask,
  type BackgroundTaskKind,
} from '../../../core/store/background-tasks';
import { btnGhost } from '../../../shared/components/buttonStyles';
import Icon from '../../../shared/components/Icon';

export interface BackgroundTaskMenuProps {
  /** Active topic (presence marker for status-row siblings; the registry is global). */
  activeTopic: () => Topic | null | undefined;
}

/** Kinds whose scope is a project root path (resolved to a display name). */
const PROJECT_KINDS: ReadonlySet<BackgroundTaskKind> = new Set([
  'memory-extract',
  'memory-consolidate',
  'reindex',
  'code-index',
]);

const KIND_ICON: Record<BackgroundTaskKind, Parameters<typeof Icon>[0]['name']> = {
  'memory-extract': 'brain',
  'memory-consolidate': 'layers',
  reindex: 'rotate-ccw',
  'code-index': 'code',
  'model-download': 'download',
  'gguf-download': 'download',
};

/** Download progress counters are bytes; index counters are item counts. */
const isDownloadKind = (k: BackgroundTaskKind) => k === 'model-download' || k === 'gguf-download';

function formatMb(n: number): string {
  const mb = n / (1024 * 1024);
  return mb >= 100 ? Math.round(mb).toString() : mb.toFixed(1);
}

/** Memory-job scopes are project root paths: resolve to the project name, else basename. */
function scopeLabel(task: BackgroundTask): string {
  if (PROJECT_KINDS.has(task.kind)) {
    const proj = projects().find((p) => p.path === task.scope);
    if (proj?.name) return proj.name;
    const base = task.scope.split(/[\\/]/).filter(Boolean).pop();
    return base || task.scope;
  }
  return task.scope;
}

function formatDuration(ms: number): string {
  const s = Math.max(1, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  return `${m}m${String(s % 60).padStart(2, '0')}s`;
}

const BackgroundTaskMenu: Component<BackgroundTaskMenuProps> = (props) => {
  const [menuOpen, setMenuOpen] = createSignal(false);
  const tasks = createMemo(() => backgroundTasks());
  const hasRunning = () => tasks().some((task) => task.status === 'running');
  void props;

  // Close the pull-up menu on outside click (the container class is the boundary)
  const closeMenu = (e: MouseEvent) => {
    const target = e.target as HTMLElement;
    if (menuOpen() && !target.closest('.background-task-menu-container')) {
      setMenuOpen(false);
    }
  };
  onMount(() => document.addEventListener('mousedown', closeMenu));
  onCleanup(() => document.removeEventListener('mousedown', closeMenu));

  const progressText = (task: BackgroundTask) => {
    const p = task.progress;
    if (!p || p.total <= 0) return undefined;
    if (isDownloadKind(task.kind)) {
      return `${formatMb(p.done)} / ${formatMb(p.total)} MB`;
    }
    return `${p.done} / ${p.total}`;
  };

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
          <Show when={hasRunning()}>
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
            class="absolute bottom-full right-0 mb-2 z-[41] w-[340px] rounded-xl overflow-hidden opacity-0 scale-95 transition-all duration-200 ease-out origin-bottom"
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
                {(task) => (
                  <div class="flex items-start gap-2 px-3 py-2 text-left">
                    <span class="flex items-center justify-center w-5 h-5 rounded shrink-0 bg-[rgba(var(--text-base-rgb),0.06)]">
                      <Icon
                        name={KIND_ICON[task.kind]}
                        size={12}
                        style={{ color: 'rgba(var(--text-base-rgb),0.55)' }}
                      />
                    </span>
                    <span class="flex-1 min-w-0 text-left">
                      <span class="flex items-center gap-1.5">
                        <span
                          class="text-[12px] font-medium truncate"
                          style={{ color: 'rgba(var(--text-base-rgb),0.8)' }}
                        >
                          {t(`agent.background.kind.${task.kind}`)}
                        </span>
                        <span
                          class="text-[10px] truncate shrink-0"
                          style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}
                        >
                          {scopeLabel(task)}
                        </span>
                      </span>
                      <Show when={task.detail && task.status === 'running'}>
                        <span
                          class="block truncate text-[11px]"
                          style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}
                        >
                          {task.detail}
                        </span>
                      </Show>
                      <Show when={progressText(task) && task.status === 'running'}>
                        <span class="flex items-center gap-1.5 mt-1">
                          <span
                            class="h-1 flex-1 rounded-full overflow-hidden"
                            style={{ background: 'rgba(var(--text-base-rgb),0.1)' }}
                          >
                            <span
                              class="block h-full rounded-full"
                              style={{
                                width: `${Math.min(100, Math.floor(((task.progress?.done ?? 0) / (task.progress?.total || 1)) * 100))}%`,
                                background: 'rgba(var(--primary-rgb),0.65)',
                                transition: 'width 0.2s ease-out',
                              }}
                            />
                          </span>
                          <span
                            class="font-mono text-[10px] shrink-0"
                            style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                          >
                            {progressText(task)}
                          </span>
                        </span>
                      </Show>
                      <Show when={task.status === 'error' && task.error}>
                        <span
                          class="block truncate text-[11px]"
                          style={{ color: 'var(--color-danger)' }}
                          title={task.error}
                        >
                          {task.error}
                        </span>
                      </Show>
                    </span>
                    <span class="flex items-center gap-1.5 shrink-0 pt-0.5">
                      <Show
                        when={task.status === 'running'}
                        fallback={
                          <Show
                            when={task.status === 'error'}
                            fallback={
                              <Show
                                when={task.status === 'paused'}
                                fallback={
                                  <Show
                                    when={task.status === 'cancelled'}
                                    fallback={
                                      <span
                                        class="text-[10px]"
                                        style={{ color: 'rgba(var(--text-base-rgb),0.45)' }}
                                      >
                                        {formatDuration(task.endedAt! - task.startedAt)}
                                      </span>
                                    }
                                  >
                                    <span
                                      class="text-[10px]"
                                      style={{ color: 'rgba(var(--text-base-rgb),0.45)' }}
                                    >
                                      {t('agent.background.cancelled')}
                                    </span>
                                  </Show>
                                }
                              >
                                <span
                                  class="text-[10px]"
                                  style={{ color: 'rgba(var(--text-base-rgb),0.45)' }}
                                >
                                  {t('agent.background.paused')}
                                </span>
                              </Show>
                            }
                          >
                            <Icon name="x" size={12} class="text-red-400/70 shrink-0" />
                          </Show>
                        }
                      >
                        <Icon name="spinner" size={12} class="animate-spin shrink-0" />
                      </Show>
                    </span>
                  </div>
                )}
              </For>
            </div>
          </div>
        </Show>
      </div>
    </Show>
  );
};

export default BackgroundTaskMenu;
