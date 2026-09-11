// LSP 域 Tauri 命令封装（与 commands/lsp.rs 对应）。
import { invoke } from '@tauri-apps/api/core';

/** 自动检测项目可用的语言服务器（返回 { languages: [{ languageId }] }）。 */
export const autoDetectLsp = (projectPath: string) =>
  invoke<{ languages: { languageId: string }[] }>('auto_detect_ls', { projectPath });

/** 为某语言启动语言服务器。 */
export const startLspServer = (projectPath: string, languageId: string) =>
  invoke<void>('start_lsp_server', { projectPath, languageId });
