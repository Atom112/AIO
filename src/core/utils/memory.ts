// 记忆相关 Tauri 命令封装（与 commands/memory.rs、commands/embedding.rs 对应）
import { invoke } from '@tauri-apps/api/core';
import type {
  EmbeddingDownloadProgress,
  EmbeddingTestResult,
  MemoryEmbeddingConfig,
  MemoryFact,
  MemoryListPage,
  MemoryStats,
  MemoryStatus,
  OllamaModelInfo,
  ScoredFact,
} from '../types/memory';

export const memoryGetStatus = (projectId: string) =>
  invoke<MemoryStatus>('memory_get_status', { projectId });

export const memorySearch = (projectId: string, query: string, category?: string, k?: number) =>
  invoke<ScoredFact[]>('memory_search', { projectId, query, category, k });

export const memoryList = (
  projectId: string,
  opts: { status?: string; category?: string; limit?: number; offset?: number } = {},
) => invoke<MemoryListPage>('memory_list', { projectId, ...opts });

export const memoryGet = (projectId: string, id: string) =>
  invoke<MemoryFact>('memory_get', { projectId, id });

export const memoryAdd = (
  projectId: string,
  args: { key?: string; content: string; category?: string; importance?: number },
) => invoke<MemoryFact>('memory_add', { projectId, ...args });

export const memoryUpdate = (
  projectId: string,
  id: string,
  args: { content?: string; category?: string },
) => invoke<MemoryFact>('memory_update', { projectId, id, ...args });

export const memoryDelete = (projectId: string, id: string) =>
  invoke<void>('memory_delete', { projectId, id });

export const memoryClear = (projectId: string) => invoke<void>('memory_clear', { projectId });

export const memorySetEnabled = (projectId: string, enabled: boolean) =>
  invoke<void>('memory_set_enabled', { projectId, enabled });

export const memoryResetEnabled = (projectId: string) =>
  invoke<void>('memory_reset_enabled', { projectId });

export const memoryStats = (projectId: string) =>
  invoke<MemoryStats>('memory_stats', { projectId });

export const memoryReindex = (projectId: string) =>
  invoke<[number, number]>('memory_reindex', { projectId });

export const embeddingSaveApiKey = (key: string) => invoke<void>('embedding_save_api_key', { key });

export const embeddingTest = (config: MemoryEmbeddingConfig) =>
  invoke<EmbeddingTestResult>('embedding_test', { config });

export const embeddingStatus = () =>
  invoke<{
    provider: string;
    model: string;
    configured: boolean;
    available: boolean;
    dimensions: number;
    enabled: boolean;
  }>('embedding_status');

export const embeddingListOllamaModels = (apiUrl?: string) =>
  invoke<OllamaModelInfo[]>('embedding_list_ollama_models', { apiUrl });

export const embeddingPullOllamaModel = (model: string, apiUrl?: string) =>
  invoke<string>('embedding_pull_ollama_model', { model, apiUrl });

export const embeddingDeleteOllamaModel = (model: string, apiUrl?: string) =>
  invoke<void>('embedding_delete_ollama_model', { model, apiUrl });

export type { EmbeddingDownloadProgress, MemoryEmbeddingConfig };
