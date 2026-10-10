// Provider domain Tauri command wrappers (mirrors commands/provider_config.rs).
import { invoke } from '@tauri-apps/api/core';
import type { ProviderConfig, ProviderConfigFile, TestConnectionResult } from '../utils/models';

/** Target parameters for provider connection tests and model fetching. */
export interface ProviderEndpoint {
  apiUrl: string;
  apiKey: string;
  proxyUrl: string | null;
  /** Local engine type (llama_cpp/ollama/vllm); null for cloud */
  engineType: string | null;
}

/** Persist by providers map (fills in the version/updatedAt envelope). */
export const saveProviderMap = (providers: Record<string, ProviderConfig>) =>
  saveProviderConfigs({
    version: 2,
    updatedAt: String(Date.now()),
    providers,
  });

/** Save the provider config file (whole-file overwrite). */
export const saveProviderConfigs = (file: ProviderConfigFile) =>
  invoke<void>('save_provider_configs', { file: file as unknown as Record<string, unknown> });

/** Test a provider connection (returns model count + latency). */
export const testProviderConnection = (endpoint: ProviderEndpoint) =>
  invoke<TestConnectionResult>('test_provider_connection', {
    ...endpoint,
  });

/** Fetch the provider's available model list. */
export const fetchProviderModels = (endpoint: ProviderEndpoint) =>
  invoke<{
    success: boolean;
    models: Array<{
      id: string;
      owned_by: string;
      display_name?: string;
      released_at?: string;
    }>;
    error: string | null;
    elapsedMs: number;
  }>('fetch_provider_models', { ...endpoint });
