/**
 * TokenStatsBar — 紧凑型一行式 Token/上下文统计条
 *
 * 位于聊天输入框与 Agent 模式选择器之间，一行展示：
 *   上下文进度条 | 输入/输出 tokens | 消息数 | 工具调用
 */

import { Component, Show, createMemo } from 'solid-js';
import { datas, currentAssistantId, currentTopicId, selectedModel } from '../../../core/store/store';
import { getCachedCatalog } from '../../../core/utils/models';
import type { Topic } from '../../../core/store/store';

function fmt(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return String(n);
}

function usageColor(pct: number): string {
    if (pct > 0.95) return '#ef4444';
    if (pct > 0.85) return '#f59e0b';
    if (pct > 0.70) return '#eab308';
    return 'rgba(255,255,255,0.45)';
}

const TokenStatsBar: Component = () => {
    const stats = createMemo(() => {
        const asst = datas.assistants.find(a => a.id === currentAssistantId());
        const topic = asst?.topics.find((t: Topic) => t.id === currentTopicId());
        if (!topic) return { input: 0, output: 0, messages: 0, tools: 0, maxContext: 128_000 };
        let input = 0, output = 0, tools = 0;
        for (const msg of topic.history || []) {
            if (msg.role === 'assistant') {
                input += msg.inputTokens || 0;
                output += msg.outputTokens || 0;
                tools += msg.toolCalls?.length || 0;
            }
        }
        let maxCtx = 128_000;
        const cat = getCachedCatalog();
        const mdl = selectedModel();
        if (cat && mdl) {
            const meta = cat.models?.find((m: any) => m.id === mdl.model_id);
            if (meta?.contextWindow && meta.contextWindow > 0) maxCtx = meta.contextWindow;
        }
        return { input, output, messages: topic.history?.length || 0, tools, maxContext: maxCtx };
    });

    const total = () => stats().input + stats().output;
    const pct = () => Math.min(total() / stats().maxContext, 1);
    const color = () => usageColor(pct());

    return (
        <Show when={stats().messages > 0}>
            <div
                class="flex items-center gap-1.5 px-1 py-0.5 text-[10px] select-none"
                title={`会话 tokens: ↗${fmt(stats().input)} / ↘${fmt(stats().output)}\n${stats().messages} 条消息 · ${stats().tools} 次工具调用\n上下文窗口: ${fmt(stats().maxContext)} (${(pct()*100).toFixed(0)}%)`}
            >
                {/* 微型进度条 */}
                <div class="w-8 h-1 rounded-full overflow-hidden shrink-0" style="background: rgba(255,255,255,0.08);">
                    <div
                        class="h-full rounded-full transition-all duration-500"
                        style={{ width: `${Math.max(pct() * 100, 2)}%`, background: color() }}
                    />
                </div>

                {/* 用量数字 */}
                <span class="font-mono whitespace-nowrap" style={`color: ${color()};`}>
                    {fmt(total())}
                </span>

                <Show when={stats().input > 0 || stats().output > 0}>
                    <Show when={stats().input > 0}>
                        <span class="font-mono whitespace-nowrap" style="color: rgba(255,255,255,0.30);">
                            ↗{fmt(stats().input)}
                        </span>
                    </Show>
                    <Show when={stats().output > 0}>
                        <span class="font-mono whitespace-nowrap" style="color: rgba(255,255,255,0.30);">
                            ↘{fmt(stats().output)}
                        </span>
                    </Show>
                </Show>

                <span style="color: rgba(255,255,255,0.10);">·</span>
                <span class="whitespace-nowrap" style="color: rgba(255,255,255,0.22);">
                    {stats().messages}条
                </span>

                <Show when={stats().tools > 0}>
                    <span class="whitespace-nowrap" style="color: rgba(255,255,255,0.22);">
                        🔧{stats().tools}
                    </span>
                </Show>
            </div>
        </Show>
    );
};

export default TokenStatsBar;
