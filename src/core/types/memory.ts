// Frontend mirror types for the backend memory module (src-tauri/src/services/memory + commands/memory.rs + commands/embedding.rs)

/** Memory fact */
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
  /** Enclosing symbol name (symbol-aware chunking; empty for line-window chunks) */
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

/** Fact hit by retrieval */
export interface ScoredFact {
  id: string;
  content: string;
  category: string;
  score: number;
  sourceType?: string;
  updatedAt?: string;
}

/** Memory store statistics */
export interface MemoryStats {
  totalFacts: number;
  activeFacts: number;
  archivedFacts: number;
  supersededFacts: number;
  embeddedFacts: number;
  dbSizeBytes: number;
  lastReindexAt?: string;
  /** Last 7 days: retrieval query count */
  retrievalQueries7d: number;
  /** Last 7 days: retrieval hit count */
  retrievalHits7d: number;
  /** Last 7 days: dedup trigger count */
  dedupTriggers7d: number;
  /** Last 7 days: facts persisted by auto-extraction */
  extracted7d: number;
  /** Last 7 days: LLM arbitration action distribution (merge/update/supersede/keep_separate) */
  judgeDistribution7d: Record<string, number>;
}

/** Consolidation report (sleep-time consolidation) */
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

/** Graph extraction report (lightweight knowledge graph) */
export interface GraphExtractReport {
  factsProcessed: number;
  relations: number;
  failed: number;
  error?: string;
}

/** Embedder readiness status */
export interface EmbedderStatus {
  provider: string;
  model: string;
  configured: boolean;
  available: boolean;
  dimensions: number;
}

/** Project memory status */
export interface MemoryStatus {
  enabled: boolean;
  storePath: string;
  stats: MemoryStats;
  embedder: EmbedderStatus;
  /** Current embedding config for this project (project-level, stored in memory store meta) */
  embedding: MemoryEmbeddingConfig;
  /** Current rerank config for the project (project-level; only the enabled/provider/model fields) */
  rerank: MemoryRerankConfig;
}

/** Memory list page */
export interface MemoryListPage {
  facts: MemoryFact[];
  total: number;
}

/** Embedding config (mirrors backend MemoryEmbeddingConfig; since v0.8 only local models are supported, provider is always local) */
export interface MemoryEmbeddingConfig {
  /** Local embedding model provider (the only supported one; legacy online values are auto-normalized on the backend) */
  provider: 'local' | 'online';
  /** Local embedding model id (manifest entry id, e.g. bge-small-en-v1.5 / all-MiniLM-L6-v2) */
  model: string;
  /** Deprecated (online service URL); always empty, kept only for legacy config compatibility */
  apiUrl: string;
  /** Target dimensions (0 = model default full dims; truncatable for Matryoshka models like bge-m3) */
  dimensions: number;
  enabled: boolean;
  apiKey?: string;
}

/** Embedding connection test result */
export interface EmbeddingTestResult {
  ok: boolean;
  provider: string;
  model: string;
  dimensions: number;
  latencyMs: number;
  error?: string;
}

/** Rerank config (mirrors backend MemoryRerankConfig; since v0.8.1 only local models are supported, provider is always local) */
export interface MemoryRerankConfig {
  enabled: boolean;
  /** Always local (legacy online values are auto-normalized on the backend) */
  provider: 'local' | 'online';
  /** Local rerank model id (manifest entry id, e.g. bge-reranker-base / bge-reranker-v2-m3) */
  model: string;
}

/** Rerank connection test result */
export interface RerankTestResult {
  ok: boolean;
  provider: string;
  model: string;
  latencyMs: number;
  error?: string;
}

/** Downloadable/installed model info (local rerank/embedding models) */
export interface EmbeddingModelInfo {
  id: string;
  /** embed | rerank */
  kind: string;
  label: string;
  /** Vector dimensions (0 for rerank models) */
  dims: number;
  /** mean | cls | none */
  pooled: string;
  /** Bundled model (shipped with the app, no download needed, cannot be deleted) */
  bundled: boolean;
  /** Global enablement state (both embedding and rerank models can be toggled globally) */
  enabled: boolean;
  files: { name: string; downloaded: boolean }[];
  downloaded: boolean;
  /** Disk space used by downloaded files (bytes) */
  sizeBytes: number;
}

/** Model download outcome (status: ok = finished | paused = paused with checkpoint kept | cancelled = cancelled and checkpoint deleted) */
export interface ModelDownloadOutcome {
  status: 'ok' | 'paused' | 'cancelled';
  info: EmbeddingModelInfo;
}
