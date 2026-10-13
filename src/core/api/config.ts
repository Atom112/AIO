// App config domain Tauri command wrappers (mirrors commands/config.rs).
import { invoke } from '@tauri-apps/api/core';
import type { ActivatedModel, AppConfig } from '../store/types';

/** Load the app config (the caller decides the fallback strategy on failure). */
export const loadAppConfig = () => invoke<AppConfig>('load_app_config');

/** FE-07: partial app-config patch (merged atomically backend-side in `patch_app_config`). */
export interface AppConfigPatch {
  maxToolRounds?: number;
  maxConcurrentSubagents?: number;
  subagentIsolationMode?: string;
  /** Main agent built-in tool access: "hybrid" | "full" | "orchestrator" */
  mainAgentToolAccess?: string;
}

/**
 * Merge a partial app-config update in one backend call (load → merge → atomic save).
 * Replaces the settings page's former load→modify→save round-trip per keystroke.
 */
export const patchAppConfig = (patch: AppConfigPatch) =>
  invoke<AppConfig>('patch_app_config', { patch });

/** Save the app config. */
export const saveAppConfig = (config: AppConfig) =>
  invoke<void>('save_app_config', { config: config as unknown as Record<string, unknown> });

/** Sync the frontend locale to the backend (language of user-visible text such as error messages). */
export const setBackendLocale = (locale: string) => invoke<void>('set_locale', { locale });

/** Set OS auto-start (true = launch with the system). */
export const setAutoStart = (enabled: boolean) => invoke<void>('set_auto_start', { enabled });

/** Load the activated model list (includes entries injected by local engines). */
export const loadActivatedModels = () => invoke<ActivatedModel[]>('load_activated_models');

/** Load the provider config file (includes custom providers and models). */
export const loadProviderConfigs = () =>
  invoke<{ providers: Record<string, import('../utils/models').ProviderConfig> }>(
    'load_provider_configs',
  );
