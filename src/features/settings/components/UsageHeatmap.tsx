/**
 * UsageHeatmap — GitHub 风格贡献热力图（固定 365 天）
 *
 * 7 行（周日-周六）× 最多 53 列（周）
 * 每个格子代表一天，颜色深度对应 token 用量
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
    const nonZero = tokens.filter(t => t > 0).sort((a, b) => a - b);
    if (nonZero.length === 0) return [1, 1, 1, 1];
    const max = nonZero[nonZero.length - 1];
    const p25 = nonZero[Math.floor(nonZero.length * 0.25)] || max * 0.25;
    const p50 = nonZero[Math.floor(nonZero.length * 0.50)] || max * 0.50;
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
        case 0: return 'rgba(255,255,255,0.03)';
        case 1: return 'rgba(var(--primary-rgb), 0.15)';
        case 2: return 'rgba(var(--primary-rgb), 0.30)';
        case 3: return 'rgba(var(--primary-rgb), 0.50)';
        case 4: return 'rgba(var(--primary-rgb), 0.75)';
        default: return 'rgba(255,255,255,0.03)';
    }
}

interface Props {
    summary: UsageSummary[];
    onDateSelect?: (date: string) => void;
    highlightDate?: string;
}

const UsageHeatmap: Component<Props> = (props) => {
    const dayLabels = createMemo(() => Array.from({ length: 7 }, (_, day) =>
        day % 2 === 0 ? '' : new Intl.DateTimeFormat(locale(), { weekday: 'narrow' }).format(new Date(2024, 0, 7 + day))
    ));
    const [tooltip, setTooltip] = createSignal<{
        x: number;
        y: number;
        date: string;
        tokens: number;
        requests: number;
    } | null>(null);

    const gridData = createMemo(() => {
        const dateMap = buildDateMap(props.summary);

        // 固定生成最近 365 天
        const today = new Date();
        today.setHours(0, 0, 0, 0);
        const dates: Date[] = [];
        for (let i = 364; i >= 0; i--) {
            const d = new Date(today);
            d.setDate(d.getDate() - i);
            dates.push(d);
        }

        // 对齐到周日开始
        const firstDate = dates[0];
        const dayOfWeek = firstDate.getDay(); // 0=Sun
        const padded: (Date | null)[] = [];
        for (let i = 0; i < dayOfWeek; i++) padded.push(null);
        for (const d of dates) padded.push(d);

        // 分成 7 行（每周 7 天，每行是一个星期几）
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

        // 分位数
        const allTokens: number[] = [];
        for (const d of dates) {
            const key = fmtDate(d);
            const entry = dateMap.get(key);
            if (entry) allTokens.push(entry.inputTokens + entry.outputTokens);
        }
        const levels = calcLevels(allTokens);

        // 月份标签（带年份）
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
        // 以格子中心为基准计算 tooltip 位置
        const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
        const containerRect = (e.currentTarget as HTMLElement)
            .closest('.heatmap-container')?.getBoundingClientRect();
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
            {/* 年份 + 月份标签（两行：年份在上，月份在下） */}
            <div class="relative ml-8" style="height: 28px; margin-bottom: 2px;">
                {/* 年份行 */}
                <div class="absolute top-0 left-0 w-full" style="height: 14px;">
                    <For each={gridData().monthLabels}>
                        {({ col, year }) => {
                            if (!year) return null;
                            const left = 32 + col * 14;
                            return (
                                <span
                                    class="text-[9px] font-bold absolute whitespace-nowrap"
                                    style={{
                                        left: `${left}px`,
                                        color: 'rgba(255,255,255,0.35)',
                                        'line-height': '14px',
                                    }}
                                >
                                    {year}
                                </span>
                            );
                        }}
                    </For>
                </div>
                {/* 月份行 */}
                <div class="absolute bottom-0 left-0 w-full" style="height: 14px;">
                    <For each={gridData().monthLabels}>
                        {({ col, month }) => {
                            const left = 32 + col * 14;
                            return (
                                <span
                                    class="text-[9px] font-bold absolute whitespace-nowrap"
                                    style={{
                                        left: `${left}px`,
                                        color: 'rgba(255,255,255,0.25)',
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
                {/* 星期标签 */}
                <div class="flex flex-col gap-0.5 mr-1.5">
                    <For each={dayLabels()}>
                        {(label, i) => (
                            <span
                                class="w-3 h-3 flex items-center justify-center text-[8px] font-bold"
                                style={{
                                    color: i() % 2 === 0 ? 'rgba(255,255,255,0.18)' : 'transparent',
                                    'line-height': '12px',
                                }}
                            >
                                {label}
                            </span>
                        )}
                    </For>
                </div>

                {/* 热力图网格 */}
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

            {/* 图例 */}
            <div class="flex items-center gap-1.5 mt-2 justify-end">
                <span class="text-[9px]" style={{ color: 'rgba(255,255,255,0.2)' }}>{t('usage.less')}</span>
                {[0, 1, 2, 3, 4].map(lvl => (
                    <div class="w-3 h-3 rounded-sm" style={{ background: levelColor(lvl) }} />
                ))}
                <span class="text-[9px]" style={{ color: 'rgba(255,255,255,0.2)' }}>{t('usage.more')}</span>
            </div>

            {/* Tooltip — absolute 定位，跟随鼠标在格子上的位置 */}
            <Show when={tooltip()}>
                {(tip) => (
                    <div
                        class="absolute z-50 pointer-events-none px-3 py-2 rounded-lg text-xs font-mono"
                        style={{
                            left: `${tip().x}px`,
                            top: `${tip().y}px`,
                            transform: 'translate(-50%, -100%)',
                            background: 'rgba(10,14,26,0.96)',
                            border: '1px solid rgba(255,255,255,0.15)',
                            color: 'rgba(255,255,255,0.85)',
                            'white-space': 'nowrap',
                            'box-shadow': '0 4px 16px rgba(0,0,0,0.4)',
                        }}
                    >
                        <div style={{ color: 'rgba(255,255,255,0.5)' }}>{formatDateTime(`${tip().date}T00:00:00`, { dateStyle: 'medium' })}</div>
                        <div>
                            <span style={{ color: 'rgba(255,255,255,0.85)' }}>
                                {fmtTokens(tip().tokens)}
                            </span>{' '}
                            tokens
                        </div>
                        <div style={{ color: 'rgba(255,255,255,0.4)' }}>
                            {t('usage.requestCount', { count: formatNumber(tip().requests) })}
                        </div>
                    </div>
                )}
            </Show>
        </div>
    );
};

export default UsageHeatmap;
