/**
 * TokenStatsBar - compact single-line token/context stats bar
 *
 * Sits between the chat input and the agent mode selector, showing one row:
 *   context progress bar | input/output tokens | message count | tool calls
 */

import { Component, Show, createMemo } from 'solid-js';
import {
  datas,
  currentAssistantId,
  currentTopicId,
  selectedModel,
} from '../../../core/store/store';
import { getCachedCatalog } from '../../../core/utils/models';
import type { Topic } from '../../../core/store/store';
import Icon from '../../../shared/components/Icon';
import { t } from '../../../core/i18n';

function fmt(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

function fmtPrice(n: number): string {
  if (n <= 0) return '$0';
  if (n < 0.0001) return '<$0.0001';
  if (n < 0.01) return `$${n.toFixed(4)}`;
  if (n < 1) return `$${n.toFixed(3)}`;
  return `$${n.toFixed(2)}`;
}

function usageColor(pct: number): string {
  if (pct > 0.95) return 'var(--color-danger)';
  if (pct > 0.85) return 'var(--color-warn)';
  if (pct > 0.7) return 'var(--color-warn)';
  return 'rgba(var(--text-base-rgb),0.45)';
}

const TokenStatsBar: Component = () => {
  const stats = createMemo(() => {
    const asst = datas.assistants.find((a) => a.id === currentAssistantId());
    const topic = asst?.topics.find((t: Topic) => t.id === currentTopicId());
    if (!topic)
      return {
        input: 0,
        output: 0,
        cached: 0,
        messages: 0,
        tools: 0,
        maxContext: 128_000,
        contextInput: 0,
        price: 0,
      };
    let input = 0,
      output = 0,
      cached = 0,
      tools = 0;
    let contextInput = 0;
    const history = topic.history || [];
    for (const msg of history) {
      if (msg.role === 'assistant') {
        input += msg.inputTokens || 0;
        output += msg.outputTokens || 0;
        cached += msg.cachedInputTokens || 0;
        tools += msg.toolCalls?.length || 0;
      }
    }
    // Context-window occupancy = the most recent KNOWN peak context (contextTokens of
    // the last assistant message that carries one, scanned backwards). Mainstream agent
    // harnesses (Claude Code / Codex) keep showing the last measured prompt size while
    // the next turn streams, and never reset to 0 mid-session. Deliberately NO
    // inputTokens fallback: for agent turns inputTokens is the SUM over all rounds
    // (cost unit), which wildly overstates the real context (the "500k after a short
    // chat" report). Legacy topics without a persisted contextTokens show 0 (unknown).
    for (let i = history.length - 1; i >= 0; i--) {
      const msg = history[i];
      if (msg.role === 'assistant' && (msg.contextTokens || 0) > 0) {
        contextInput = msg.contextTokens!;
        break;
      }
    }
    let maxCtx = 128_000;
    let price = 0;
    const cat = getCachedCatalog();
    const mdl = selectedModel();
    if (cat && mdl) {
      const meta = cat.models?.find((m: any) => m.id === mdl.model_id);
      if (meta?.contextWindow && meta.contextWindow > 0) maxCtx = meta.contextWindow;
      const pricing = meta?.pricing;
      if (pricing && pricing.input != null && pricing.output != null) {
        price = (input * pricing.input + output * pricing.output) / 1_000_000;
      }
    }
    return {
      input,
      output,
      cached,
      messages: topic.history?.length || 0,
      tools,
      maxContext: maxCtx,
      contextInput,
      price,
    };
  });

  // Cache hit rate = cached input / total input (cumulative for this topic)
  // input already includes the cached portion: OpenAI/DeepSeek cached_tokens is a subset of prompt_tokens,
  // while Anthropic input_tokens excludes cache_read_input_tokens, so normalize as (cached + input).
  const cacheDenom = () => stats().input + stats().cached;
  const cachePct = () => (cacheDenom() > 0 ? Math.round((stats().cached / cacheDenom()) * 100) : 0);
  /** Whether any cache hit data was ever received: always false for models whose backend does not report cached usage */
  const hasCacheStats = () => stats().cached > 0;
  /** Any input usage is enough to show the rate; with no cache hit data show "—" (distinguishes "no cache" from "not reported") */
  const showCacheRate = () => cacheDenom() > 0;

  const total = () => stats().contextInput;
  const pct = () => Math.min(total() / stats().maxContext, 1);
  const color = () => usageColor(pct());

  return (
    <Show when={stats().messages > 0}>
      <div
        class="flex items-center gap-1.5 px-1 py-0.5 text-[10px] select-none"
        title={t('chat.statsTooltip', {
          input: fmt(stats().input),
          output: fmt(stats().output),
          messages: stats().messages,
          toolCalls: stats().tools,
          maxContext: fmt(stats().maxContext),
          pct: (pct() * 100).toFixed(0),
          cacheHit: hasCacheStats() ? `${cachePct()}%` : '—',
        })}
      >
        {/* Mini progress bar */}
        <div
          class="w-8 h-1 rounded-full overflow-hidden shrink-0"
          style={{ background: 'rgba(var(--text-base-rgb),0.08)' }}
        >
          <div
            class="h-full rounded-full transition-all duration-500"
            style={{ width: `${Math.max(pct() * 100, 2)}%`, background: color() }}
          />
        </div>

        {/* Usage numbers */}
        <span class="font-mono whitespace-nowrap" style={{ color: color() }}>
          {fmt(total())}
        </span>

        <Show when={stats().input > 0 || stats().output > 0}>
          <Show when={stats().input > 0}>
            <span
              class="font-mono whitespace-nowrap"
              style={{ color: 'rgba(var(--text-base-rgb),0.30)' }}
            >
              ↗{fmt(stats().input)}
            </span>
          </Show>
          <Show when={stats().output > 0}>
            <span
              class="font-mono whitespace-nowrap"
              style={{ color: 'rgba(var(--text-base-rgb),0.30)' }}
            >
              ↘{fmt(stats().output)}
            </span>
          </Show>
        </Show>

        <span style={{ color: 'rgba(var(--text-base-rgb),0.10)' }}>·</span>
        <span class="whitespace-nowrap" style={{ color: 'rgba(var(--text-base-rgb),0.22)' }}>
          {t('chat.messageCount', { count: stats().messages })}
        </span>

        <Show when={stats().tools > 0}>
          <span
            class="inline-flex items-center gap-1 whitespace-nowrap"
            style={{ color: 'rgba(var(--text-base-rgb),0.22)' }}
          >
            <Icon name="wrench" size={14} />
            {stats().tools}
          </span>
        </Show>

        <Show when={stats().price > 0}>
          <span style={{ color: 'rgba(var(--text-base-rgb),0.10)' }}>·</span>
          <span
            class="font-mono whitespace-nowrap"
            style={{ color: 'rgba(var(--text-base-rgb),0.22)' }}
          >
            {fmtPrice(stats().price)}
          </span>
        </Show>

        {/* Cache hit rate (OpenAI cached_tokens / DeepSeek prompt_cache_hit_tokens / Anthropic cache_read_input_tokens) */}
        <Show when={showCacheRate()}>
          <span style={{ color: 'rgba(var(--text-base-rgb),0.10)' }}>·</span>
          <span
            class="inline-flex items-center gap-1 whitespace-nowrap"
            style={{
              color: hasCacheStats() ? 'var(--color-success)' : 'rgba(var(--text-base-rgb),0.22)',
            }}
            title={
              hasCacheStats()
                ? t('chat.statsCacheHit', {
                    pct: cachePct(),
                    cached: fmt(stats().cached),
                    total: fmt(cacheDenom()),
                  })
                : t('chat.statsCacheHitUnavailable')
            }
          >
            <Icon name="bolt" size={12} />
            <span class="font-mono">{hasCacheStats() ? `${cachePct()}%` : '—'}</span>
          </span>
        </Show>
      </div>
    </Show>
  );
};

export default TokenStatsBar;
