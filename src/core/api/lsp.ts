// LSP domain Tauri command wrappers (mirrors commands/lsp.rs).
import { invoke } from '@tauri-apps/api/core';

/** Auto-detect language servers available for the project (returns { languages: [{ languageId }] }). */
export const autoDetectLsp = (projectPath: string) =>
  invoke<{ languages: { languageId: string }[] }>('auto_detect_ls', { projectPath });

/** Start the language server for a language. */
export const startLspServer = (projectPath: string, languageId: string) =>
  invoke<void>('start_lsp_server', { projectPath, languageId });
