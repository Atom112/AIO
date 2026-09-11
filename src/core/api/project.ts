// 项目与 Git 域 Tauri 命令封装（与 commands/project.rs、commands/git.rs 对应）。
import { invoke } from '@tauri-apps/api/core';
import type { Project } from '../store/types';

/** 列出所有项目。 */
export const listProjects = () => invoke<Project[]>('list_projects');

/** 读取项目当前 Git 分支（非 git 仓库返回 null）。 */
export const getGitBranch = (projectPath: string) =>
  invoke<string | null>('get_git_branch', { projectPath });

/** 列出项目本地 Git 分支。 */
export const listGitBranches = (projectPath: string) =>
  invoke<string[]>('list_git_branches', { projectPath });

/** 切换到指定 Git 分支。 */
export const switchGitBranch = (projectPath: string, branchName: string) =>
  invoke<string>('switch_git_branch', { projectPath, branchName });

/** 批量撤销一组文件变更（按 Agent 步骤记录的 fileChanges 还原）。 */
export const revertFileChangesBatch = (projectPath: string, filePaths: string[]) =>
  invoke<void>('revert_file_changes_batch', { projectPath, filePaths });

/** 撤销单个文件变更。 */
export const revertFileChange = (projectPath: string, filePath: string) =>
  invoke<void>('revert_file_change', { projectPath, filePath });

/** 校验并规范化项目路径（返回是否可用 + 规范化路径）。 */
export const validateProjectPath = (path: string) =>
  invoke<{ valid: boolean; reason: string; canonicalPath: string }>('validate_project_path', {
    path,
  });

/** 新建项目，返回创建的项目。 */
export const createProject = (name: string, path: string) =>
  invoke<Project>('create_project', { name, path });

/** 更新项目（改名等）。 */
export const updateProject = (id: string, name: string) =>
  invoke<void>('update_project', { id, name });

/** 删除项目。 */
export const deleteProject = (id: string) => invoke<void>('delete_project', { id });

/** 在系统文件管理器中打开项目目录。 */
export const openProjectDirectory = (path: string) =>
  invoke<void>('open_project_directory', { path });
