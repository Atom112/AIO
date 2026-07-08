/**
 * @file ProjectSelector.tsx
 * @description 项目选择下拉组件，位于聊天输入栏左下方。支持切换全局/项目模式。
 */
import { createSignal, onMount, Show, For } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import {
    currentProjectId,
    setCurrentProjectId,
    projects,
    setProjects,
    initProjects,
    initSkills,
    initMcpServers,
    currentProject,
    saveLastAgentProjectId,
} from '../store/store';
import ProjectCreateModal from './ProjectCreateModal';

let ref: HTMLDivElement | undefined;

export default function ProjectSelector() {
    const [open, setOpen] = createSignal(false);
    const [showCreate, setShowCreate] = createSignal(false);

    onMount(() => {
        initProjects();
    });

    const selectProject = async (id: string) => {
        setCurrentProjectId(id);
        saveLastAgentProjectId(id);
        setOpen(false);
        // 重新加载 skills 和 MCP
        await Promise.all([initSkills(id), initMcpServers(id)]);
    };

    const handleOpenDir = async (e: MouseEvent, path: string) => {
        e.stopPropagation();
        try {
            await invoke('open_project_directory', { path });
        } catch (err) {
            console.warn('打开目录失败:', err);
        }
    };

    return (
        <>
            <div class="relative" ref={ref}>
                <button
                    class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-sm
                           bg-white/10 hover:bg-white/20 text-white/90 transition-colors
                           border border-white/10 hover:border-white/20"
                    onClick={() => setOpen(!open())}
                    title={currentProject() ? `项目: ${currentProject()!.name} (${currentProject()!.path})` : '选择工作目录'}
                >
                    <svg class="w-4 h-4 opacity-70" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                              d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
                    </svg>
                    <span class="max-w-[100px] truncate">{currentProject()?.name ?? '选择目录...'}</span>
                    <svg class="w-3 h-3 opacity-50" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7" />
                    </svg>
                </button>

                <Show when={open()}>
                    <div
                        class="absolute bottom-full left-0 mb-1 w-64 bg-[#1e2a3a] border border-white/15
                               rounded-lg shadow-xl z-50 overflow-hidden"
                        onClick={(e) => e.stopPropagation()}
                    >
                        {/* 项目列表 */}
                        <div class="max-h-64 overflow-y-auto">
                            <For each={projects()}>
                                {(proj) => (
                                    <button
                                        class="w-full flex items-center gap-2 px-4 py-2.5 text-sm text-white/80
                                               hover:bg-white/10 transition-colors group"
                                        classList={{ 'bg-white/5': currentProjectId() === proj.id }}
                                        onClick={() => selectProject(proj.id)}
                                    >
                                        <svg class="w-4 h-4 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                                  d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
                                        </svg>
                                        <span class="truncate flex-1 text-left">{proj.name}</span>
                                        <span
                                            class="opacity-0 group-hover:opacity-60 transition-opacity shrink-0"
                                            onClick={(e) => handleOpenDir(e, proj.path)}
                                            title="打开项目目录"
                                        >
                                            <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                                      d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h12a2 2 0 002-2V10a2 2 0 00-2-2h-4l-2-2H8a2 2 0 00-2 2v0" />
                                            </svg>
                                        </span>
                                    </button>
                                )}
                            </For>
                        </div>

                        <div class="border-t border-white/10" />

                        {/* 新建项目 */}
                        <button
                            class="w-full flex items-center gap-2 px-4 py-2.5 text-sm text-[#7c9abf]
                                   hover:bg-white/10 transition-colors"
                            onClick={() => { setShowCreate(true); setOpen(false); }}
                        >
                            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                      d="M12 4v16m8-8H4" />
                            </svg>
                            新建项目
                        </button>
                    </div>
                </Show>
            </div>

            {/* 点击外部关闭 */}
            <Show when={open()}>
                <div class="fixed inset-0 z-40" onClick={() => setOpen(false)} />
            </Show>

            <Show when={showCreate()}>
                <ProjectCreateModal
                    onClose={() => setShowCreate(false)}
                    onCreated={async (id) => {
                        setShowCreate(false);
                        await initProjects();
                        await selectProject(id);
                    }}
                />
            </Show>
        </>
    );
}
