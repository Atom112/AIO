/**
 * UsageSummaryCards — 用量统计卡片行
 *
 * 展示：总 Token、总请求数、预估费用、活跃天数
 * 4 张等宽卡片水平排列
 */
import { Component, createMemo } from 'solid-js';
import { getCachedCatalog } from '../../../core/utils/models';
import type { ModelMeta } from '@aio/models-data';

/** 按天聚合的用量（对应 Rust UsageSummary） */
export interface UsageSummary {
    date: string;
    inputTokens: number;
    outputTokens: number;
    requestCount: number;
}

/** 按模型聚合的用量（对应 Rust UsageSummaryByModel） */
export interface UsageSummaryByModel {
    modelId: string;
    inputTokens: number;
    outputTokens: number;
    requestCount: number;
}

function fmt(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return String(Math.round(n));
}

function calcCost(modelId: string, inputTokens: number, outputTokens: number, catalog: { models: ModelMeta[] } | null): number {
    if (!catalog) return 0;
    const meta = catalog.models.find(m => m.id === modelId);
    if (!meta?.pricing) return 0;
    return (inputTokens / 1_000_000) * meta.pricing.input +
           (outputTokens / 1_000_000) * meta.pricing.output;
}

interface Props {
    summary: UsageSummary[];
    byModel: UsageSummaryByModel[];
}

const UsageSummaryCards: Component<Props> = (props) => {
    const catalog = () => getCachedCatalog();

    const totals = createMemo(() => {
        let input = 0, output = 0, requests = 0;
        for (const d of props.summary) {
            input += d.inputTokens;
            output += d.outputTokens;
            requests += d.requestCount;
        }
        return { input, output, requests, activeDays: props.summary.length };
    });

    const cost = createMemo(() => {
        let total = 0;
        const cat = catalog();
        for (const m of props.byModel) {
            total += calcCost(m.modelId, m.inputTokens, m.outputTokens, cat);
        }
        return total;
    });

    const totalTokens = () => totals().input + totals().output;

    return (
        <div class="grid grid-cols-4 gap-3">
            {/* 总 Token */}
            <div
                class="rounded-xl p-4 flex flex-col gap-1.5"
                style="background: rgba(255, 255, 255, 0.035); border: 1px solid rgba(255,255,255,0.06);"
            >
                <span class="text-[10px] uppercase tracking-widest font-bold" style="color: rgba(255,255,255,0.3);">总 Token</span>
                <span class="text-xl font-bold font-mono" style="color: rgba(255,255,255,0.85);">{fmt(totalTokens())}</span>
                <div class="flex gap-2 text-[10px] font-mono">
                    <span style="color: rgba(255,255,255,0.35);">↗ {fmt(totals().input)}</span>
                    <span style="color: rgba(255,255,255,0.35);">↘ {fmt(totals().output)}</span>
                </div>
            </div>

            {/* 总请求 */}
            <div
                class="rounded-xl p-4 flex flex-col gap-1.5"
                style="background: rgba(255, 255, 255, 0.035); border: 1px solid rgba(255,255,255,0.06);"
            >
                <span class="text-[10px] uppercase tracking-widest font-bold" style="color: rgba(255,255,255,0.3);">总请求</span>
                <span class="text-xl font-bold font-mono" style="color: rgba(255,255,255,0.85);">{totals().requests}</span>
                <span class="text-[10px]" style="color: rgba(255,255,255,0.25);">次 LLM 调用</span>
            </div>

            {/* 预估费用 */}
            <div
                class="rounded-xl p-4 flex flex-col gap-1.5"
                style="background: rgba(255, 255, 255, 0.035); border: 1px solid rgba(255,255,255,0.06);"
            >
                <span class="text-[10px] uppercase tracking-widest font-bold" style="color: rgba(255,255,255,0.3);">预估费用</span>
                <span class="text-xl font-bold font-mono" style="color: rgba(255,255,255,0.85);">
                    ${cost().toFixed(2)}
                </span>
                <span class="text-[10px]" style="color: rgba(255,255,255,0.25);">USD</span>
            </div>

            {/* 活跃天数 */}
            <div
                class="rounded-xl p-4 flex flex-col gap-1.5"
                style="background: rgba(255, 255, 255, 0.035); border: 1px solid rgba(255,255,255,0.06);"
            >
                <span class="text-[10px] uppercase tracking-widest font-bold" style="color: rgba(255,255,255,0.3);">活跃天数</span>
                <span class="text-xl font-bold font-mono" style="color: rgba(255,255,255,0.85);">{totals().activeDays}</span>
                <span class="text-[10px]" style="color: rgba(255,255,255,0.25);">天有使用记录</span>
            </div>
        </div>
    );
};

export default UsageSummaryCards;
