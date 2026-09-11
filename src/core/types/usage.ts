// 用量统计类型（镜像后端 commands/usage.rs 的返回结构）。
// 此前误放在组件文件 UsageSummaryCards.tsx 中 —— 领域类型应属于 core/types。

/** 按天聚合的用量（对应 Rust UsageSummary） */
export interface UsageSummary {
  date: string;
  inputTokens: number;
  outputTokens: number;
  cachedInputTokens: number;
  requestCount: number;
}

/** 按模型聚合的用量（对应 Rust UsageSummaryByModel） */
export interface UsageSummaryByModel {
  modelId: string;
  inputTokens: number;
  outputTokens: number;
  cachedInputTokens: number;
  requestCount: number;
}
