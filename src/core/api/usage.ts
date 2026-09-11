// 用量统计域 Tauri 命令封装（与 commands/usage.rs 对应）。
import { invoke } from '@tauri-apps/api/core';
import type { UsageSummary, UsageSummaryByModel } from '../types/usage';

/** 按天聚合的用量汇总（近 N 天）。 */
export const getUsageSummary = (days: number) =>
  invoke<UsageSummary[]>('get_usage_summary', { days });

/** 按模型聚合的用量汇总（近 N 天；date 指定单日过滤）。 */
export const getUsageSummaryByModel = (days: number, date?: string) =>
  invoke<UsageSummaryByModel[]>('get_usage_summary_by_model', { days, date: date ?? null });
