import { Component, For, Show, createSignal, createMemo, onMount, onCleanup } from 'solid-js';
import { Portal } from 'solid-js/web';
import {
  datas,
  currentAssistantId,
  setCurrentAssistantId,
  setCurrentTopicId,
  projects,
  setProjects,
  setCurrentProjectId,
  ensureProjectAssistant,
  saveSingleAssistantToBackend,
  setDatas,
  deleteAssistantFile,
  initMcpServers,
  showProjectCreateModal,
  setShowProjectCreateModal,
} from '../../../core/store/store';
import {
  deleteProject as deleteProjectApi,
  listProjects,
  updateProject,
} from '../../../core/api/project';
import Icon from '../../../shared/components/Icon';
import ProjectCreateModal from './ProjectCreateModal';
import { t } from '../../../core/i18n';

interface ProjectSidebarProps {
  width: number;
  isCollapsed: boolean;
  onToggle: (e: MouseEvent) => void;
  onResize: (e: MouseEvent) => void;
  isResizing: boolean;
  onOpenSettings: (assistantId: string) => void;
  /** Layout width (px) for expanded content: when narrowing, content keeps this width and is revealed by clipping at the panel's left edge */
  revealMinPx?: number;
}

const DIALOG_ASST_ID = 'default-assistant-id';

/**
 * Project sidebar: chat entry + project list.
 * Provides the "chat" and "project" dual-mode entries.
 */
