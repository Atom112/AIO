// Appearance domain Tauri command wrappers (mirrors commands/appearance.rs).
import { invoke } from '@tauri-apps/api/core';

/** Copy a picked image into the app data backgrounds directory; returns the stored absolute path. */
export const saveAppBackground = (path: string) => invoke<string>('save_app_background', { path });

/** Delete a stored background image (best-effort; ignores empty/missing paths). */
export const deleteAppBackground = (path: string) =>
  invoke<void>('delete_app_background', { path });
