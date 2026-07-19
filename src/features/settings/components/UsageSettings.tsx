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

type Range = { label: string; days: number };

const RANGES: Range[] = [
    { label: '7 天', days: 7 },
    { label: '30 天', days: 30 },
    { label: '90 天', days: 90 },
    { label: '全部', days: 365 },
];

const UsageSettings: Component = () => {
    const [range, setRange] = createSignal<Range>(RANGES[1]); // 默认 30 天
    const [summary, setSummary] = createSignal<UsageSummary[]>([]);
    const [byModel, setByModel] = createSignal<UsageSummaryByModel[]>([]);
    // 热力图固定加载 365 天数据，不受时间范围影响
    const [heatmapSummary, setHeatmapSummary] = createSignal<UsageSummary[]>([]);
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
            setError(String(e));
        } finally {
            setLoading(false);
        }
    };

    // 初始化：同时加载热力图数据（固定365天）和时间范围数据
    createEffect(() => {
        // 加载热力图数据（只执行一次，因为不依赖 range）
        invoke<UsageSummary[]>('get_usage_summary', { days: 365 })
            .then(setHeatmapSummary)
            .catch(console.error);
    });

    // 时间范围变化时重新加载 summary 和 byModel
    createEffect(() => {
        fetchRangeData(range().days);
    });

    const hasData = createMemo(() => summary().length > 0 || byModel().length > 0);

    return (
        <div class="w-full h-full">
            <div
                class="w-full h-full rounded-xl p-8 flex flex-col"
                style="background: transparent; box-shadow: 0 8px 32px rgba(0, 0, 0, 0.2);"
            >
                {/* 标题行 */}
                <div class="flex items-center justify-between pb-3 mb-6 shrink-0" style="border-bottom: 1px solid rgba(255,255,255,0.06);">
                    <h3 class="text-xl font-bold tracking-tight" style="color: rgba(255,255,255,0.85);">使用量</h3>
                    {/* 时间范围选择器 */}
                    <div class="flex gap-1" style="background: rgba(255,255,255,0.04); border-radius: 8px; padding: 3px;">
                        <For each={RANGES}>
                            {(r) => (
                                <button
                                    class="px-3 py-1 text-xs font-medium rounded-md transition-all duration-200 border-none cursor-pointer"
                                    style={{
                                        background: r.days === range().days ? 'rgba(var(--primary-rgb), 0.2)' : 'transparent',
                                        color: r.days === range().days ? 'rgba(255,255,255,0.9)' : 'rgba(255,255,255,0.4)',
                                    }}
                                    onClick={() => setRange(r)}
                                >
                                    {r.label}
                                </button>
                            )}
                        </For>
                    </div>
                </div>

                {/* 加载状态 */}
                <Show when={loading()}>
                    <div class="flex items-center justify-center py-16" style={{ color: 'rgba(255,255,255,0.3)' }}>
                        <span class="text-sm font-mono">加载中...</span>
                    </div>
                </Show>

                {/* 错误状态 */}
                <Show when={error() && !loading()}>
                    <div class="flex items-center justify-center py-16" style={{ color: 'rgba(224,128,144,0.7)' }}>
                        <span class="text-sm font-mono">加载失败: {error()}</span>
                    </div>
                </Show>

                {/* 数据展示 */}
                <Show when={!loading() && !error()}>
                    {/* 无数据占位 */}
                    <Show when={!hasData()}>
                        <div class="flex flex-col items-center justify-center py-16 gap-3">
                            <Icon name="chart-bar" size={30} />
                            <span class="text-sm" style={{ color: 'rgba(255, 255, 255, 0.035);' }}>暂无使用数据</span>
                            <span class="text-xs" style={{ color: 'rgba(255, 255, 255, 0.035);' }}>开始对话后，Token 用量将自动记录于此</span>
                        </div>
                    </Show>

                    <Show when={hasData()}>
                        <div class="flex flex-col gap-6 overflow-y-auto">
                            {/* 统计卡片 */}
                            <UsageSummaryCards summary={summary()} byModel={byModel()} />

                            {/* 热力图 — 固定 365 天，不受时间范围影响 */}
                            <div
                                class="rounded-xl p-5"
                                style="background: rgba(255, 255, 255, 0.035); border: 1px solid rgba(255,255,255,0.05);"
                            >
                                <div class="text-xs font-bold uppercase tracking-wider mb-3" style={{ color: 'rgba(255,255,255,0.25)' }}>
                                    每日活动
                                </div>
                                <UsageHeatmap summary={heatmapSummary()} />
                            </div>

                            {/* 模型分布 */}
                            <Show when={byModel().length > 0}>
                                <div
                                    class="rounded-xl p-5"
                                    style="background: rgba(255, 255, 255, 0.035); border: 1px solid rgba(255,255,255,0.05);"
                                >
                                    <div class="text-xs font-bold uppercase tracking-wider mb-3" style={{ color: 'rgba(255,255,255,0.25)' }}>
                                        模型分布
                                    </div>
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
