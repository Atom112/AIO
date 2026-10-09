/**
 * UsageSettings - usage stats sub-page
 *
 * Composes: time range selector + summary cards + heatmap (fixed 365 days) + model breakdown
 */
import { Component, createSignal, createEffect, createMemo, Show, For } from 'solid-js';
import UsageSummaryCards from './UsageSummaryCards';
import type { UsageSummary, UsageSummaryByModel } from '../../../core/types/usage';
import { getUsageSummary, getUsageSummaryByModel } from '../../../core/api/usage';
import UsageHeatmap from './UsageHeatmap';
import ModelBreakdown from './ModelBreakdown';
import Icon from '../../../shared/components/Icon';
import { btnIcon, btnTabXs, tabActive, tabInactive } from '../../../shared/components/buttonStyles';
import { reportError, t, type TranslationKey } from '../../../core/i18n';

type Range = { labelKey: TranslationKey; days: number };

const RANGES: Range[] = [
  { labelKey: 'usage.range7', days: 7 },
  { labelKey: 'usage.range30', days: 30 },
  { labelKey: 'usage.range90', days: 90 },
  { labelKey: 'usage.rangeAll', days: 365 },
];

const UsageSettings: Component = () => {
  const [range, setRange] = createSignal<Range>(RANGES[1]); // default: 30 days
  const [summary, setSummary] = createSignal<UsageSummary[]>([]);
  const [byModel, setByModel] = createSignal<UsageSummaryByModel[]>([]);
  // Heatmap always loads 365 days of data, independent of the time range
  const [heatmapSummary, setHeatmapSummary] = createSignal<UsageSummary[]>([]);
  const [selectedDate, setSelectedDate] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);

  // Load range-dependent data
  const fetchRangeData = async (days: number) => {
    setLoading(true);
    setError(null);
    try {
      const [s, m] = await Promise.all([getUsageSummary(days), getUsageSummaryByModel(days)]);
      setSummary(s);
      setByModel(m);
    } catch (e) {
      console.error('获取用量数据失败:', e);
      setError(reportError('error.load', e));
    } finally {
      setLoading(false);
    }
  };

  // Fetch model breakdown for one date (called after a heatmap click)
  const fetchModelByDate = async (date: string) => {
    try {
      const m = await getUsageSummaryByModel(range().days, date);
      setByModel(m);
    } catch (e) {
      console.error('获取单日模型分布失败:', e);
    }
  };

  // Heatmap date click callback
  const handleDateSelect = (date: string) => {
    setSelectedDate(date);
    fetchModelByDate(date);
  };

  // Init: load heatmap data (fixed 365 days) and time-range data together
  createEffect(() => {
    // Load heatmap data (runs once since it does not depend on range)
    getUsageSummary(365).then(setHeatmapSummary).catch(console.error);
  });

  // Single effect: date-selected mode vs time-range mode
  createEffect(() => {
    if (selectedDate()) {
      // Date-selected mode: refresh model breakdown only
      fetchModelByDate(selectedDate()!);
    } else {
      // Normal mode: refresh summary + byModel
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
        {/* Header row */}
        <div
          class="flex items-center justify-between pb-3 mb-6 shrink-0 animate-row-in"
          style={{ 'border-bottom': '1px solid var(--border-dim)' }}
        >
          <h3
            class="text-xl font-bold tracking-tight"
            style={{ color: 'rgba(var(--text-base-rgb),0.85)' }}
          >
            {t('usage.title')}
          </h3>
          {/* Time range selector */}
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
                  class={`${btnTabXs} ${r.days === range().days ? tabActive : tabInactive}`}
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

        {/* Loading state */}
        <Show when={loading()}>
          <div
            class="flex items-center justify-center py-16"
            style={{ color: 'rgba(var(--text-base-rgb),0.3)' }}
          >
            <span class="text-sm font-mono">{t('common.loading')}</span>
          </div>
        </Show>

        {/* Error state */}
        <Show when={error() && !loading()}>
          <div
            class="flex items-center justify-center py-16"
            style={{ color: 'rgba(224,128,144,0.7)' }}
          >
            <span class="text-sm font-mono">{error()}</span>
          </div>
        </Show>

        {/* Data display */}
        <Show when={!loading() && !error()}>
          {/* Empty state */}
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
              {/* Summary cards */}
              <div class="animate-row-in" style={{ 'animation-delay': '60ms' }}>
                <UsageSummaryCards summary={summary()} byModel={byModel()} />
              </div>

              {/* Heatmap - fixed 365 days, independent of the time range */}
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

              {/* Model breakdown */}
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
                        class={'w-4 h-4 text-white/40 ' + btnIcon}
                        style={{ 'font-size': '10px', 'line-height': '1' }}
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
