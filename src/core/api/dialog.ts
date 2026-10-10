// Dialog-domain Tauri command wrappers (backend-side native dialogs).
// The returned paths are session-approved on the backend for the privileged
// save/read commands (save_binary_file / memory_export_file / memory_import_file),
// so a path string crossing the IPC boundary alone is never accepted.
import { invoke } from '@tauri-apps/api/core';

/** File-type filter for the native dialog. */
export interface DialogFilter {
  name: string;
  extensions: string[];
}

/**
 * Open the backend-side native save dialog.
 * Returns the chosen absolute path (session-approved for the privileged write
 * commands) or null when the user cancelled.
 */
export const pickSavePath = (defaultName: string, filter?: DialogFilter) =>
  invoke<string | null>('pick_save_path', {
    defaultName,
    filterName: filter?.name ?? null,
    extensions: filter?.extensions ?? null,
  });

/**
 * Open the backend-side native open dialog.
 * Returns the chosen absolute path (session-approved for the read commands)
 * or null when the user cancelled.
 */
export const pickOpenPath = (filter?: DialogFilter) =>
  invoke<string | null>('pick_open_path', {
    filterName: filter?.name ?? null,
    extensions: filter?.extensions ?? null,
  });
