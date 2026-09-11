// 项目域：项目列表 / 当前项目 / Git 分支与切换 —— 从 store.ts 拆出。
// 叶域：仅依赖 types 与 invoke，不依赖其他 store 域（聊天相关判定见 chat 域）。

import { createEffect, createSignal } from 'solid-js';
import { getGitBranch, listGitBranches, listProjects, switchGitBranch } from '../api/project';
import type { Project } from './types';

/** 当前活跃的项目 ID（null = 全局模式）。从 localStorage 持久化恢复。 */
const SAVED_PROJECT_KEY = 'aio-current-project-id';
const savedProjectId = (() => {
  try {
    return localStorage.getItem(SAVED_PROJECT_KEY);
  } catch {
    return null;
  }
})();

export const [currentProjectId, setCurrentProjectId] = createSignal<string | null>(savedProjectId);

/** 项目 ID 变更时自动持久化到 localStorage */
createEffect(() => {
  const id = currentProjectId();
  try {
    if (id) {
      localStorage.setItem(SAVED_PROJECT_KEY, id);
    } else {
      localStorage.removeItem(SAVED_PROJECT_KEY);
    }
  } catch {
    /* ignore */
  }
});

/** 上次 Agent 模式下使用的工作目录 ID（用于从对话模式切换到 Agent 时自动恢复） */
const LAST_AGENT_PROJECT_KEY = 'aio-last-agent-project-id';
export const getLastAgentProjectId = (): string | null => {
  try {
    return localStorage.getItem(LAST_AGENT_PROJECT_KEY);
  } catch {
    return null;
  }
};
export const saveLastAgentProjectId = (id: string | null) => {
  try {
    if (id) {
      localStorage.setItem(LAST_AGENT_PROJECT_KEY, id);
    } else {
      localStorage.removeItem(LAST_AGENT_PROJECT_KEY);
    }
  } catch {
    /* ignore */
  }
};

/** 所有项目列表 */
export const [projects, setProjects] = createSignal<Project[]>([]);

/** 当前活跃项目的完整对象（派生） */
export const currentProject = (): Project | null => {
  const id = currentProjectId();
  if (!id) return null;
  return projects().find((p) => p.id === id) ?? null;
};

/** 当前项目的 Git 分支名称（null = 非 git 仓库或尚未加载） */
export const [gitBranch, setGitBranch] = createSignal<string | null>(null);

// 当前项目变更时自动获取 Git 分支
createEffect(() => {
  const project = currentProject();
  if (project?.path) {
    getGitBranch(project.path)
      .then((branch) => setGitBranch(branch ?? null))
      .catch(() => setGitBranch(null));
  } else {
    setGitBranch(null);
  }
});

/** 当前项目的所有 Git 本地分支列表（null = 非 git 仓库或尚未加载） */
export const [gitBranches, setGitBranches] = createSignal<string[] | null>(null);

// 当前项目变更时自动获取 Git 分支列表
createEffect(() => {
  const project = currentProject();
  if (project?.path) {
    listGitBranches(project.path)
      .then((branches) => setGitBranches(branches ?? null))
      .catch(() => setGitBranches(null));
  } else {
    setGitBranches(null);
  }
});

/** 切换到指定 Git 分支并更新本地状态 */
export const switchBranch = async (branchName: string): Promise<void> => {
  const project = currentProject();
  if (!project?.path) return;
  try {
    await switchGitBranch(project.path, branchName);
    // 切换成功后，立即更新当前分支名（不需要等待下一个 effect 周期）
    setGitBranch(branchName);
  } catch (e) {
    // 切换失败时，通过控制台或 UI 反馈错误
    console.error('切换分支失败:', e);
    throw e; // 让调用方处理显示
  }
};

/** 加载项目列表，恢复后验证持久化的项目 ID 仍然有效 */
export const initProjects = async () => {
  try {
    const list = await listProjects();
    setProjects(list);
    // 验证持久化的 projectId 是否仍存在，不存在则回退全局模式
    const saved = currentProjectId();
    if (saved && !list.find((p) => p.id === saved)) {
      setCurrentProjectId(null);
    }
  } catch (e) {
    console.warn('加载项目列表失败:', e);
  }
};

/** 键盘快捷键触发的"新建项目"弹窗信号 */
export const [showProjectCreateModal, setShowProjectCreateModal] = createSignal(false);
