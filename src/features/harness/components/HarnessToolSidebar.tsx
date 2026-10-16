/**
 * Harness tool sidebar — strict replica of the chat page's ProjectSidebar /
 * TopicSidebar mechanics and visual language, extended per user requests:
 * - Dual-layer layout (expanded curtain-reveal layer + collapsed icon rail);
 * - Drag-resize strip with the floating pill handle (always draggable; the
 *   pill click toggles collapse);
 * - CLI tool rows: chat project-card style with each tool's BRAND LOGO
 *   (lobehub icons; badge chips / hand-drawn marks for the rest; custom tools
 *   fall back to initials) + availability state dot;
 * - Each tool's workspace list folds/unfolds exactly like the chat page's
 *   child-topic branches: a hover-visible caret on the parent row toggles the
 *   fold (grid-rows 0fr↔1fr + opacity), fold state persists per tool, and a
 *   tool gaining a workspace auto-unfolds (chat's new-branch auto-expand
 *   parity). Workspace rows reuse the child-topic style with the elbow
 *   connector rail;
 * - The hover "+" on a tool row adds a workspace for THAT tool (selects it,
 *   then opens the modal).
 */
import { Component, createEffect, createSignal, For, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import {
  currentHarnessToolId,
  currentHarnessWorkspaceId,
  harnessTools,
  harnessWorkspaces,
  selectHarnessTool,
  selectHarnessWorkspace,
} from '../../../core/store/harness';
import { loadHarnessToolFoldState, saveHarnessToolFoldState } from '../../../core/store/layout';
import { t } from '../../../core/i18n';
import { getHarnessLogo } from '../utils/harnessLogos';

interface HarnessToolSidebarProps {
  width: number;
  isCollapsed: boolean;
  revealMinPx?: number;
  onToggle: (e: MouseEvent) => void;
  onResize: (e: MouseEvent) => void;
  isResizing: boolean;
  onAddWorkspace: () => void;
  onRemoveWorkspace: (workspaceId: string) => void;
  onAddCustomAgent: () => void;
  onDeleteCustomAgent: (agentId: string, name: string) => void;
}

/** Page container left inset (page root px-[6px]) — compensates right padding. */
const PAGE_SIDE_INSET_PX = 6;
/** Collapsed rail width: matches the chat page's ProjectSidebar. */
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

/** Elbow-connector geometry for one workspace row (TopicSidebar child rows). */
function connectorFor(index: () => number, total: () => number) {
  const upExt = () => (index() === 0 ? 4 : 2);
  const stemOn = () => index() < total() - 1;
  const railH = () => 28 + upExt() + (stemOn() ? 2 : 0);
  const midY = () => upExt() + 14;
  const path = () =>
    stemOn()
      ? `M5 0 V${railH()} M5 ${upExt() + 10} Q5 ${midY()} 9 ${midY()} H22`
      : `M5 0 V${upExt() + 10} Q5 ${midY()} 9 ${midY()} H22`;
  return { upExt, railH, path };
}

// ---- Fold state (module scope: survives sidebar remounts; persisted) ----
// Same pattern as the chat TopicSidebar's collapsedTopicIds. prevWsCounts is
// null until the first effect run: the first run only snapshots (restored
// persisted folds must not be auto-unfolded by the initial 0→N count jump).
const [foldedToolIds, setFoldedToolIds] = createSignal<Set<string>>(loadHarnessToolFoldState());
let prevWsCounts: Map<string, number> | null = null;

const toggleFold = (toolId: string) =>
  setFoldedToolIds((prev) => {
    const next = new Set(prev);
    if (next.has(toolId)) next.delete(toolId);
    else next.add(toolId);
    return next;
  });

const HarnessToolSidebar: Component<HarnessToolSidebarProps> = (props) => {
  /** Expansion ratio 0-1: interpolates padding and fades content in by width. */
  const expandRatio = () => Math.min(Math.max(props.width / 15, 0), 1);
  /** Expanded-content opacity: meets the rail icons' fade-out curve end to end. */
  const expandedOpacity = () => Math.min(Math.max((props.width - 4) / 1, 0), 1);
  /** Curtain soft-transition mask (same math as ProjectSidebar). */
  const revealMask = () => {
    const w = props.width;
    if (props.isCollapsed || w >= 15) return 'none';
    const revealMin = props.revealMinPx ?? 0;
    if (revealMin <= 0) return 'none';
    const base = (revealMin + 38) / 0.15;
    const contentPx = (w / 100) * base - 2 * padX() - PAGE_SIDE_INSET_PX;
    const overflowPx = revealMin - contentPx;
    if (overflowPx <= 0.5) return 'none';
    const fadePx = Math.min(overflowPx + 20, contentPx * 0.6);
    return `linear-gradient(to right, black calc(100% - ${fadePx}px), transparent 100%)`;
  };
  const padY = () => (props.isCollapsed ? 6 : 6 + 9 * expandRatio());
  const padX = () => (props.isCollapsed ? 4 : 4 + 11 * expandRatio());

  // Auto-unfold: a tool that gains a workspace while folded unfolds (chat's
  // new-branch auto-expand parity). Write-through persistence follows.
  createEffect(() => {
    const list = harnessWorkspaces();
    const counts = new Map<string, number>();
    for (const w of list) counts.set(w.toolId, (counts.get(w.toolId) ?? 0) + 1);
    if (prevWsCounts !== null) {
      let changed = false;
      const next = new Set(foldedToolIds());
      for (const [tid, n] of counts) {
        if (n > (prevWsCounts.get(tid) ?? 0) && next.has(tid)) {
          next.delete(tid);
          changed = true;
        }
      }
      if (changed) setFoldedToolIds(next);
    }
    prevWsCounts = counts;
  });
  createEffect(() => saveHarnessToolFoldState(foldedToolIds()));

  /** Workspaces registered under one tool. */
  const workspacesOf = (toolId: string) => harnessWorkspaces().filter((w) => w.toolId === toolId);

  /** Brand logo; custom tools fall back to an initials chip. */
  const ToolLogo: Component<{ tool: { id: string; name: string }; size: number }> = (p) => {
    const logo = () => getHarnessLogo(p.tool.id);
    return (
      <Show
        when={logo()}
        fallback={
          <span
            class="flex items-center justify-center rounded-[5px] bg-white/[0.08] text-white/70 font-semibold shrink-0"
            style={{
              width: `${Math.round(p.size * 0.85)}px`,
              height: `${Math.round(p.size * 0.85)}px`,
              'font-size': `${Math.round(p.size * 0.42)}px`,
            }}
            title={p.tool.name}
          >
            {p.tool.name.slice(0, 2).toUpperCase()}
          </span>
        }
      >
        <img
          src={logo()!}
          alt={p.tool.name}
          class="shrink-0 object-contain"
          style={{ width: `${p.size}px`, height: `${p.size}px` }}
        />
      </Show>
    );
  };

  return (
    <div
      class="relative flex flex-col flex-shrink-0 min-w-0 z-10 h-full"
      style={{
        'min-width': `${RAIL_WIDTH_PX}px`,
        '--reveal-min': `${props.revealMinPx ?? 0}px`,
        width: props.isCollapsed ? `${RAIL_WIDTH_PX}px` : `${props.width}%`,
        padding: `${padY()}px ${padX() + PAGE_SIDE_INSET_PX}px ${padY()}px ${padX()}px`,
        transition: props.isResizing ? 'none' : 'all 0.3s cubic-bezier(0.4, 0, 0.2, 1)',
      }}
      onContextMenu={(e) => e.preventDefault()}
    >
      {/* ===== Expanded layer: curtain-style soft reveal ===== */}
      <div class="relative flex-1 min-h-0">
        <div
          class="absolute inset-0 overflow-hidden transition-opacity ease-out"
          style={{
            opacity: props.isCollapsed ? 0 : expandedOpacity(),
            'pointer-events': props.isCollapsed || expandedOpacity() < 0.5 ? 'none' : 'auto',
            'mask-image': revealMask(),
            transition: props.isResizing ? 'none' : 'opacity 0.3s ease-out',
          }}
        >
          <div
            class="h-full w-full overflow-hidden hover:overflow-y-auto"
            style={{ 'min-width': 'var(--reveal-min, 0px)' }}
          >
            {/* Section header (chat page style: uppercase tracking label) */}
            <div
              class="flex items-center h-12 text-xs uppercase tracking-[1.5px] font-semibold px-3 mb-1"
              style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}
            >
              {t('harness.sidebar.toolsSection')}
            </div>

            <For each={harnessTools()}>
              {(tool) => {
                const isSelected = () => currentHarnessToolId() === tool.id;
                const wsList = () => workspacesOf(tool.id);
                const hasWs = () => wsList().length > 0;
                const isFolded = () => foldedToolIds().has(tool.id);
                const addChild = () => {
                  // Select first so the add-workspace modal targets this tool.
                  selectHarnessTool(tool.id);
                  props.onAddWorkspace();
                };
                return (
                  <div>
                    {/* Tool card: chat project-card style + brand logo */}
                    <div
                      class="group flex items-center justify-between px-3 h-12 cursor-pointer rounded-3xl transition-all duration-200 bg-white/[0.03] border border-white/[0.04] text-white/75 hover:bg-white/[0.06] my-1"
                      classList={{ 'active-glow': isSelected() }}
                      onClick={() => selectHarnessTool(tool.id)}
                      title={`${tool.name} (${stateLabel(tool.state)})`}
                    >
                      <span
                        class="flex-grow inline-flex items-center gap-1.5 text-[0.95rem] overflow-hidden pr-[10px]"
                        style={{ color: 'rgba(var(--text-base-rgb),0.85)' }}
                      >
                        <ToolLogo tool={tool} size={18} />
                        <span class="truncate">{tool.name}</span>
                      </span>
                      <div class="flex items-center gap-0.5 shrink-0">
                        <span
                          class="w-2 h-2 rounded-full shrink-0"
                          style={{ background: stateDot(tool.state) }}
                          title={stateLabel(tool.state)}
                        />
                        {/* Fold caret (child-topic parity): hover-visible, only
                            when the tool has workspaces. Caret icons indicate
                            the click action: down = folded (click expands),
                            up = expanded (click folds up). */}
                        <Show when={hasWs()}>
                          <button
                            type="button"
                            class="flex items-center justify-center border-none bg-transparent cursor-pointer rounded-md text-white/40 hover:text-white/85 transition-all duration-200 active:scale-90 opacity-0 group-hover:opacity-100"
                            style={{ width: '24px', height: '24px' }}
                            title={
                              isFolded()
                                ? t('harness.sidebar.expandWorkspaces')
                                : t('harness.sidebar.collapseWorkspaces')
                            }
                            aria-expanded={!isFolded()}
                            onClick={(e) => {
                              e.stopPropagation();
                              toggleFold(tool.id);
                            }}
                          >
                            <Icon name={isFolded() ? 'caret-down' : 'caret-up'} size={15} />
                          </button>
                        </Show>
                        {/* Add workspace for THIS tool (hover-visible). */}
                        <button
                          type="button"
                          class="flex items-center justify-center border-none bg-transparent cursor-pointer rounded-md text-white/40 hover:text-white/85 transition-all duration-200 active:scale-90 opacity-0 group-hover:opacity-100"
                          style={{ width: '24px', height: '24px' }}
                          title={t('harness.sidebar.addWorkspace')}
                          onClick={(e) => {
                            e.stopPropagation();
                            addChild();
                          }}
                        >
                          <Icon name="plus" size={14} />
                        </button>
                        <Show when={tool.custom}>
                          <button
                            class="flex items-center justify-center w-[30px] h-[30px] border-none rounded-full cursor-pointer transition-all duration-200 active:scale-90 opacity-0 group-hover:opacity-100 bg-white/[0.06] text-white/60 hover:bg-pri-10"
                            title={t('harness.sidebar.deleteCustomTool')}
                            onClick={(e) => {
                              e.stopPropagation();
                              props.onDeleteCustomAgent(tool.id, tool.name);
                            }}
                          >
                            <Icon name="x" class="w-[14px] h-[14px]" />
                          </button>
                        </Show>
                      </div>
                    </div>

                    {/* Workspace block (child-topic parity): visible when
                        unfolded, independent of selection; fold animates via
                        grid-rows; the connector rail bridges the 4px gap. */}
                    <div
                      class="grid transition-[grid-template-rows] duration-200 ease-out"
                      style={{ 'grid-template-rows': isFolded() ? '0fr' : '1fr' }}
                    >
                      <div class="overflow-hidden min-h-0">
                        <div
                          class="space-y-0.5 my-1 transition-opacity duration-200"
                          style={{ opacity: isFolded() ? '0' : '1' }}
                        >
                          <For each={wsList()}>
                            {(ws) => {
                              const idx = () => wsList().findIndex((x) => x.id === ws.id);
                              const c = connectorFor(idx, () => wsList().length);
                              const isCurrent = () => currentHarnessWorkspaceId() === ws.id;
                              return (
                                <div
                                  class="group flex items-center justify-between cursor-pointer rounded-2xl transition-all duration-200 text-white/75 hover:bg-white/[0.06] ml-5 h-7 px-1.5 bg-transparent"
                                  classList={{ 'active-glow-sm': isCurrent() }}
                                  onClick={() => {
                                    // A workspace click implies its tool.
                                    selectHarnessTool(tool.id);
                                    selectHarnessWorkspace(ws.id);
                                  }}
                                  title={ws.path}
                                >
                                  <svg
                                    viewBox={`0 0 22 ${c.railH()}`}
                                    width="22"
                                    height={c.railH()}
                                    class="shrink-0 self-start"
                                    style={
                                      c.upExt() ? { 'margin-top': `-${c.upExt()}px` } : undefined
                                    }
                                    fill="none"
                                    stroke="rgba(var(--text-base-rgb),0.35)"
                                    stroke-width="1.5"
                                    stroke-linecap="round"
                                    aria-hidden="true"
                                  >
                                    <path d={c.path()} />
                                  </svg>
                                  <span
                                    class="flex-1 min-w-0"
                                    style={{
                                      color: 'rgba(var(--text-base-rgb),0.45)',
                                      'font-size': '0.78rem',
                                      overflow: 'hidden',
                                      'text-overflow': 'ellipsis',
                                      'white-space': 'nowrap',
                                      'padding-right': '8px',
                                      'user-select': 'none',
                                    }}
                                  >
                                    {ws.name}
                                  </span>
                                  <button
                                    class="flex items-center justify-center border-none rounded-full cursor-pointer transition-all duration-200 active:scale-90 opacity-0 group-hover:opacity-100 bg-white/[0.06] text-white/60 hover:bg-pri-10"
                                    style={{ width: '22px', height: '22px' }}
                                    title={t('harness.sidebar.removeWorkspace')}
                                    onClick={(e) => {
                                      e.stopPropagation();
                                      if (
                                        confirm(
                                          t('harness.sidebar.removeConfirm', { name: ws.name }),
                                        )
                                      ) {
                                        props.onRemoveWorkspace(ws.id);
                                      }
                                    }}
                                  >
                                    <Icon name="x" class="w-[12px] h-[12px]" />
                                  </button>
                                </div>
                              );
                            }}
                          </For>
                        </div>
                      </div>
                    </div>
                  </div>
                );
              }}
            </For>

            {/* Add custom tool: chat page's new-project button style */}
            <button
              class="w-full mt-[10px] px-3 h-12 inline-flex items-center justify-center rounded-3xl cursor-pointer transition-all duration-300"
              style={{
                background: 'rgba(var(--text-base-rgb),0.04)',
                border: '1px solid var(--border-dim)',
                color: 'rgba(var(--text-base-rgb),0.6)',
              }}
              onClick={() => props.onAddCustomAgent()}
              onMouseEnter={(e) =>
                (e.currentTarget.style.background = 'rgba(var(--primary-rgb),0.12)')
              }
              onMouseLeave={(e) =>
                (e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.04)')
              }
            >
              {t('harness.sidebar.addCustomTool')}
            </button>
          </div>
        </div>
      </div>

      {/* ===== Collapsed layer (icon rail) ===== */}
      <div
        class="absolute ease-out"
        style={{
          inset: `0 ${PAGE_SIDE_INSET_PX}px 0 0`,
          opacity: props.isCollapsed ? 1 : Math.min(Math.max((4 - props.width) / 1, 0), 1),
          'pointer-events': props.isCollapsed ? 'auto' : 'none',
          transition: props.isResizing ? 'none' : 'opacity 0.3s ease-out',
        }}
      >
        <div class="flex flex-col items-center gap-3 py-2 h-full">
          <div class="flex flex-col items-center gap-1 flex-1 w-full overflow-y-auto">
            <For each={harnessTools()}>
              {(tool) => {
                const isSelected = () => currentHarnessToolId() === tool.id;
                return (
                  <div
                    class="flex items-center justify-center w-9 h-9 rounded-full cursor-pointer transition-all duration-200 bg-white/[0.05] text-white/60 hover:bg-white/[0.10] hover:text-white select-none"
                    style={isSelected() ? 'background: rgba(var(--primary-rgb),0.30);' : ''}
                    onClick={() => selectHarnessTool(tool.id)}
                    title={`${tool.name} (${stateLabel(tool.state)})`}
                  >
                    <ToolLogo tool={tool} size={16} />
                  </div>
                );
              }}
            </For>
            <button
              class="flex items-center justify-center w-9 h-9 rounded-full transition-all duration-200 bg-white/[0.05] text-white/60 hover:bg-white/[0.10] hover:text-white"
              onClick={() => props.onAddCustomAgent()}
              title={t('harness.sidebar.addCustomTool')}
            >
              <Icon name="plus" size={16} />
            </button>
          </div>
        </div>
      </div>

      {/* Three-pane divider line (chat-page convention) */}
      <span
        aria-hidden="true"
        class="absolute top-0 bottom-0 w-px pointer-events-none"
        style={{ right: '-0.5px', background: 'rgba(var(--text-base-rgb),0.12)' }}
      />

      {/* Resize handle: always draggable (dragging out from the rail expands);
          the pill click toggles collapse — the expand affordance that survives
          the collapsed state. */}
      <div
        class="absolute top-0 bottom-0 right-[-3px] w-[6px] flex items-center justify-center cursor-ew-resize z-[1000] group"
        onMouseDown={(e) => props.onResize(e)}
      >
        <div
          class="absolute z-[1001] w-[10px] h-12 rounded-[20px] backdrop-blur-md cursor-pointer flex items-center justify-center text-xs font-bold transition-all duration-200 hover:scale-110"
          classList={{
            'opacity-0 group-hover:opacity-100': !props.isCollapsed,
            'opacity-60 group-hover:opacity-100': props.isCollapsed,
          }}
          style={{
            background: 'rgba(var(--text-base-rgb),0.08)',
            color: 'rgba(var(--text-base-rgb),0.6)',
            'box-shadow': '0 2px 8px rgba(0,0,0,0.3)',
          }}
          title={props.isCollapsed ? t('harness.sidebar.expand') : t('harness.sidebar.collapse')}
          onClick={(e) => {
            e.stopPropagation();
            props.onToggle(e);
          }}
        >
          <Icon name={props.isCollapsed ? 'chevron-right' : 'chevron-left'} size={10} />
        </div>
      </div>
    </div>
  );
};

export default HarnessToolSidebar;
