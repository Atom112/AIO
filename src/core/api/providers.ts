// 供应商（provider）域 Tauri 命令封装（与 commands/provider_config.rs 对应）。
import { invoke } from '@tauri-apps/api/core';
import type { ProviderConfig, ProviderConfigFile, TestConnectionResult } from '../utils/models';

/** 供应商连接/模型拉取的目标参数。 */
export interface ProviderEndpoint {
  apiUrl: string;
  apiKey: string;
  proxyUrl: string | null;
  /** 本地引擎类型（llama_cpp/ollama/vllm）；云端为 null */
  engineType: string | null;
}

/** 按 providers map 持久化（补全 version/updatedAt 封套；3 处调用方此前各自拼装）。 */
export const saveProviderMap = (providers: Record<string, ProviderConfig>) =>
  saveProviderConfigs({
    version: 2,
    updatedAt: String(Date.now()),
    providers,
  });

/** 保存 provider 配置文件（整文件覆盖）。 */
export const saveProviderConfigs = (file: ProviderConfigFile) =>
  invoke<void>('save_provider_configs', { file: file as unknown as Record<string, unknown> });

/** 测试供应商连接（返回模型数 + 延迟）。 */
export const testProviderConnection = (endpoint: ProviderEndpoint) =>
  invoke<TestConnectionResult>('test_provider_connection', {
    ...endpoint,
  });

/** 拉取供应商可用模型列表。 */
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
