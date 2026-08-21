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
}

/** 记忆列表分页 */
export interface MemoryListPage {
  facts: MemoryFact[];
  total: number;
}

/** 嵌入配置（与后端 MemoryEmbeddingConfig 对应） */
export interface MemoryEmbeddingConfig {
  provider: 'ollama' | 'openai_compat';
  model: string;
  apiUrl: string;
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

/** Ollama 模型信息 */
export interface OllamaModelInfo {
  name: string;
  sizeBytes: number;
  digest?: string;
}

/** embedding-download-progress 事件负载 */
export interface EmbeddingDownloadProgress {
  provider: string;
  model: string;
  status: string;
  completed?: number;
  total?: number;
  digest?: string;
  error?: string;
}