const ProjectSidebar: Component<ProjectSidebarProps> = (props) => {
  const [showMenuDiv, setShowMenuDiv] = createSignal(false);
  const [isMenuAnimatingOut, setIsMenuAnimatingOut] = createSignal(false);
  const [menuState, setMenuState] = createSignal({
    isOpen: false,
    x: 0,
    y: 0,
    targetProjectId: null as string | null,
  });

  let menuCloseTimeoutId: ReturnType<typeof setTimeout>;
  const [editingProjectId, setEditingProjectId] = createSignal<string | null>(null);

  // Auto-close when clicking outside the menu (matches the topic sidebar "more" button behavior)
  onMount(() => {
    const handleClickOutside = () => {
      if (menuState().isOpen) closeMenu();
    };
    window.addEventListener('click', handleClickOutside);
    onCleanup(() => window.removeEventListener('click', handleClickOutside));

    const handleContextMenuOutside = (e: MouseEvent) => {
      if (menuState().isOpen) {
        e.preventDefault();
        closeMenu();
      }
    };
    window.addEventListener('contextmenu', handleContextMenuOutside);
    onCleanup(() => window.removeEventListener('contextmenu', handleContextMenuOutside));
  });

  /** Current assistant */
  const currentAsst = () => datas.assistants.find((a) => a.id === currentAssistantId());

  /** Whether chat mode is active */
  const isChatActive = () =>
    currentAsst()?.assistantType === 'chat' || currentAsst()?.id === DIALOG_ASST_ID;
  /** Currently active project ID (resolved from the assistant) */
  const activeProjectId = () => currentAsst()?.projectId ?? null;

  const openMenu = (e: MouseEvent, projectId: string, isRightClick = false) => {
    e.stopPropagation();
    if (isRightClick) e.preventDefault();
    if (menuState().isOpen && menuState().targetProjectId === projectId) {
      closeMenu();
      return;
    }
    setShowMenuDiv(true);
    setIsMenuAnimatingOut(false);
    let x: number, y: number;
    if (isRightClick) {
      x = e.clientX;
      y = e.clientY;
    } else {
      const rect = (e.currentTarget as Element).getBoundingClientRect();
      x = rect.left;
      y = rect.top + rect.height;
    }
    setMenuState({ isOpen: true, x, y, targetProjectId: projectId });
  };

  const closeMenu = () => {
    setMenuState((p) => ({ ...p, isOpen: false }));
    setIsMenuAnimatingOut(true);
    clearTimeout(menuCloseTimeoutId);
    menuCloseTimeoutId = setTimeout(() => {
      setShowMenuDiv(false);
      setIsMenuAnimatingOut(false);
    }, 200);
  };

  /** Switch to chat mode */
  const switchToChat = () => {
    setCurrentProjectId(null);
    setCurrentAssistantId(DIALOG_ASST_ID);
    const asst = datas.assistants.find((a) => a.id === DIALOG_ASST_ID);
    if (asst?.topics?.length) {
      setCurrentTopicId(asst.topics[0].id);
    }
  };

  /** Switch to a specific project */
  const switchToProject = async (projectId: string) => {
    try {
      setCurrentProjectId(projectId);
      const asstId = await ensureProjectAssistant(projectId);
      setCurrentAssistantId(asstId);
      initMcpServers(projectId);
      const asst = datas.assistants.find((a) => a.id === asstId);
      if (asst?.topics?.length) {
        setCurrentTopicId(asst.topics[0].id);
      }
    } catch (e) {
      console.error('切换项目失败:', e);
    }
  };

  /** Rename a project */
  const renameProject = async (projectId: string, newName: string) => {
    if (!newName.trim()) return;
    try {
      await updateProject(projectId, newName.trim());
      // Also update the assistant name
      const asst = datas.assistants.find((a) => a.projectId === projectId);
      if (asst) {
        setDatas('assistants', (a) => a.id === asst.id, 'name', newName.trim());
        await saveSingleAssistantToBackend(asst.id);
      }
    } catch (e) {
      alert(t('project.renameFailed') + '\n' + e);
    }
  };

  /** Delete a project */
  const deleteProject = async (projectId: string) => {
    if (!confirm(t('project.deleteConfirm'))) return;
    try {
      const asst = datas.assistants.find((a) => a.projectId === projectId);
      if (asst) {
        await deleteAssistantFile(asst.id);
        // If this assistant is currently selected, switch back to chat
        if (currentAssistantId() === asst.id) {
          switchToChat();
        }
        setDatas('assistants', (prev) => prev.filter((a) => a.id !== asst.id));
      }
      await deleteProjectApi(projectId);
      setProjects((prev) => prev.filter((p) => p.id !== projectId));
    } catch (e) {
      alert(t('project.deleteFailed') + '\n' + e);
    }
    closeMenu();
  };

  /** After a project is created */
  const handleProjectCreated = async (projectId: string) => {
    setShowProjectCreateModal(false);
    // Reload the project list
    try {
      const list = await listProjects();
      setProjects(list);
    } catch {
      /* ignore */
    }
    await switchToProject(projectId);
  };

  /** Filter: only show projects tied to project-type assistants (excludes chat and orphan assistants) */
  const projectList = createMemo(() => {
    const allProjects = projects();
    return allProjects;
  });

  /** Expansion ratio 0-1: used to interpolate padding and fade content in by width */
  const expandRatio = () => Math.min(Math.max(props.width / 15, 0), 1);
  /**
   * Expanded-content opacity: meets the rail icons' fade-out curve end to end (crossing at 4%-5%),
   * so at any width at least one of the two content layers stays visible with no empty transition frames;
   * the content itself uses a curtain-style reveal (fixed layout + clipping) and does not rely on fading to hide it.
   */
  const expandedOpacity = () => Math.min(Math.max((props.width - 4) / 1, 0), 1);
  /**
   * Curtain soft-transition mask: computes the right-edge fade zone from the exact overflow amount
   * (covering the whole clipped region plus a 20px margin) so no raw clip line is exposed: icons stay sharp
   * while labels fade out instead of being cut mid-glyph. Back at 15% width (no overflow) the fade zone
   * collapses to 0 for a seamless switch. Container width is derived from revealMin: revealMin = base*0.15 - 38 (same as ChatPage, with 2px safety margin).
   */
  const revealMask = () => {
    const w = props.width;
    if (props.isCollapsed || w >= 15) return 'none';
    const revealMin = props.revealMinPx ?? 0;
    if (revealMin <= 0) return 'none';
    const base = (revealMin + 38) / 0.15;
    const contentPx = (w / 100) * base - 2 * padX() - PAGE_SIDE_INSET_PX; // subtract the asymmetric horizontal padding
    const overflowPx = revealMin - contentPx;
    if (overflowPx <= 0.5) return 'none';
    const fadePx = Math.min(overflowPx + 20, contentPx * 0.6);
    return `linear-gradient(to right, black calc(100% - ${fadePx}px), transparent 100%)`;
  };
  /**
   * Page container left inset (ChatPage root px-[6px]). The right divider sits flush against the panel edge,
   * while left-side elements are 6px away from the window edge. Adding the same value to the right padding
   * makes the spacing to the left window edge and to the right divider equal (each state keeps its own spacing values).
   */
  const PAGE_SIDE_INSET_PX = 6;
  /** Collapsed rail width: 54px leaves 12px on each side of the icons (matching the window-edge spacing), centering icons between window edge and divider */
  const RAIL_WIDTH_PX = 54;
  /** Effective padding (px): rail value when collapsed; when expanded, interpolated from the rail value to 15px by width */
  const padY = () => (props.isCollapsed ? 6 : 6 + 9 * expandRatio());
  const padX = () => (props.isCollapsed ? 4 : 4 + 11 * expandRatio());

  return (
    <div
      class="relative flex flex-col flex-shrink-0 min-w-0 z-10 h-full"
      style={{
        // CSS-level minimum width: whatever width the upper layer computes, the panel physically cannot be narrower than the collapsed rail
        'min-width': `${RAIL_WIDTH_PX}px`,
        // Curtain-reveal layout width (child content lays out at this width; narrowing clips it instead of reflowing)
        '--reveal-min': `${props.revealMinPx ?? 0}px`,
        width: props.isCollapsed ? `${RAIL_WIDTH_PX}px` : `${props.width}%`,
        // Right padding exceeds the left by PAGE_SIDE_INSET_PX to compensate the page container's left inset,
        // equalizing spacing to the left window edge and the right divider (interpolated by width; each state takes its own value)
        padding: `${padY()}px ${padX() + PAGE_SIDE_INSET_PX}px ${padY()}px ${padX()}px`,
        transition: props.isResizing ? 'none' : 'all 0.3s cubic-bezier(0.4, 0, 0.2, 1)',
      }}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div class="relative flex-1 min-h-0">
        {/* Expanded state - curtain-style soft reveal: content keeps its full layout without reflow, the part beyond the panel is clipped, and the right edge uses a mask fade instead of a hard clip */}
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
            {/* Chat entry */}
            <div
              class="group flex items-center justify-between px-3 h-12 cursor-pointer rounded-3xl transition-all duration-200 bg-white/[0.03] border border-white/[0.04] text-white/75 hover:bg-white/[0.06] my-1"
              classList={{
                '!border-[rgba(var(--primary-rgb),0.55)] active-glow': isChatActive(),
              }}
              onClick={switchToChat}
            >
              <span
                class="flex-grow inline-flex items-center gap-1.5 text-[0.95rem] overflow-hidden pr-[10px]"
                style={{ color: 'rgba(var(--text-base-rgb),0.85)' }}
              >
                <Icon name="chat" size={14} class="shrink-0" />{' '}
                <span class="truncate">{t('project.sidebar.dialog')}</span>
              </span>
            </div>

            {/* Divider */}
            <div class="my-3 border-t" style={{ 'border-color': 'var(--border-dim)' }} />

            {/* Project section */}
            <div
              class="flex items-center h-12 text-xs uppercase tracking-[1.5px] font-semibold px-3 mb-1"
              style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}
            >
              {t('project.sidebar.project')}
            </div>

            <For each={projectList()}>
              {(project) => {
                const isActive = () => activeProjectId() === project.id;
                return (
                  <div
                    class="group flex items-center justify-between px-3 h-12 cursor-pointer rounded-3xl transition-all duration-200 bg-white/[0.03] border border-white/[0.04] text-white/75 hover:bg-white/[0.06] my-1"
                    classList={{
                      '!border-[rgba(var(--primary-rgb),0.55)] active-glow': isActive(),
                    }}
                    onContextMenu={(e) => openMenu(e as MouseEvent, project.id, true)}
                    onClick={() => switchToProject(project.id)}
                  >
                    <Show
                      when={editingProjectId() === project.id}
                      fallback={
                        <span
                          class="flex-grow inline-flex items-center gap-1.5 text-[0.95rem] overflow-hidden pr-[10px]"
                          style={{ color: 'rgba(var(--text-base-rgb),0.85)' }}
                        >
                          <Icon name="folder" size={14} class="shrink-0" />{' '}
                          <span class="truncate">{project.name}</span>
                        </span>
                      }
                    >
                      <input
                        class="flex-grow rounded px-2 py-0.5 text-[0.85rem] h-6 outline-none mx-1"
                        style={{
                          background: 'rgba(0,0,0,0.3)',
                          border: '1px solid var(--border-dim)',
                          color: 'rgba(var(--text-base-rgb),0.85)',
                        }}
                        value={project.name}
                        ref={(el) => {
                          setTimeout(() => {
                            el.focus();
                            el.select();
                          }, 0);
                        }}
                        onBlur={(e) => {
                          renameProject(project.id, e.currentTarget.value);
                          setEditingProjectId(null);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            renameProject(project.id, e.currentTarget.value);
                            setEditingProjectId(null);
                          } else if (e.key === 'Escape') setEditingProjectId(null);
                        }}
                        onClick={(e) => e.stopPropagation()}
                      />
                    </Show>

                    <button
                      class="flex items-center justify-center w-[30px] h-[30px] border-none rounded-full cursor-pointer transition-all duration-200 active:scale-90 opacity-0 group-hover:opacity-100 bg-white/[0.06] text-white/60 hover:bg-pri-10"
                      onClick={(e) => openMenu(e as MouseEvent, project.id)}
                    >
                      <Icon src="/icons/app-logo/dot-menu.svg" class="w-[18px] h-[18px]" />
                    </button>
                  </div>
                );
              }}
            </For>

            {/* New project button */}
            <button
              class="w-full mt-[10px] px-3 h-12 inline-flex items-center justify-center rounded-3xl cursor-pointer transition-all duration-300"
              style={{
                background: 'rgba(var(--text-base-rgb),0.04)',
                border: '1px solid var(--border-dim)',
                color: 'rgba(var(--text-base-rgb),0.6)',
              }}
              onClick={() => setShowProjectCreateModal(true)}
              onMouseEnter={(e) =>
                (e.currentTarget.style.background = 'rgba(var(--primary-rgb),0.12)')
              }
              onMouseLeave={(e) =>
                (e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.04)')
              }
            >
              {t('project.sidebar.newProject')}
            </button>
          </div>
        </div>
      </div>

      {/* Collapsed state (icon rail) - attached to the panel root with right inset PAGE_SIDE_INSET_PX: icons center in the window-edge-to-divider band with equal spacing on both sides; fades to zero with opacity when expanded, so the right inset has no visual effect */}
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
          <button
            class="flex items-center justify-center w-9 h-9 rounded-full transition-all duration-200 bg-white/[0.05] text-white/75 hover:bg-white/[0.10] hover:text-white"
            style={isChatActive() ? 'background: rgba(var(--primary-rgb),0.30);' : ''}
            onClick={switchToChat}
            title={t('project.sidebar.dialog') as string}
          >
            <Icon name="chat" size={16} />
          </button>
          <div class="w-5 border-t" style={{ 'border-color': 'var(--border-dim)' }} />
          <div class="flex flex-col items-center gap-1 flex-1 w-full overflow-y-auto">
            <For each={projectList()}>
              {(project) => {
                const isActive = () => activeProjectId() === project.id;
                return (
                  <div
                    class="flex items-center justify-center w-9 h-9 rounded-full cursor-pointer transition-all duration-200 bg-white/[0.05] text-white/60 hover:bg-white/[0.10] hover:text-white select-none"
                    style={isActive() ? 'background: rgba(var(--primary-rgb),0.30);' : ''}
                    onClick={() => switchToProject(project.id)}
                    onContextMenu={(e) => openMenu(e as MouseEvent, project.id, true)}
                    title={project.name}
                  >
                    <span class="text-xs font-semibold">
                      {(project.name || '?').charAt(0).toUpperCase()}
                    </span>
                  </div>
                );
              }}
            </For>
            <button
              class="flex items-center justify-center w-9 h-9 rounded-full transition-all duration-200 bg-white/[0.05] text-white/60 hover:bg-white/[0.10] hover:text-white"
              onClick={() => setShowProjectCreateModal(true)}
              title={t('project.sidebar.newProject')}
            >
              <Icon name="plus" size={16} />
            </button>
          </div>
        </div>
      </div>

      {/* Context menu */}
      {showMenuDiv() && (
        <Portal>
          <div
            class="fixed z-[1500] min-w-[150px] rounded-lg shadow-[0_12px_40px_rgba(0,0,0,0.35)] py-1 origin-top-left"
            style={{
              top: `${menuState().y}px`,
              left: `${menuState().x}px`,
              background: 'var(--acrylic-bg)',
              'backdrop-filter': 'blur(24px) saturate(150%)',
              border: '1px solid var(--acrylic-border)',
              animation: isMenuAnimatingOut()
                ? 'contextMenuOut 0.14s ease-in forwards'
                : 'contextMenuIn 0.18s cubic-bezier(0.2, 0.8, 0.2, 1) forwards',
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <button
              class="w-full text-left px-3 py-2 bg-transparent border-none cursor-pointer rounded-lg transition-all duration-200 text-white/75 hover:bg-pri-10 hover:text-white"
              onClick={() => {
                const pid = menuState().targetProjectId;
                if (pid) {
                  const asst = datas.assistants.find((a) => a.projectId === pid);
                  if (asst) props.onOpenSettings(asst.id);
                }
                closeMenu();
              }}
            >
              {t('project.settings')}
            </button>
            <button
              class="w-full text-left px-3 py-2 bg-transparent border-none cursor-pointer rounded-lg transition-all duration-200 text-white/75 hover:bg-pri-10 hover:text-white"
              onClick={() => {
                const pid = menuState().targetProjectId;
                if (pid) setEditingProjectId(pid);
                closeMenu();
              }}
            >
              {t('project.rename')}
            </button>
            <button
              class="w-full text-left px-3 py-2 bg-transparent border-none cursor-pointer rounded-lg transition-all duration-200"
              style={{ color: 'rgba(255,77,77,0.8)' }}
              onClick={() => deleteProject(menuState().targetProjectId!)}
            >
              {t('project.delete')}
            </button>
          </div>
        </Portal>
      )}

      {/* New project modal - portaled to body to avoid clipping by the sidebar's overflow-hidden */}
      <Show when={showProjectCreateModal()}>
        <Portal>
          <ProjectCreateModal
            onClose={() => setShowProjectCreateModal(false)}
            onCreated={handleProjectCreated}
          />
        </Portal>
      </Show>

      {/* Three-pane divider line: placed on the panel edge (+/-0.5px), its center exactly aligned with the resize handle below */}
      <span
        aria-hidden="true"
        class="absolute top-0 bottom-0 w-px pointer-events-none"
        style={{ right: '-0.5px', background: 'rgba(var(--text-base-rgb),0.12)' }}
      />

      {/* Resize handle: 6px grab strip centered on the panel edge (concentric with the divider line); when collapsed it can still be dragged outward to expand */}
      <div
        class="absolute top-0 bottom-0 right-[-3px] w-[6px] flex items-center justify-center cursor-ew-resize z-[1000] group"
        onMouseDown={(e) => props.onResize(e as MouseEvent)}
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
          title={props.isCollapsed ? t('project.sidebar.expand') : t('project.sidebar.collapse')}
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

export default ProjectSidebar;
