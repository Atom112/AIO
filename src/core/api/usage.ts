// Usage stats domain Tauri command wrappers (mirrors commands/usage.rs).
import { invoke } from '@tauri-apps/api/core';
import type { UsageSummary, UsageSummaryByModel } from '../types/usage';

/** Daily usage summary (last N days). */
export const getUsageSummary = (days: number) =>
  invoke<UsageSummary[]>('get_usage_summary', { days });

/** Per-model usage summary (last N days; date filters a single day). */
export const getUsageSummaryByModel = (days: number, date?: string) =>
  invoke<UsageSummaryByModel[]>('get_usage_summary_by_model', { days, date: date ?? null });
