// 记忆相关 Tauri 命令封装（与 commands/memory.rs、commands/embedding.rs 对应）
import { invoke } from '@tauri-apps/api/core';
import type {
  CodeChunkHit,
  CodeIndexResult,
  CodeIndexStatus,
  EmbeddingTestResult,
  FactVersion,
  MemoryEmbeddingConfig,
  MemoryFact,
  MemoryListPage,
  MemoryStats,
  MemoryStatus,
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

export const memoryGetVersions = (projectId: string, id: string) =>
  invoke<FactVersion[]>('memory_get_versions', { projectId, id });

export const memoryMergeFacts = (
  projectId: string,
  targetId: string,
  sourceId: string,
  mergedContent: string,
) => invoke<MemoryFact>('memory_merge_facts', { projectId, targetId, sourceId, mergedContent });

export const memoryArchive = (projectId: string, id: string) =>
  invoke<MemoryFact>('memory_archive', { projectId, id });

export const memorySetPinned = (projectId: string, id: string, pinned: boolean) =>
  invoke<MemoryFact>('memory_set_pinned', { projectId, id, pinned });

export const memoryPrune = (projectId: string) => invoke<number>('memory_prune', { projectId });

export const memoryExport = (projectId: string) => invoke<string>('memory_export', { projectId });

export const memoryImport = (projectId: string, json: string) =>
  invoke<number>('memory_import', { projectId, json });

export const memoryCodeIndex = (projectId: string, force?: boolean) =>
  invoke<CodeIndexResult>('memory_code_index', { projectId, force });

export const memoryCodeSearch = (projectId: string, query: string, k?: number) =>
  invoke<CodeChunkHit[]>('memory_code_search', { projectId, query, k });

export const memoryCodeStatus = (projectId: string) =>
  invoke<CodeIndexStatus>('memory_code_status', { projectId });

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

export const embeddingSaveApiKey = (key: string) => invoke<void>('embedding_save_api_key', { key });

export type { MemoryEmbeddingConfig };
