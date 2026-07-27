/**
 * ModelBreakdown — 按模型用量分布
 *
 * 水平条形图 + 每行显示模型名、token 用量、占比、预估费用
 */
import { Component, createMemo, Show, For } from 'solid-js';
import type { UsageSummaryByModel } from './UsageSummaryCards';
import { getCachedCatalog } from '../../../core/utils/models';
import { t } from '../../../core/i18n';
import type { ModelMeta } from '@aio/models-data';

function fmt(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(Math.round(n));
}

function calcCost(meta: ModelMeta | undefined, inputTokens: number, outputTokens: number): number {
  if (!meta?.pricing) return 0;
  return (
    (inputTokens / 1_000_000) * meta.pricing.input +
    (outputTokens / 1_000_000) * meta.pricing.output
  );
}

const BAR_COLORS = [
  'rgba(var(--primary-rgb), 0.75)',
  'rgba(var(--primary-rgb), 0.55)',
  'rgba(var(--primary-rgb), 0.40)',
  'rgba(var(--primary-rgb), 0.28)',
  'rgba(var(--primary-rgb), 0.18)',
];

interface Props {
  byModel: UsageSummaryByModel[];
}

const ModelBreakdown: Component<Props> = (props) => {
  const catalog = () => getCachedCatalog();

  const items = createMemo(() => {
    const cat = catalog();
    const list = props.byModel.map((m) => {
      const total = m.inputTokens + m.outputTokens;
      const meta = cat?.models.find((md: ModelMeta) => md.id === m.modelId);
      const displayName = meta?.displayName || m.modelId;
      const cost = calcCost(meta, m.inputTokens, m.outputTokens);
      return { ...m, total, displayName, cost, meta };
    });

    const grandTotal = list.reduce((s, it) => s + it.total, 0);

    // Top 8 + 其他
    const top = list.slice(0, 8);
    const rest = list.slice(8);
    if (rest.length > 0) {
      const otherTotal = rest.reduce((s, it) => s + it.total, 0);
      const otherInput = rest.reduce((s, it) => s + it.inputTokens, 0);
      const otherOutput = rest.reduce((s, it) => s + it.outputTokens, 0);
      const otherCost = rest.reduce((s, it) => s + it.cost, 0);
      top.push({
        modelId: '__other__',
        inputTokens: otherInput,
        outputTokens: otherOutput,
        requestCount: rest.reduce((s, it) => s + it.requestCount, 0),
        total: otherTotal,
        displayName: t('usage.other'),
        cost: otherCost,
        meta: undefined,
      });
    }

    return top.map((it) => ({
      ...it,
      pct: grandTotal > 0 ? (it.total / grandTotal) * 100 : 0,
    }));
  });

  const maxPct = () => Math.max(...items().map((it) => it.pct), 1);

  return (
    <div class="flex flex-col gap-1">
      <For each={items()}>
        {(item, i) => (
          <div
            class="rounded-lg px-4 py-3 transition-colors duration-150 hover:bg-white/[0.02]"
            style={{
              background: 'rgba(var(--text-base-rgb), 0.035)',
              border: '1px solid var(--border-dim)',
            }}
          >
            {/* 主行：模型名 + 占比 + 条形图 */}
            <div class="flex items-center gap-3">
              <span
                class="text-sm font-medium truncate min-w-0 flex-1"
                style={{ color: 'rgba(var(--text-base-rgb),0.8)' }}
              >
                {item.displayName}
              </span>
              <span
                class="text-xs font-mono shrink-0 w-12 text-right"
                style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}
              >
                {item.pct.toFixed(1)}%
              </span>
              {/* 条形图 */}
              <div
                class="flex-1 h-2 rounded-full overflow-hidden"
                style={{ background: 'rgba(var(--text-base-rgb),0.04)' }}
              >
                <div
                  class="h-full rounded-full transition-all duration-500"
                  style={{
                    width: `${(item.pct / maxPct()) * 100}%`,
                    background: BAR_COLORS[i() % BAR_COLORS.length],
                  }}
                />
              </div>
            </div>

            {/* 次级行：input/output + 费用 */}
            <div
              class="flex items-center gap-3 mt-1.5 text-[10px] font-mono"
              style={{ color: 'rgba(var(--text-base-rgb),0.25)' }}
            >
              <span class="flex-1" />
              <span>↗ {fmt(item.inputTokens)}</span>
              <span>↘ {fmt(item.outputTokens)}</span>
              <Show when={item.cost > 0}>
                <span style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}>${item.cost.toFixed(2)}</span>
              </Show>
              <Show when={item.requestCount > 0}>
                <span>{t('usage.count', { count: item.requestCount })}</span>
              </Show>
            </div>
          </div>
        )}
      </For>
    </div>
  );
};

export default ModelBreakdown;
