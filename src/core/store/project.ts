// Project domain: project list / current project / Git branch and switching.
// Leaf domain: depends only on types and invoke, not on other store domains (chat-related checks live in the chat domain).

import { createEffect, createSignal } from 'solid-js';
import { getGitBranch, listGitBranches, listProjects, switchGitBranch } from '../api/project';
import type { Project } from './types';

/** Active project ID (null = global mode). Restored from localStorage. */
const SAVED_PROJECT_KEY = 'aio-current-project-id';
const savedProjectId = (() => {
  try {
    return localStorage.getItem(SAVED_PROJECT_KEY);
  } catch {
    return null;
  }
})();

export const [currentProjectId, setCurrentProjectId] = createSignal<string | null>(savedProjectId);

/** Auto-persist the project ID to localStorage whenever it changes. */
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

/** Working-directory ID last used in Agent mode (auto-restored when switching from chat mode to Agent). */
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

/** Full list of all projects. */
export const [projects, setProjects] = createSignal<Project[]>([]);

/** Full object of the active project (derived). */
export const currentProject = (): Project | null => {
  const id = currentProjectId();
  if (!id) return null;
  return projects().find((p) => p.id === id) ?? null;
};

/** Git branch name of the current project (null = not a git repo or not loaded yet). */
export const [gitBranch, setGitBranch] = createSignal<string | null>(null);

// Fetch the Git branch automatically when the current project changes.
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

/** All local Git branches of the current project (null = not a git repo or not loaded yet). */
export const [gitBranches, setGitBranches] = createSignal<string[] | null>(null);

// Fetch the Git branch list automatically when the current project changes.
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

/** Switch to the given Git branch and update local state. */
export const switchBranch = async (branchName: string): Promise<void> => {
  const project = currentProject();
  if (!project?.path) return;
  try {
    await switchGitBranch(project.path, branchName);
    // On success, update the current branch name immediately (no need to wait for the next effect cycle).
    setGitBranch(branchName);
  } catch (e) {
    // On failure, surface the error via console or UI feedback.
    console.error('Failed to switch branch:', e);
    throw e; // rethrow so the caller handles display
  }
};

/** Load the project list, then verify the persisted project ID is still valid. */
export const initProjects = async () => {
  try {
    const list = await listProjects();
    setProjects(list);
    // Verify the persisted projectId still exists; fall back to global mode if not.
    const saved = currentProjectId();
    if (saved && !list.find((p) => p.id === saved)) {
      setCurrentProjectId(null);
    }
  } catch (e) {
    console.warn('Failed to load project list:', e);
  }
};

/** Signal for the "new project" modal triggered by a keyboard shortcut. */
export const [showProjectCreateModal, setShowProjectCreateModal] = createSignal(false);
