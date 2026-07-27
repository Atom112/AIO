/**
 * 思考过程块 (reasoning / chain-of-thought)
 *
 * 渲染模型输出的 <think>...</think> 思考内容。
 * - 流式生成中: 自动展开，header 显示"正在思考" + 动画点
 * - 生成完成: 默认折叠，header 显示"已思考 X.Xs"
 * - 亚克力主题: 微妙透明背景 + 斜体文本 + 软边框
 */
import { Component, createSignal, onCleanup, createEffect } from 'solid-js';
import Icon from './Icon';
import { t } from '../../core/i18n';

interface ThinkBlockProps {
  /** 思考过程纯文本 (不含 <think> 标签) */
  content: string;
  /** 是否仍在流式生成 */
  isStreaming?: boolean;
}

const ThinkBlock: Component<ThinkBlockProps> = (props) => {
  const [userExpanded, setUserExpanded] = createSignal<boolean | null>(null);

  /** 流式时强制展开，完成后允许用户控制 */
  const isExpanded = () => {
    if (props.isStreaming) return true;
    return userExpanded() ?? false;
  };

  const [elapsedMs, setElapsedMs] = createSignal(0);
  let startTs = 0;
  let rafId: number | null = null;

  createEffect(() => {
    if (props.isStreaming) {
      startTs = performance.now();
      const tick = () => {
        if (!props.isStreaming) return;
        setElapsedMs(performance.now() - startTs);
        rafId = requestAnimationFrame(tick);
      };
      rafId = requestAnimationFrame(tick);
    } else {
      if (rafId) cancelAnimationFrame(rafId);
      if (startTs) setElapsedMs(performance.now() - startTs);
    }
  });

  onCleanup(() => {
    if (rafId) cancelAnimationFrame(rafId);
  });

  const formatDuration = (ms: number) => {
    if (ms < 50) return '0.1s';
    return `${(ms / 1000).toFixed(1)}s`;
  };

  const toggle = () => {
    if (props.isStreaming) return;
    setUserExpanded((p) => !(p ?? false));
  };

  return (
    <div
      class="my-3 rounded-lg overflow-hidden bg-white/[0.025] border border-white/[0.05] animate-expand-width transition-[background,border-color] duration-200 hover:bg-white/[0.04] hover:border-white/[0.08]"
      classList={{ 'is-open': isExpanded(), 'is-streaming': props.isStreaming }}
    >
      <button
        type="button"
        class="flex items-center w-full gap-2 px-3 py-2 cursor-pointer select-none bg-transparent border-none text-white/50 text-xs transition-colors duration-200 hover:text-white/75"
        onClick={toggle}
        aria-expanded={isExpanded()}
      >
        <span class="flex items-center justify-center w-5 h-5 rounded bg-white/[0.05] relative">
          <Icon name={props.isStreaming ? 'sparkles' : 'brain'} size={13} class="w-3.5 h-3.5" />
        </span>
        <span class="font-medium flex-none">
          {props.isStreaming ? t('chat.thinkingActive') : t('chat.thought')}
        </span>
        <span class="font-mono text-white/30 text-[11px] flex-none ml-0.5">
          {formatDuration(elapsedMs())}
        </span>
        <span
          class="flex items-center justify-center ml-auto text-white/30 transition-[transform,color] duration-200"
          aria-hidden="true"
        >
          <Icon name="arrow-left" size={11} class="-rotate-90 transition-transform duration-200" />
        </span>
      </button>
      <div class="px-3 overflow-hidden">
        <div class="text-[12.5px] leading-relaxed italic whitespace-pre-wrap break-words text-white/55 px-2.5 py-2 rounded-md bg-black/15 border-l-2 border-l-white/[0.08] max-h-[360px] overflow-y-auto">
          {props.content}
        </div>
      </div>
    </div>
  );
};

export default ThinkBlock;
