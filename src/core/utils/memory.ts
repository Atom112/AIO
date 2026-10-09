// 记忆相关 Tauri 命令封装（与 commands/memory.rs、commands/embedding.rs、commands/rerank.rs、commands/model_download.rs 对应）
import { invoke } from '@tauri-apps/api/core';

/** 内置默认本地嵌入模型 id（随应用打包；与后端 core::model_registry::BUNDLED_EMBED_ID 保持一致） */
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

/** 导出全部记忆事实为 JSON 文件（不含向量），写入用户指定路径（dialog save 选取） */
export const memoryExportFile = (projectId: string, path: string) =>
  invoke<void>('memory_export_file', { projectId, path });

/** 从用户指定路径的 JSON 文件导入记忆事实（dialog open 选取；按 key 去重，向量待重建） */
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

/** 切换项目嵌入模型：后端会清空本项目全部向量并自动重建索引（带 reindex-progress 事件） */
export const memoryUpdateEmbedding = (projectId: string, config: MemoryEmbeddingConfig) =>
  invoke<MemoryStatus>('memory_update_embedding', { projectId, config });

/** 设置项目级重排配置（不涉及向量，不触发重建） */
export const memoryUpdateRerank = (projectId: string, config: MemoryRerankConfig) =>
  invoke<MemoryStatus>('memory_update_rerank', { projectId, config });

/** 手动触发一次合并整理（sleep-time consolidation；预算默认取全局配置） */
export const memoryConsolidate = (projectId: string) =>
  invoke<ConsolidationReport>('memory_consolidate', { projectId });

/** 抽取事实关系（P3-3 轻量知识图谱；幂等，已有关系的事实跳过） */
export const memoryGraphExtract = (projectId: string) =>
  invoke<GraphExtractReport>('memory_graph_extract', { projectId });

/** 重排连接测试（本地模型需已下载且全局启用） */
export const rerankTest = (config: MemoryRerankConfig) =>
  invoke<RerankTestResult>('rerank_test', { config });

/** 可下载模型清单（本地重排/嵌入模型） */
export const embeddingModelList = () => invoke<EmbeddingModelInfo[]>('embedding_model_list');

/** 下载模型（带 model-download-progress 事件；支持暂停/取消，见 pause/cancel 封装） */
export const embeddingModelDownload = (id: string) =>
  invoke<ModelDownloadOutcome>('embedding_model_download', { id });

/** 暂停进行中的模型下载（保留断点，可续传）；返回是否有进行中的下载 */
export const embeddingModelPauseDownload = (id: string) =>
  invoke<boolean>('embedding_model_pause_download', { id });

/** 取消模型下载（删除断点；无进行中下载时直接清理该模型断点文件）；返回是否有进行中的下载 */
export const embeddingModelCancelDownload = (id: string) =>
  invoke<boolean>('embedding_model_cancel_download', { id });

/** 删除已下载模型（内置模型为 no-op） */
export const embeddingModelDelete = (id: string) => invoke<void>('embedding_model_delete', { id });

/** 全局启用/停用某个嵌入模型（与供应商设置类似） */
export const embeddingModelSetEnabled = (id: string, enabled: boolean) =>
  invoke<EmbeddingModelInfo>('embedding_model_set_enabled', { id, enabled });

/** 当前 embed 模型安装目录 */
export const embeddingModelsGetDir = () => invoke<string>('embedding_models_get_dir');

/** 设置 embed 模型安装目录（空串恢复默认目录） */
export const embeddingModelsSetDir = (dir: string) =>
  invoke<string>('embedding_models_set_dir', { dir });

export type { MemoryEmbeddingConfig, MemoryRerankConfig };
