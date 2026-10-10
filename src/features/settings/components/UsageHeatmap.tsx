/**
 * UsageHeatmap - GitHub-style contribution heatmap (fixed 365 days)
 *
 * 7 rows (Sunday-Saturday) x up to 53 columns (weeks)
 * Each cell is one day; color depth maps to token usage
 */
import { Component, createMemo, createSignal, Show, For } from 'solid-js';
import type { UsageSummary } from './UsageSummaryCards';
import { formatDateTime, formatNumber, locale, t } from '../../../core/i18n';

function buildDateMap(summary: UsageSummary[]): Map<string, UsageSummary> {
  const m = new Map<string, UsageSummary>();
  for (const d of summary) {
    m.set(d.date, d);
  }
  return m;
}

function calcLevels(tokens: number[]): [number, number, number, number] {
  const nonZero = tokens.filter((t) => t > 0).sort((a, b) => a - b);
  if (nonZero.length === 0) return [1, 1, 1, 1];
  const max = nonZero[nonZero.length - 1];
  const p25 = nonZero[Math.floor(nonZero.length * 0.25)] || max * 0.25;
  const p50 = nonZero[Math.floor(nonZero.length * 0.5)] || max * 0.5;
  const p75 = nonZero[Math.floor(nonZero.length * 0.75)] || max * 0.75;
  return [p25, p50, p75, max];
}

function getLevel(val: number, levels: [number, number, number, number]): number {
  if (val <= 0) return 0;
  if (val <= levels[0]) return 1;
  if (val <= levels[1]) return 2;
  if (val <= levels[2]) return 3;
  return 4;
}

function levelColor(level: number): string {
  switch (level) {
    case 0:
      return 'rgba(var(--text-base-rgb),0.03)';
    case 1:
      return 'rgba(var(--primary-rgb), 0.15)';
    case 2:
      return 'rgba(var(--primary-rgb), 0.30)';
    case 3:
      return 'rgba(var(--primary-rgb), 0.50)';
    case 4:
      return 'rgba(var(--primary-rgb), 0.75)';
    default:
      return 'rgba(var(--text-base-rgb),0.03)';
  }
}

interface Props {
  summary: UsageSummary[];
  onDateSelect?: (date: string) => void;
  highlightDate?: string;
}

