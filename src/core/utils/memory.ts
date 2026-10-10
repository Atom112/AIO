// Tauri command wrappers for memory (mirrors commands/memory.rs, commands/embedding.rs, commands/rerank.rs, commands/model_download.rs)
import { invoke } from '@tauri-apps/api/core';

/** Bundled default local embedding model id (shipped with the app; kept in sync with backend core::model_registry::BUNDLED_EMBED_ID) */
export const DEFAULT_EMBED_MODEL_ID = 'all-MiniLM-L6-v2';
import type {
  CodeChunkHit,
  CodeIndexResult,
  CodeIndexStatus,
  ConsolidationReport,
  EmbeddingModelInfo,
  EmbeddingTestResult,
  FactVersion,
  GraphExtractReport,
  MemoryEmbeddingConfig,
  MemoryFact,
  MemoryListPage,
  MemoryRerankConfig,
  MemoryStats,
  MemoryStatus,
  ModelDownloadOutcome,
  RerankTestResult,
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

export const memoryArchive = (projectId: string, id: string) =>
  invoke<MemoryFact>('memory_archive', { projectId, id });

export const memorySetPinned = (projectId: string, id: string, pinned: boolean) =>
  invoke<MemoryFact>('memory_set_pinned', { projectId, id, pinned });

export const memoryPrune = (projectId: string) => invoke<number>('memory_prune', { projectId });

/** Exports all memory facts as a JSON file (vectors excluded), written to a user-chosen path (via dialog save) */
export const memoryExportFile = (projectId: string, path: string) =>
  invoke<void>('memory_export_file', { projectId, path });

/** Imports memory facts from a user-chosen JSON file (via dialog open; deduplicated by key, vectors rebuilt later) */
export const memoryImportFile = (projectId: string, path: string) =>
  invoke<number>('memory_import_file', { projectId, path });

export const memoryCodeIndex = (projectId: string, force?: boolean) =>
  invoke<CodeIndexResult>('memory_code_index', { projectId, force });

export const memoryCodeSearch = (projectId: string, query: string, k?: number) =>
  invoke<CodeChunkHit[]>('memory_code_search', { projectId, query, k });

export const memoryCodeStatus = (projectId: string) =>
  invoke<CodeIndexStatus>('memory_code_status', { projectId });

export const embeddingTest = (config: MemoryEmbeddingConfig) =>
  invoke<EmbeddingTestResult>('embedding_test', { config });

/** Switches the project embedding model: the backend clears all project vectors and rebuilds the index automatically (emits reindex-progress events) */
export const memoryUpdateEmbedding = (projectId: string, config: MemoryEmbeddingConfig) =>
  invoke<MemoryStatus>('memory_update_embedding', { projectId, config });

/** Sets the project-level rerank config (no vectors involved, no rebuild triggered) */
export const memoryUpdateRerank = (projectId: string, config: MemoryRerankConfig) =>
  invoke<MemoryStatus>('memory_update_rerank', { projectId, config });

/** Manually triggers one consolidation run (sleep-time consolidation; budget defaults to the global config) */
export const memoryConsolidate = (projectId: string) =>
  invoke<ConsolidationReport>('memory_consolidate', { projectId });

/** Extracts fact relations for the lightweight knowledge graph (idempotent; facts with existing relations are skipped) */
export const memoryGraphExtract = (projectId: string) =>
  invoke<GraphExtractReport>('memory_graph_extract', { projectId });

/** Rerank connection test (local models must be downloaded and globally enabled) */
export const rerankTest = (config: MemoryRerankConfig) =>
  invoke<RerankTestResult>('rerank_test', { config });

/** Downloadable model list (local rerank/embedding models) */
export const embeddingModelList = () => invoke<EmbeddingModelInfo[]>('embedding_model_list');

/** Downloads a model (emits model-download-progress events; supports pause/cancel, see the pause/cancel wrappers) */
export const embeddingModelDownload = (id: string) =>
  invoke<ModelDownloadOutcome>('embedding_model_download', { id });

/** Pauses an in-progress model download (keeps the checkpoint for resume); returns whether a download was in progress */
export const embeddingModelPauseDownload = (id: string) =>
  invoke<boolean>('embedding_model_pause_download', { id });

/** Cancels a model download (deletes the checkpoint; with no active download, just cleans up the model's checkpoint file); returns whether a download was in progress */
export const embeddingModelCancelDownload = (id: string) =>
  invoke<boolean>('embedding_model_cancel_download', { id });

/** Deletes a downloaded model (no-op for bundled models) */
export const embeddingModelDelete = (id: string) => invoke<void>('embedding_model_delete', { id });

/** Globally enables/disables an embedding model (similar to provider settings) */
export const embeddingModelSetEnabled = (id: string, enabled: boolean) =>
  invoke<EmbeddingModelInfo>('embedding_model_set_enabled', { id, enabled });

/** Current embed model install directory */
export const embeddingModelsGetDir = () => invoke<string>('embedding_models_get_dir');

/** Sets the embed model install directory (empty string restores the default directory) */
export const embeddingModelsSetDir = (dir: string) =>
  invoke<string>('embedding_models_set_dir', { dir });

export type { MemoryEmbeddingConfig, MemoryRerankConfig };
