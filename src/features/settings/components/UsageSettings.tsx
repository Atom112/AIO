/**
 * UsageSettings — 使用量子页面
 *
 * 组合：时间范围选择器 + 统计卡片 + 热力图(固定365天) + 模型分布
 */
import { Component, createSignal, createEffect, createMemo, Show, For } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import UsageSummaryCards from './UsageSummaryCards';
import type { UsageSummary, UsageSummaryByModel } from './UsageSummaryCards';
import UsageHeatmap from './UsageHeatmap';
import ModelBreakdown from './ModelBreakdown';
import Icon from '../../../shared/components/Icon';
import { reportError, t, type TranslationKey } from '../../../core/i18n';

type Range = { labelKey: TranslationKey; days: number };

const RANGES: Range[] = [
  { labelKey: 'usage.range7', days: 7 },
  { labelKey: 'usage.range30', days: 30 },
  { labelKey: 'usage.range90', days: 90 },
  { labelKey: 'usage.rangeAll', days: 365 },
];

const UsageSettings: Component = () => {
  const [range, setRange] = createSignal<Range>(RANGES[1]); // 默认 30 天
  const [summary, setSummary] = createSignal<UsageSummary[]>([]);
  const [byModel, setByModel] = createSignal<UsageSummaryByModel[]>([]);
  // 热力图固定加载 365 天数据，不受时间范围影响
  const [heatmapSummary, setHeatmapSummary] = createSignal<UsageSummary[]>([]);
  const [selectedDate, setSelectedDate] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);

  // 加载受时间范围影响的数据
  const fetchRangeData = async (days: number) => {
    setLoading(true);
    setError(null);
    try {
      const [s, m] = await Promise.all([
        invoke<UsageSummary[]>('get_usage_summary', { days }),
        invoke<UsageSummaryByModel[]>('get_usage_summary_by_model', { days }),
      ]);
      setSummary(s);
      setByModel(m);
    } catch (e) {
      console.error('获取用量数据失败:', e);
      setError(reportError('error.load', e));
    } finally {
      setLoading(false);
    }
  };

  // 按日期获取模型分布（热力图点击后调用）
  const fetchModelByDate = async (date: string) => {
    try {
      const m = await invoke<UsageSummaryByModel[]>('get_usage_summary_by_model', {
        days: range().days,
        date,
      });
      setByModel(m);
    } catch (e) {
      console.error('获取单日模型分布失败:', e);
    }
  };

  // 热力图日期点击回调
  const handleDateSelect = (date: string) => {
    setSelectedDate(date);
    fetchModelByDate(date);
  };

  // 初始化：同时加载热力图数据（固定365天）和时间范围数据
  createEffect(() => {
    // 加载热力图数据（只执行一次，因为不依赖 range）
    invoke<UsageSummary[]>('get_usage_summary', { days: 365 })
      .then(setHeatmapSummary)
      .catch(console.error);
  });

  // 统一 Effect：日期选择模式 vs 时间范围模式
  createEffect(() => {
    if (selectedDate()) {
      // 日期选择模式：仅刷新模型分布
      fetchModelByDate(selectedDate()!);
    } else {
      // 普通模式：刷新 summary + byModel
      fetchRangeData(range().days);
    }
  });

  const hasData = createMemo(() => summary().length > 0 || byModel().length > 0);

  return (
    <div class="w-full h-full">
      <div
        class="w-full h-full rounded-xl p-8 flex flex-col"
        style={{ background: 'transparent', 'box-shadow': '0 8px 32px rgba(0, 0, 0, 0.2)' }}
      >
        {/* 标题行 */}
        <div
          class="flex items-center justify-between pb-3 mb-6 shrink-0 animate-row-in"
          style={{ 'border-bottom': '1px solid var(--border-dim)' }}
        >
          <h3 class="text-xl font-bold tracking-tight" style={{ color: 'rgba(var(--text-base-rgb),0.85)' }}>
            {t('usage.title')}
          </h3>
          {/* 时间范围选择器 */}
          <div
            class="flex gap-1 animate-row-in"
            style={{
              background: 'rgba(var(--text-base-rgb),0.04)',
              'border-radius': '8px',
              padding: '3px',
              'animation-delay': '30ms',
            }}
          >
            <For each={RANGES}>
              {(r) => (
                <button
                  class="px-3 py-1 text-xs font-medium rounded-md transition-all duration-200 border-none cursor-pointer"
                  style={{
                    background:
                      r.days === range().days ? 'rgba(var(--primary-rgb), 0.2)' : 'transparent',
                    color:
                      r.days === range().days ? 'rgba(var(--text-base-rgb),0.9)' : 'rgba(var(--text-base-rgb),0.4)',
                  }}
                  onClick={() => {
                    setSelectedDate(null);
                    setRange(r);
                  }}
                >
                  {t(r.labelKey)}
                </button>
              )}
            </For>
          </div>
        </div>

        {/* 加载状态 */}
        <Show when={loading()}>
          <div
            class="flex items-center justify-center py-16"
            style={{ color: 'rgba(var(--text-base-rgb),0.3)' }}
          >
            <span class="text-sm font-mono">{t('common.loading')}</span>
          </div>
        </Show>

        {/* 错误状态 */}
        <Show when={error() && !loading()}>
          <div
            class="flex items-center justify-center py-16"
            style={{ color: 'rgba(224,128,144,0.7)' }}
          >
            <span class="text-sm font-mono">{error()}</span>
          </div>
        </Show>

        {/* 数据展示 */}
        <Show when={!loading() && !error()}>
          {/* 无数据占位 */}
          <Show when={!hasData()}>
            <div class="flex flex-col items-center justify-center py-16 gap-3">
              <Icon name="chart-bar" size={30} />
              <span class="text-sm" style={{ color: 'rgba(var(--text-base-rgb), 0.035);' }}>
                {t('usage.noData')}
              </span>
              <span class="text-xs" style={{ color: 'rgba(var(--text-base-rgb), 0.035);' }}>
                {t('usage.noDataDescription')}
              </span>
            </div>
          </Show>

          <Show when={hasData()}>
            <div class="flex flex-col gap-6 overflow-y-auto">
              {/* 统计卡片 */}
              <div class="animate-row-in" style={{ 'animation-delay': '60ms' }}>
                <UsageSummaryCards summary={summary()} byModel={byModel()} />
              </div>

              {/* 热力图 — 固定 365 天，不受时间范围影响 */}
              <div
                class="rounded-xl p-5 animate-row-in"
                style={{
                  background: 'rgba(var(--text-base-rgb), 0.035)',
                  border: '1px solid var(--border-dim)',
                  'animation-delay': '90ms',
                }}
              >
                <div
                  class="text-xs font-bold uppercase tracking-wider mb-3"
                  style={{ color: 'rgba(var(--text-base-rgb),0.25)' }}
                >
                  {t('usage.dailyActivity')}
                </div>
                <UsageHeatmap
                  summary={heatmapSummary()}
                  onDateSelect={handleDateSelect}
                  highlightDate={selectedDate() ?? undefined}
                />
              </div>

              {/* 模型分布 */}
              <Show when={byModel().length > 0}>
                <div
                  class="rounded-xl p-5 animate-row-in"
                  style={{
                    background: 'rgba(var(--text-base-rgb), 0.035)',
                    border: '1px solid var(--border-dim)',
                    'animation-delay': '120ms',
                  }}
                >
                  <Show
                    when={selectedDate()}
                    fallback={
                      <div
                        class="text-xs font-bold uppercase tracking-wider mb-3"
                        style={{ color: 'rgba(var(--text-base-rgb),0.25)' }}
                      >
                        {t('usage.byModel')}
                      </div>
                    }
                  >
                    <div
                      class="flex items-center gap-2 text-xs font-bold uppercase tracking-wider mb-3"
                      style={{ color: 'rgba(var(--text-base-rgb),0.25)' }}
                    >
                      <span>{t('usage.selectedModelBreakdown', { date: selectedDate()! })}</span>
                      <button
                        class="border-none cursor-pointer flex items-center justify-center w-4 h-4 rounded-full hover:bg-white/[0.1] transition-colors"
                        style={{
                          color: 'rgba(var(--text-base-rgb),0.4)',
                          'font-size': '10px',
                          'line-height': '1',
                        }}
                        onClick={() => {
                          setSelectedDate(null);
                          fetchRangeData(range().days);
                        }}
                        title={t('usage.clearDate')}
                      >
                        ✕
                      </button>
                    </div>
                  </Show>
                  <ModelBreakdown byModel={byModel()} />
                </div>
              </Show>
            </div>
          </Show>
        </Show>
      </div>
    </div>
  );
};

export default UsageSettings;