const UsageHeatmap: Component<Props> = (props) => {
  const dayLabels = createMemo(() =>
    Array.from({ length: 7 }, (_, day) =>
      day % 2 === 0
        ? ''
        : new Intl.DateTimeFormat(locale(), { weekday: 'narrow' }).format(
            new Date(2024, 0, 7 + day),
          ),
    ),
  );
  const [tooltip, setTooltip] = createSignal<{
    x: number;
    y: number;
    date: string;
    tokens: number;
    requests: number;
  } | null>(null);

  const gridData = createMemo(() => {
    const dateMap = buildDateMap(props.summary);

    // Always generate the last 365 days
    const today = new Date();
    today.setHours(0, 0, 0, 0);
    const dates: Date[] = [];
    for (let i = 364; i >= 0; i--) {
      const d = new Date(today);
      d.setDate(d.getDate() - i);
      dates.push(d);
    }

    // Pad to align the grid to a Sunday start
    const firstDate = dates[0];
    const dayOfWeek = firstDate.getDay(); // 0=Sun
    const padded: (Date | null)[] = [];
    for (let i = 0; i < dayOfWeek; i++) padded.push(null);
    for (const d of dates) padded.push(d);

    // Split into 7 rows (one per weekday, 7 days per week)
    const rows: (Date | null)[][] = [];
    const cols = Math.ceil(padded.length / 7);
    for (let row = 0; row < 7; row++) {
      const weekRow: (Date | null)[] = [];
      for (let col = 0; col < cols; col++) {
        const idx = col * 7 + row;
        weekRow.push(idx < padded.length ? padded[idx] : null);
      }
      rows.push(weekRow);
    }

    // Quantiles
    const allTokens: number[] = [];
    for (const d of dates) {
      const key = fmtDate(d);
      const entry = dateMap.get(key);
      if (entry) allTokens.push(entry.inputTokens + entry.outputTokens);
    }
    const levels = calcLevels(allTokens);

    // Month labels (with year)
    const monthLabels: { col: number; year?: string; month: string }[] = [];
    if (rows[0]) {
      let lastMonth = -1;
      let lastYear = -1;
      for (let col = 0; col < rows[0].length; col++) {
        const d = rows[0][col];
        if (d && (d.getMonth() !== lastMonth || d.getFullYear() !== lastYear)) {
          const showYear = d.getFullYear() !== lastYear;
          monthLabels.push({
            col,
            year: showYear ? formatDateTime(d, { year: 'numeric' }) : undefined,
            month: formatDateTime(d, { month: 'short' }),
          });
          lastMonth = d.getMonth();
          lastYear = d.getFullYear();
        }
      }
    }

    return { rows, cols, levels, monthLabels, dateMap };
  });

  function fmtDate(d: Date): string {
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, '0');
    const day = String(d.getDate()).padStart(2, '0');
    return `${y}-${m}-${day}`;
  }

  function fmtTokens(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return String(n);
  }

  const handleMove = (d: Date, e: MouseEvent) => {
    const key = fmtDate(d);
    const entry = gridData().dateMap.get(key);
    // Compute the tooltip position relative to the cell center
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const containerRect = (e.currentTarget as HTMLElement)
      .closest('.heatmap-container')
      ?.getBoundingClientRect();
    if (!containerRect) return;
    setTooltip({
      x: rect.left - containerRect.left + rect.width / 2,
      y: rect.top - containerRect.top - 8,
      date: key,
      tokens: entry ? entry.inputTokens + entry.outputTokens : 0,
      requests: entry ? entry.requestCount : 0,
    });
  };

  const handleLeave = () => setTooltip(null);

  return (
    <div class="heatmap-container relative select-none">
      {/* Year + month labels (two rows: year on top, month below) */}
      <div class="relative ml-8" style={{ height: '28px', 'margin-bottom': '2px' }}>
        {/* Year row */}
        <div class="absolute top-0 left-0 w-full" style={{ height: '14px' }}>
          <For each={gridData().monthLabels}>
            {({ col, year }) => {
              if (!year) return null;
              const left = 32 + col * 14;
              return (
                <span
                  class="text-[9px] font-bold absolute whitespace-nowrap"
                  style={{
                    left: `${left}px`,
                    color: 'rgba(var(--text-base-rgb),0.35)',
                    'line-height': '14px',
                  }}
                >
                  {year}
                </span>
              );
            }}
          </For>
        </div>
        {/* Month row */}
        <div class="absolute bottom-0 left-0 w-full" style={{ height: '14px' }}>
          <For each={gridData().monthLabels}>
            {({ col, month }) => {
              const left = 32 + col * 14;
              return (
                <span
                  class="text-[9px] font-bold absolute whitespace-nowrap"
                  style={{
                    left: `${left}px`,
                    color: 'rgba(var(--text-base-rgb),0.25)',
                    'line-height': '14px',
                  }}
                >
                  {month}
                </span>
              );
            }}
          </For>
        </div>
      </div>
      <div class="flex gap-0.5">
        {/* Weekday labels */}
        <div class="flex flex-col gap-0.5 mr-1.5">
          <For each={dayLabels()}>
            {(label, i) => (
              <span
                class="w-3 h-3 flex items-center justify-center text-[8px] font-bold"
                style={{
                  color: i() % 2 === 0 ? 'rgba(var(--text-base-rgb),0.18)' : 'transparent',
                  'line-height': '12px',
                }}
              >
                {label}
              </span>
            )}
          </For>
        </div>

        {/* Heatmap grid */}
        <div class="flex gap-0.5">
          <For each={Array.from({ length: gridData().cols })}>
            {(_, col) => (
              <div class="flex flex-col gap-0.5">
                <For each={gridData().rows}>
                  {(cell) => {
                    const d = cell[col()];
                    if (!d) return <div class="w-3 h-3 rounded-sm" />;
                    const key = fmtDate(d);
                    const entry = gridData().dateMap.get(key);
                    const val = entry ? entry.inputTokens + entry.outputTokens : 0;
                    const lvl = getLevel(val, gridData().levels);
                    return (
                      <div
                        classList={{
                          'w-3 h-3 rounded-sm cursor-pointer transition-colors duration-150 hover:ring-1 hover:ring-white/30': true,
                          'ring-1 ring-white/60': props.highlightDate === key,
                        }}
                        style={{ background: levelColor(lvl) }}
                        onMouseMove={(e) => handleMove(d, e)}
                        onMouseLeave={handleLeave}
                        onClick={() => props.onDateSelect?.(key)}
                      />
                    );
                  }}
                </For>
              </div>
            )}
          </For>
        </div>
      </div>

      {/* Legend */}
      <div class="flex items-center gap-1.5 mt-2 justify-end">
        <span class="text-[9px]" style={{ color: 'rgba(var(--text-base-rgb),0.2)' }}>
          {t('usage.less')}
        </span>
        <For each={[0, 1, 2, 3, 4]}>
          {(lvl) => <div class="w-3 h-3 rounded-sm" style={{ background: levelColor(lvl) }} />}
        </For>
        <span class="text-[9px]" style={{ color: 'rgba(var(--text-base-rgb),0.2)' }}>
          {t('usage.more')}
        </span>
      </div>

      {/* Tooltip - absolutely positioned, follows the hovered cell */}
      <Show when={tooltip()}>
        {(tip) => (
          <div
            class="absolute z-50 pointer-events-none px-3 py-2 rounded-lg text-xs font-mono"
            style={{
              left: `${tip().x}px`,
              top: `${tip().y}px`,
              transform: 'translate(-50%, -100%)',
              background: 'rgba(10,14,26,0.96)',
              border: '1px solid var(--border-dim)',
              color: 'rgba(var(--text-base-rgb),0.85)',
              'white-space': 'nowrap',
              'box-shadow': '0 4px 16px rgba(0,0,0,0.4)',
            }}
          >
            <div style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}>
              {formatDateTime(`${tip().date}T00:00:00`, { dateStyle: 'medium' })}
            </div>
            <div>
              <span style={{ color: 'rgba(var(--text-base-rgb),0.85)' }}>
                {fmtTokens(tip().tokens)}
              </span>{' '}
              {t('usage.tokens')}
            </div>
            <div style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}>
              {t('usage.requestCount', { count: formatNumber(tip().requests) })}
            </div>
          </div>
        )}
      </Show>
    </div>
  );
};

export default UsageHeatmap;
