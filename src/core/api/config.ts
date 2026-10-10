// App config domain Tauri command wrappers (mirrors commands/config.rs).
import { invoke } from '@tauri-apps/api/core';
import type { ActivatedModel, AppConfig } from '../store/types';

/** Load the app config (the caller decides the fallback strategy on failure). */
export const loadAppConfig = () => invoke<AppConfig>('load_app_config');

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
