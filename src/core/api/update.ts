// App update domain Tauri command wrappers (mirrors commands/update.rs).
// Single source of truth for CheckUpdateResult.
import { invoke } from '@tauri-apps/api/core';

/** Available update info (mirrors AppUpdateInfo in commands/update.rs). */
export interface AppUpdateInfo {
  /** Latest version (v prefix stripped) */
  version: string;
  currentVersion: string;
  /** Release notes (raw Markdown) */
  notes?: string;
  /** Publication time (ISO 8601) */
  pubDate?: string;
}

/**
 * Result of check_app_update (mirrors CheckUpdateResult in commands/update.rs).
 * Discriminant field kind: up_to_date / update_available / service_not_ready / network / failed.
 */
export type CheckUpdateResult =
  | { kind: 'up_to_date'; current_version: string }
  | { kind: 'update_available'; info: AppUpdateInfo }
  | { kind: 'service_not_ready'; current_version: string; endpoint: string; reason: string }
  | { kind: 'network'; current_version: string; endpoint: string; reason: string }
  | { kind: 'failed'; current_version: string; endpoint: string; reason: string };

/** Check for app updates. */
export const checkAppUpdate = () => invoke<CheckUpdateResult>('check_app_update');

/** Download and install the update (progress reported via the app-update-progress event). */
export const installAppUpdate = () => invoke<void>('install_app_update');

/** Restart the app (after installing an update). */
export const restartApp = () => invoke<void>('restart_app');
