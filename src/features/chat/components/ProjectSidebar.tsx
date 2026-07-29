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
import { invoke } from '@tauri-apps/api/core';
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
      await invoke('update_project', { id: projectId, name: newName.trim() });
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
      await invoke('delete_project', { id: projectId });
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
      const list = await invoke<
        Array<{
          id: string;
          name: string;
          path: string;
          createdAt: string;
          updatedAt: string;
          assistantId: string;
        }>
      >('list_projects');
      setProjects(
        list.map((p: Record<string, string>) => ({
          id: p.id,
          name: p.name,
          path: p.path,
          createdAt: p.createdAt,
          updatedAt: p.updatedAt,
          assistantId: p.assistantId,
        })),
      );
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

  return (
    <div
      class="relative flex flex-col flex-shrink-0 min-w-0 z-10 h-full"
      style={{
        width: props.isCollapsed ? '48px' : `${props.width}%`,
        padding: props.isCollapsed ? '6px 4px' : '15px',
        background: 'rgba(var(--surface-bg), 0.15)',
        'backdrop-filter': 'blur(16px)',
        border: '1px solid var(--border-dim)',
        'border-radius': '12px',
        'box-shadow':
          'inset 0 0 1px rgba(var(--text-base-rgb),0.06), 0 8px 32px rgba(0, 0, 0, 0.2)',
        transition: props.isResizing ? 'none' : 'all 0.3s cubic-bezier(0.4, 0, 0.2, 1)',
      }}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div class="relative flex-1 min-h-0">
        {/* 展开状态 — 折叠时淡出 */}
        <div
          class="absolute inset-0 transition-opacity duration-300 ease-out"
          classList={{ 'opacity-0 pointer-events-none': props.isCollapsed }}
        >
          <div class="h-full w-full overflow-hidden hover:overflow-y-auto">
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
              新建项目
            </button>
          </div>
        </div>

        {/* 收起状态 — 展开时淡出 */}
        <div
          class="absolute inset-0 transition-opacity duration-300 ease-out"
          classList={{ 'opacity-0 pointer-events-none': !props.isCollapsed }}
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

      {/* 调整大小把手 */}
      <div
        class="absolute top-0 bottom-0 right-[-4px] w-1 flex items-center justify-center cursor-ew-resize z-[1000] group"
        onMouseDown={(e) => props.onResize(e as MouseEvent)}
      >
        <div
          class="absolute z-[1001] w-[10px] h-12 rounded-[20px] backdrop-blur-md cursor-pointer flex items-center justify-center text-xs font-bold transition-all duration-200 opacity-0 group-hover:opacity-100 hover:scale-110"
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
