// 后端记忆模块的类型镜像（src-tauri/src/services/memory + commands/memory.rs + commands/embedding.rs）

/** 记忆事实 */
export interface MemoryFact {
  id: string;
  key?: string;
  content: string;
  category: string;
  importance: number;
  status: 'active' | 'superseded' | 'archived';
  embeddingModel?: string;
  sourceType?: string;
  sourceRefs?: string;
  createdAt: string;
  updatedAt: string;
  accessCount: number;
  pinned: boolean;
}

export interface CodeIndexResult {
  files: number;
  chunks: number;
  embedded: number;
  failed: number;
  skipped: number;
  elapsedMs: number;
  error?: string;
}

export interface CodeChunkHit {
  chunkId: string;
  filePath: string;
  startLine: number;
  endLine: number;
  /** 所在符号名（符号感知分块；行窗口分块为空） */
  symbol?: string;
  content: string;
  score: number;
}

export interface CodeIndexStatus {
  chunks: number;
  files: number;
  indexedFiles: number;
  lastIndexAt?: string;
}
export interface FactVersion {
  id: number;
  factId: string;
  version: number;
  reason: string;
  contentBefore?: string;
  contentAfter?: string;
  relatedFactIds?: string;
  createdAt: string;
}

/** 检索命中的事实 */
export interface ScoredFact {
  id: string;
  content: string;
  category: string;
  score: number;
  sourceType?: string;
  updatedAt?: string;
}

/** 记忆库统计 */
export interface MemoryStats {
  totalFacts: number;
  activeFacts: number;
  archivedFacts: number;
  supersededFacts: number;
  embeddedFacts: number;
  dbSizeBytes: number;
  lastReindexAt?: string;
  /** 近 7 天：检索查询次数 */
  retrievalQueries7d: number;
  /** 近 7 天：检索命中条数 */
  retrievalHits7d: number;
  /** 近 7 天：判重触发次数 */
  dedupTriggers7d: number;
  /** 近 7 天：自动提取沉淀事实数 */
  extracted7d: number;
  /** 近 7 天：LLM 仲裁动作分布（merge/update/supersede/keep_separate） */
  judgeDistribution7d: Record<string, number>;
}

/** 合并整理结果（sleep-time consolidation） */
export interface ConsolidationReport {
  clusters: number;
  arbitrated: number;
  merged: number;
  updated: number;
  superseded: number;
  kept: number;
  budgetUsed: number;
  error?: string;
}

/** 图谱抽取结果（P3-3 轻量知识图谱） */
export interface GraphExtractReport {
  factsProcessed: number;
  relations: number;
  failed: number;
  error?: string;
}

/** 嵌入器就绪状态 */
export interface EmbedderStatus {
  provider: string;
  model: string;
  configured: boolean;
  available: boolean;
  dimensions: number;
}

/** 项目记忆状态 */
export interface MemoryStatus {
  enabled: boolean;
  storePath: string;
  stats: MemoryStats;
  embedder: EmbedderStatus;
  /** 本项目当前嵌入配置（项目级，存于记忆库 meta） */
  embedding: MemoryEmbeddingConfig;
  /** 项目当前重排配置（项目级；仅 enabled/provider/model 三个字段） */
  rerank: MemoryRerankConfig;
}

/** 记忆列表分页 */
export interface MemoryListPage {
  facts: MemoryFact[];
  total: number;
}

/** 嵌入配置（与后端 MemoryEmbeddingConfig 对应；v0.8 起仅支持本地模型，provider 恒为 local） */
export interface MemoryEmbeddingConfig {
  /** 本地嵌入模型（唯一受支持的 provider；旧配置中的 online 会在后端自动归一化） */
  provider: 'local' | 'online';
  /** 本地嵌入模型 id（manifest 条目 id，如 bge-small-en-v1.5 / all-MiniLM-L6-v2） */
  model: string;
  /** 已废弃（在线服务地址），恒为空，保留仅为旧配置兼容 */
  apiUrl: string;
  /** 目标维度（0=模型默认全维；对 bge-m3 等 Matryoshka 模型可裁剪） */
  dimensions: number;
  enabled: boolean;
  apiKey?: string;
}

/** 嵌入连接测试结果 */
export interface EmbeddingTestResult {
  ok: boolean;
  provider: string;
  model: string;
  dimensions: number;
  latencyMs: number;
  error?: string;
}

/** 重排配置（与后端 MemoryRerankConfig 对应；自 v0.8.1 起仅支持本地模型，provider 恒为 local） */
export interface MemoryRerankConfig {
  enabled: boolean;
  /** 恒为 local（旧配置中的 online 在后端自动归一化） */
  provider: 'local' | 'online';
  /** 本地重排模型 id（manifest 条目 id，如 bge-reranker-base / bge-reranker-v2-m3） */
  model: string;
}

/** 重排连接测试结果 */
export interface RerankTestResult {
  ok: boolean;
  provider: string;
  model: string;
  latencyMs: number;
  error?: string;
}

/** 可下载/已安装模型信息（本地重排/嵌入模型） */
export interface EmbeddingModelInfo {
  id: string;
  /** embed | rerank */
  kind: string;
  label: string;
  /** 向量维度（rerank 为 0） */
  dims: number;
  /** mean | cls | none */
  pooled: string;
  /** 内置模型（随应用打包，无需下载，不可删除） */
  bundled: boolean;
  /** 全局启用状态（嵌入与重排模型均可全局启停） */
  enabled: boolean;
  files: { name: string; downloaded: boolean }[];
  downloaded: boolean;
  /** 已下载文件占用空间（字节） */
  sizeBytes: number;
}

/** 模型下载结果（status: ok=完成 | paused=已暂停保留断点 | cancelled=已取消断点已删） */
export interface ModelDownloadOutcome {
  status: 'ok' | 'paused' | 'cancelled';
  info: EmbeddingModelInfo;
}
