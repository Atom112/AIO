/**
 * SessionStats — 会话 Token 用量统计摘要
 *
 * 统计当前话题中的所有消息的 token 用量汇总。
 * 显示总输入/总输出 token 数及估算费用。
 */

import Icon from '../../../shared/components/Icon';
import { t } from '../../../core/i18n';
import { Component, Show, createMemo } from 'solid-js';
import { datas, currentAssistantId, currentTopicId } from '../../../core/store/store';
import type { Topic, Message } from '../../../core/store/store';

/** 格式化数字为可读形式 */
function fmt(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}

/** 统计消息的 token 用量 */
function computeStats(messages: Message[]) {
  let totalInput = 0;
  let totalOutput = 0;
  let toolCallCount = 0;
  let contextInput = 0;

  for (const msg of messages) {
    if (msg.role === 'assistant') {
      totalInput += msg.inputTokens || 0;
      totalOutput += msg.outputTokens || 0;
      toolCallCount += msg.toolCalls?.length || 0;
      // 上下文峰值：最后一个 assistant 消息的 contextTokens（或 inputTokens fallback）
      contextInput = msg.contextTokens || msg.inputTokens || 0;
    }
  }

  return { totalInput, totalOutput, toolCallCount, contextInput };
}

/** 根据使用量返回颜色 */
function usageColor(used: number, max: number): string {
  if (max <= 0) return '#9ca3af';
  const pct = used / max;
  if (pct > 0.9) return '#ef4444';
  if (pct > 0.7) return '#f59e0b';
  return '#22c55e';
}

const SessionStats: Component = () => {
  const stats = createMemo(() => {
    const asst = datas.assistants.find((a) => a.id === currentAssistantId());
    const topic = asst?.topics.find((t: Topic) => t.id === currentTopicId());
    if (!topic) return { totalInput: 0, totalOutput: 0, toolCallCount: 0, contextInput: 0 };
    return computeStats(topic.history);
  });

  const maxTokens = createMemo(() => {
    // 尝试从 catalog 中获取 contextWindow（如果可用）
    // 这里简化处理：默认 128K
    return 128_000;
  });

  const totalUsed = createMemo(() => stats().contextInput);
  const color = createMemo(() => usageColor(totalUsed(), maxTokens()));

  return (
    <Show when={totalUsed() > 0}>
      <div
        class="flex items-center gap-3 px-4 py-1.5 text-xs select-none"
        style={{
          background: 'rgba(0,0,0,0.1)',
          'border-bottom': '1px solid var(--border-dim)',
        }}
      >
        <span class="text-gray-400 shrink-0 inline-flex items-center gap-1">
          <Icon name="trending-up" size={14} />
          {t('chat.session.label')}
        </span>

        <span class="font-mono" style={{ color: color() }}>
          {t('chat.session.totalTokens', { count: fmt(totalUsed()) })}
        </span>

        <Show when={stats().totalInput > 0}>
          <span class="text-gray-500 font-mono">↗ {fmt(stats().totalInput)}</span>
        </Show>

        <Show when={stats().totalOutput > 0}>
          <span class="text-gray-500 font-mono">↘ {fmt(stats().totalOutput)}</span>
        </Show>

        <Show when={stats().toolCallCount > 0}>
          <span class="text-gray-600 font-mono">
            <Icon name="wrench" size={14} />{' '}
            {t('chat.session.toolCalls', { count: stats().toolCallCount })}
          </span>
        </Show>

        <span class="flex-1" />

        <span class="text-gray-600">
          {fmt(totalUsed())}/{fmt(maxTokens())} ({((totalUsed() / maxTokens()) * 100).toFixed(0)}%)
        </span>
      </div>
    </Show>
  );
};

export default SessionStats;
