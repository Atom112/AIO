// Project and Git domain Tauri command wrappers (mirrors commands/project.rs and commands/git.rs).
import { invoke } from '@tauri-apps/api/core';
import type { Project } from '../store/types';

/** List all projects. */
export const listProjects = () => invoke<Project[]>('list_projects');

/** Read the project's current Git branch (null if not a git repo). */
export const getGitBranch = (projectPath: string) =>
  invoke<string | null>('get_git_branch', { projectPath });

/** List the project's local Git branches. */
export const listGitBranches = (projectPath: string) =>
  invoke<string[]>('list_git_branches', { projectPath });

/** Switch to the given Git branch. */
export const switchGitBranch = (projectPath: string, branchName: string) =>
  invoke<string>('switch_git_branch', { projectPath, branchName });

/** Revert a batch of file changes (restores the fileChanges recorded by an Agent step). */
export const revertFileChangesBatch = (projectPath: string, filePaths: string[]) =>
  invoke<void>('revert_file_changes_batch', { projectPath, filePaths });

/** Revert a single file change. */
export const revertFileChange = (projectPath: string, filePath: string) =>
  invoke<void>('revert_file_change', { projectPath, filePath });

/** Validate and canonicalize a project path (returns validity + canonical path). */
export const validateProjectPath = (path: string) =>
  invoke<{ valid: boolean; reason: string; canonicalPath: string }>('validate_project_path', {
    path,
  });

/** Create a project and return it. */
export const createProject = (name: string, path: string) =>
  invoke<Project>('create_project', { name, path });

/** Update a project (e.g. rename). */
export const updateProject = (id: string, name: string) =>
  invoke<void>('update_project', { id, name });

/** Delete a project. */
export const deleteProject = (id: string) => invoke<void>('delete_project', { id });

/** Open the project directory in the system file manager. */
export const openProjectDirectory = (path: string) =>
  invoke<void>('open_project_directory', { path });
