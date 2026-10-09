// Usage statistics types (mirror the return structures of backend commands/usage.rs).
// Domain types belong in core/types.

/** Usage aggregated by day (Rust UsageSummary) */
export interface UsageSummary {
  date: string;
  inputTokens: number;
  outputTokens: number;
  cachedInputTokens: number;
  requestCount: number;
}

/** Usage aggregated by model (Rust UsageSummaryByModel) */
export interface UsageSummaryByModel {
  modelId: string;
  inputTokens: number;
  outputTokens: number;
  cachedInputTokens: number;
  requestCount: number;
}
