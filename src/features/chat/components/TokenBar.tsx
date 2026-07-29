/**
 * TokenBar — 上下文窗口用量进度条
 *
 * 显示当前会话的 token 使用量相对于模型上下文窗口的占比，
 * 提供绿→黄→红的颜色编码预警。
 * maxTokens 优先从 catalog 获取，fallback 到 props 传入值。
 */

import { Component, Show, createMemo } from 'solid-js';
import { selectedModel } from '../../../core/store/store';
import { getCachedCatalog } from '../../../core/utils/models';
import Icon from '../../../shared/components/Icon';
import { t } from '../../../core/i18n';

/** 根据占用百分比返回颜色和提示 */
function usageInfo(pct: number): { color: string; tip: string } {
  if (pct > 0.95) return { color: '#ef4444', tip: t('chat.token.windowExceeded') };
  if (pct > 0.85) return { color: '#f59e0b', tip: t('chat.token.highPressure') };
  if (pct > 0.7) return { color: '#eab308', tip: t('chat.token.inUse') };
  if (pct > 0.4) return { color: '#22c55e', tip: t('chat.token.normal') };
  return { color: '#22c55e', tip: t('chat.token.sufficient') };
}

/** 格式化数字 */
function fmt(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}

export interface TokenBarProps {
  usedTokens: number;
  /** 模型最大上下文窗口，0 表示未知。不传时从 catalog 自动获取 */
  maxTokens?: number;
  messageCount: number;
}

const TokenBar: Component<TokenBarProps> = (props) => {
  /** 从 catalog 获取真实 maxTokens */
  const resolvedMax = createMemo(() => {
    if (props.maxTokens && props.maxTokens > 0) return props.maxTokens;
    const cat = getCachedCatalog();
    const modelId = selectedModel()?.model_id;
    if (cat && modelId) {
      const meta = cat.models?.find((m: any) => m.id === modelId);
      if (meta?.contextWindow && meta.contextWindow > 0) return meta.contextWindow;
    }
    return 128_000; // fallback
  });

  const pct = createMemo(() => {
    const max = resolvedMax();
    if (max <= 0) return 0;
    return Math.min(props.usedTokens / max, 1);
  });

  const info = createMemo(() => usageInfo(pct()));

  return (
    <Show when={resolvedMax() > 0}>
      <div
        class="flex items-center gap-2 px-3 py-1.5 text-xs select-none"
        style={{
          background: 'rgba(0,0,0,0.15)',
          'border-bottom': '1px solid var(--border-dim)',
        }}
        title={info().tip}
      >
        <Icon name="chart-bar" size={14} />

        <div
          class="flex-1 h-1.5 rounded-full overflow-hidden"
          style={{ background: 'rgba(var(--text-base-rgb),0.08)' }}
        >
          <div
            class="h-full rounded-full transition-all duration-500"
            style={{
              width: `${Math.max(pct() * 100, 1)}%`,
              background: info().color,
            }}
          />
        </div>

        <span class="font-mono whitespace-nowrap text-xs" style={{ color: info().color }}>
          {fmt(props.usedTokens)} / {fmt(resolvedMax())}
        </span>

        <span class="text-gray-500 font-mono whitespace-nowrap">({(pct() * 100).toFixed(0)}%)</span>

        <Show when={pct() > 0.85}>
          <Icon name="zap" size={14} class="text-yellow-400" />
        </Show>
        <Show when={pct() > 0.95}>
          <Icon name="alert-triangle" size={14} class="text-red-400 animate-pulse" />
        </Show>

        <Show when={props.messageCount > 0}>
          <span class="text-gray-600 whitespace-nowrap">
            · {t('chat.messageCount', { count: props.messageCount })}
          </span>
        </Show>
      </div>
    </Show>
  );
};

export default TokenBar;
