/**
 * Harness tool sidebar — level 1: the detected CLI tools (with availability
 * state); level 2: the workspaces registered under the selected tool (accordion
 * nesting). Collapsed rail mode mirrors the chat ProjectSidebar.
 */
import { Component, For, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import {
  currentHarnessToolId,
  currentHarnessWorkspaces,
  currentHarnessWorkspaceId,
  harnessTools,
  selectHarnessTool,
  selectHarnessWorkspace,
} from '../../../core/store/harness';
import { t } from '../../../core/i18n';

interface HarnessToolSidebarProps {
  width: number;
  isCollapsed: boolean;
  revealMinPx?: number;
  onToggle: (e: MouseEvent) => void;
  onResize: (e: MouseEvent) => void;
  isResizing: boolean;
  onAddWorkspace: () => void;
  onRemoveWorkspace: (workspaceId: string) => void;
}

const RAIL_WIDTH_PX = 54;

/** Availability dot color per tool state. */
const stateDot = (state: string) =>
  state === 'ready'
    ? 'rgba(74,222,128,0.9)'
    : state === 'installable'
      ? 'rgba(250,204,21,0.85)'
      : 'rgba(var(--text-base-rgb),0.25)';

const stateLabel = (state: string) =>
  state === 'ready'
    ? t('harness.tool.ready')
    : state === 'installable'
      ? t('harness.tool.installable')
      : t('harness.tool.missing');

const HarnessToolSidebar: Component<HarnessToolSidebarProps> = (props) => {
  return (
    <div
      class="relative flex flex-col flex-shrink-0 min-w-0 h-full"
      style={{
        'min-width': `${RAIL_WIDTH_PX}px`,
        width: props.isCollapsed ? `${RAIL_WIDTH_PX}px` : `${props.width}%`,
        padding: '6px 3px 6px 0px',
        transition: props.isResizing ? 'none' : 'all 0.3s cubic-bezier(0.4, 0, 0.2, 1)',
      }}
    >
      {/* Three-pane divider line (chat-page convention) */}
      <div
        class="absolute top-0 right-0 h-full w-px z-[1]"
        style={{ background: 'var(--border-dim)' }}
      />
      {/* Resize handle */}
      <div
        class="absolute top-0 right-[-3px] h-full w-[6px] cursor-ew-resize z-[3] hover:bg-white/[0.04]"
        onMouseDown={(e) => !props.isCollapsed && props.onResize(e)}
        style={{ 'touch-action': 'none' }}
      />

      <Show
        when={!props.isCollapsed}
        fallback={
          // Collapsed rail: tool initials only
          <div class="flex flex-col items-center gap-1 pt-2">
            <For each={harnessTools()}>
              {(tool) => (
                <button
                  class={`flex items-center justify-center w-9 h-9 rounded-lg transition-colors cursor-pointer ${
                    currentHarnessToolId() === tool.id
                      ? 'bg-pri-20 text-white'
                      : 'text-white/50 hover:text-white/85 hover:bg-white/[0.06]'
                  }`}
                  title={tool.name}
                  onClick={() => selectHarnessTool(tool.id)}
                >
                  <span class="text-[11px] font-semibold">
                    {tool.name.slice(0, 2).toUpperCase()}
                  </span>
                </button>
              )}
            </For>
          </div>
        }
      >
        <div class="relative flex-1 min-h-0 pl-[15px] pr-[18px] pt-2">
          {/* Collapse toggle */}
          <button
            class="absolute top-1 right-1 z-[4] text-white/30 hover:text-white/70 transition-colors"
            onClick={(e) => props.onToggle(e)}
            title={t('harness.sidebar.collapse')}
          >
            <Icon name="chevron-left" class="w-4 h-4" />
          </button>

          <div class="h-full overflow-y-auto pr-1 pt-6">
            <For each={harnessTools()}>
              {(tool) => (
                <div class="mb-0.5">
                  <button
                    class={`w-full flex items-center gap-2 px-2.5 py-2 rounded-lg transition-colors cursor-pointer text-left ${
                      currentHarnessToolId() === tool.id
                        ? 'bg-pri-20 text-white'
                        : 'text-white/60 hover:text-white/85 hover:bg-white/[0.06]'
                    }`}
                    onClick={() => selectHarnessTool(tool.id)}
                    title={`${tool.name} (${stateLabel(tool.state)})`}
                  >
                    <span
                      class="w-2 h-2 rounded-full shrink-0"
                      style={{ background: stateDot(tool.state) }}
                    />
                    <span class="text-[13px] font-medium truncate flex-1">{tool.name}</span>
                    <Show when={currentHarnessToolId() === tool.id}>
                      <Icon name="chevron-down" class="w-3.5 h-3.5 shrink-0" />
                    </Show>
                  </button>

                  {/* Workspaces (level 2) */}
                  <Show when={currentHarnessToolId() === tool.id}>
                    <div class="mt-0.5 mb-1.5 flex flex-col gap-0.5 pl-[26px]">
                      <For each={currentHarnessWorkspaces()}>
                        {(ws) => (
                          <div
                            class={`group flex items-center gap-2 px-2 py-1.5 rounded-md cursor-pointer transition-colors ${
                              currentHarnessWorkspaceId() === ws.id
                                ? 'bg-pri-20/60 text-white'
                                : 'text-white/50 hover:text-white/80 hover:bg-white/[0.05]'
                            }`}
                            onClick={() => selectHarnessWorkspace(ws.id)}
                            title={ws.path}
                          >
                            <Icon name="folder" size={13} class="shrink-0" />
                            <span class="text-xs truncate flex-1">{ws.name}</span>
                            <button
                              class="shrink-0 opacity-0 group-hover:opacity-100 text-white/30 hover:text-danger transition-colors"
                              title={t('harness.sidebar.removeWorkspace')}
                              onClick={(e) => {
                                e.stopPropagation();
                                if (
                                  confirm(t('harness.sidebar.removeConfirm', { name: ws.name }))
                                ) {
                                  props.onRemoveWorkspace(ws.id);
                                }
                              }}
                            >
                              <Icon name="x" class="w-3 h-3" />
                            </button>
                          </div>
                        )}
                      </For>
                      <Show when={currentHarnessWorkspaces().length === 0}>
                        <div class="px-2 py-1.5">
                          <div class="text-[11px] leading-4 text-white/35">
                            {t('harness.sidebar.emptyWorkspace')}
                          </div>
                        </div>
                      </Show>
                      <button
                        class="flex items-center gap-2 px-2 py-1.5 rounded-md text-white/40 hover:text-white/75 hover:bg-white/[0.05] transition-colors cursor-pointer"
                        onClick={() => props.onAddWorkspace()}
                      >
                        <Icon name="plus" size={13} />
                        <span class="text-xs">{t('harness.sidebar.addWorkspace')}</span>
                      </button>
                    </div>
                  </Show>
                </div>
              )}
            </For>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default HarnessToolSidebar;
