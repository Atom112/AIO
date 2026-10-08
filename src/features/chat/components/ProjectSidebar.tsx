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
  /** 展开内容的窗帘布局宽度（px）：收窄时内容保持该宽度布局，被面板左缘裁剪揭示 */
  revealMinPx?: number;
}

const DIALOG_ASST_ID = 'default-assistant-id';

/**
 * 项目侧边栏：聊天入口 + 项目列表。
 * 替代旧的 AssistantSidebar，提供"对话"和"项目"双模式入口。
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

  // 点击菜单外部时自动关闭（与右侧话题栏"更多"按钮行为一致）
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

  /** 当前助理 */
  const currentAsst = () => datas.assistants.find((a) => a.id === currentAssistantId());

  /** 是否是聊天模式活跃 */
  const isChatActive = () =>
    currentAsst()?.assistantType === 'chat' || currentAsst()?.id === DIALOG_ASST_ID;
  /** 当前活跃的项目 ID（从助理反查） */
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

  /** 切换到聊天模式 */
  const switchToChat = () => {
    setCurrentProjectId(null);
    setCurrentAssistantId(DIALOG_ASST_ID);
    const asst = datas.assistants.find((a) => a.id === DIALOG_ASST_ID);
    if (asst?.topics?.length) {
      setCurrentTopicId(asst.topics[0].id);
    }
  };

  /** 切换到某个项目 */
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

  /** 重命名项目 */
  const renameProject = async (projectId: string, newName: string) => {
    if (!newName.trim()) return;
    try {
      await updateProject(projectId, newName.trim());
      // 也更新助理名称
      const asst = datas.assistants.find((a) => a.projectId === projectId);
      if (asst) {
        setDatas('assistants', (a) => a.id === asst.id, 'name', newName.trim());
        await saveSingleAssistantToBackend(asst.id);
      }
    } catch (e) {
      alert(t('project.renameFailed') + '\n' + e);
    }
  };

  /** 删除项目 */
  const deleteProject = async (projectId: string) => {
    if (!confirm(t('project.deleteConfirm'))) return;
    try {
      const asst = datas.assistants.find((a) => a.projectId === projectId);
      if (asst) {
        await deleteAssistantFile(asst.id);
        // 如果当前选中的是这个助理，切换到聊天
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

  /** 创建项目成功后 */
  const handleProjectCreated = async (projectId: string) => {
    setShowProjectCreateModal(false);
    // 重新加载项目列表
    try {
      const list = await listProjects();
      setProjects(list);
    } catch {
      /* ignore */
    }
    await switchToProject(projectId);
  };

  /** 过滤：仅显示 project-type 的助理关联的项目（排除 chat 和游离助理） */
  const projectList = createMemo(() => {
    const allProjects = projects();
    return allProjects;
  });

  /** 展开程度 0–1：用于插值内边距与按宽度渐显内容 */
  const expandRatio = () => Math.min(Math.max(props.width / 15, 0), 1);
  /**
   * 展开态内容不透明度：与轨道图标的淡出曲线首尾相接（4%–5% 交叉），
   * 保证任意宽度下两套内容至少一套可见，不再出现空过渡帧；
   * 内容本体走"窗帘式揭示"（固定布局 + 裁剪），不依赖淡出来遮丑。
   */
  const expandedOpacity = () => Math.min(Math.max((props.width - 4) / 1, 0), 1);
  /**
   * 窗帘软过渡遮罩：按"精确溢出量"计算右缘渐隐区（完整覆盖被裁剪区域并留 20px 余量），
   * 杜绝裁剪线裸露——图标始终清晰、标签渐隐，不会被拦腰截断；
   * 宽度回到 15%（无溢出）时渐隐区收敛为 0，切换无缝。
   * 容器宽度由 revealMin 反推：revealMin = base×0.15 − 38（ChatPage 同口径，留 2px 安全余量）。
   */
  const revealMask = () => {
    const w = props.width;
    if (props.isCollapsed || w >= 15) return 'none';
    const revealMin = props.revealMinPx ?? 0;
    if (revealMin <= 0) return 'none';
    const base = (revealMin + 38) / 0.15;
    const contentPx = (w / 100) * base - 2 * padX() - PAGE_SIDE_INSET_PX; // 扣两侧不对称内边距
    const overflowPx = revealMin - contentPx;
    if (overflowPx <= 0.5) return 'none';
    const fadePx = Math.min(overflowPx + 20, contentPx * 0.6);
    return `linear-gradient(to right, black calc(100% - ${fadePx}px), transparent 100%)`;
  };
  /**
   * 页面容器左内边距（ChatPage 根节点 px-[6px]）。右侧分隔线紧贴面板外缘，
   * 而左侧元素与窗口边界之间隔着这 6px——把右侧内边距补上同值，
   * 元素到左窗口边界与到右分隔线的间距才相等（展开/收起各自保持对应的间距值）。
   */
  const PAGE_SIDE_INSET_PX = 6;
  /** 收起轨道宽度：54px 使图标两侧各留 12px（与窗口边界间距一致），图标在窗口↔分隔线区间居中 */
  const RAIL_WIDTH_PX = 54;
  /** 实际生效的内边距（px）：收起态为轨道值；展开态随宽度从轨道值插值到 15px */
  const padY = () => (props.isCollapsed ? 6 : 6 + 9 * expandRatio());
  const padX = () => (props.isCollapsed ? 4 : 4 + 11 * expandRatio());

  return (
    <div
      class="relative flex flex-col flex-shrink-0 min-w-0 z-10 h-full"
      style={{
        // CSS 级最窄限制：无论上层计算出什么宽度，面板物理上不可能窄于收起轨道
        'min-width': `${RAIL_WIDTH_PX}px`,
        // 窗帘揭示的布局宽度（子内容以该宽度排版，收窄时被裁剪而不是重排）
        '--reveal-min': `${props.revealMinPx ?? 0}px`,
        width: props.isCollapsed ? `${RAIL_WIDTH_PX}px` : `${props.width}%`,
        // 右内边距比左侧多 PAGE_SIDE_INSET_PX：补偿页面容器左内边距，
        // 使元素到左窗口边界与到右分隔线的间距相等（随宽度插值，两态各自独立取值）
        padding: `${padY()}px ${padX() + PAGE_SIDE_INSET_PX}px ${padY()}px ${padX()}px`,
        transition: props.isResizing ? 'none' : 'all 0.3s cubic-bezier(0.4, 0, 0.2, 1)',
      }}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div class="relative flex-1 min-h-0">
        {/* 展开状态 — 窗帘式软揭示：内容保持完整布局不重排，超出面板的部分被裁剪，右缘用 mask 渐隐替代硬裁剪 */}
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
            {/* 聊天入口 */}
            <div
              class="group flex items-center justify-between px-3 h-12 cursor-pointer rounded-3xl transition-all duration-200 bg-white/[0.03] border border-white/[0.04] text-white/75 hover:bg-white/[0.06] my-1"
              classList={{
                '!bg-[rgba(var(--primary-rgb),0.22)] !border-[rgba(var(--primary-rgb),0.22)]':
                  isChatActive(),
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

            {/* 分隔线 */}
            <div class="my-3 border-t" style={{ 'border-color': 'var(--border-dim)' }} />

            {/* 项目区域 */}
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
                      '!bg-[rgba(var(--primary-rgb),0.22)] !border-[rgba(var(--primary-rgb),0.22)]':
                        isActive(),
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

            {/* 新建项目按钮 */}
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

      {/* 收起状态（图标轨道）— 挂在面板根上，右缩进 PAGE_SIDE_INSET_PX：图标在"窗口边界↔分隔线"区间居中，两侧间距恒等；展开态随透明度归零，右缩进无视觉影响 */}
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

      {/* 上下文菜单 */}
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

      {/* 新建项目弹窗 — Portal 到 body 避免被侧边栏 overflow-hidden 裁剪 */}
      <Show when={showProjectCreateModal()}>
        <Portal>
          <ProjectCreateModal
            onClose={() => setShowProjectCreateModal(false)}
            onCreated={handleProjectCreated}
          />
        </Portal>
      </Show>

      {/* 三栏分界浅竖线：置于面板外缘（±0.5px），中心与下方调整把手的中心严格重合 */}
      <span
        aria-hidden="true"
        class="absolute top-0 bottom-0 w-px pointer-events-none"
        style={{ right: '-0.5px', background: 'rgba(var(--text-base-rgb),0.12)' }}
      />

      {/* 调整大小把手：6px 抓取条居中于面板外缘（与分界竖线同心），收起态仍可向外拖出展开 */}
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
