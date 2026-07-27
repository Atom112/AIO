/**
 * 推理强度按钮 (LobeHub 风格)
 *
 * 位于聊天输入框上传文件按钮左侧, 点击展开弹窗选择推理等级.
 * 弹窗向上弹出 (避免被聊天区遮挡), 包含 4 个等级选项 (关/低/中/高).
 */
import { Component, For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import {
  reasoningLevel,
  persistReasoningLevel,
  type ReasoningLevel,
} from '../../../core/store/store';
import Icon from '../../../shared/components/Icon';
import { t, type TranslationKey } from '../../../core/i18n';

interface LevelOption {
  value: ReasoningLevel;
  labelKey: TranslationKey;
  descKey: TranslationKey;
  iconName: 'bolt' | 'sparkles' | 'brain' | 'lightbulb';
}

const LEVELS: LevelOption[] = [
  {
    value: 'off',
    labelKey: 'chat.reasoning.off',
    descKey: 'chat.reasoning.offDescription',
    iconName: 'bolt',
  },
  {
    value: 'low',
    labelKey: 'chat.reasoning.low',
    descKey: 'chat.reasoning.lowDescription',
    iconName: 'sparkles',
  },
  {
    value: 'medium',
    labelKey: 'chat.reasoning.medium',
    descKey: 'chat.reasoning.mediumDescription',
    iconName: 'lightbulb',
  },
  {
    value: 'high',
    labelKey: 'chat.reasoning.high',
    descKey: 'chat.reasoning.highDescription',
    iconName: 'brain',
  },
];

const LEVEL_COLORS: Record<ReasoningLevel, string> = {
  off: 'rgba(var(--text-base-rgb),0.55)',
  low: 'rgba(144,200,144,0.85)',
  medium: 'rgba(220,170,120,0.85)',
  high: 'rgba(180,150,220,0.85)',
};

const ReasoningButton: Component = () => {
  const [open, setOpen] = createSignal(false);
  let containerRef: HTMLDivElement | undefined;
  const current = () => LEVELS.find((l) => l.value === reasoningLevel()) || LEVELS[0];

  /** 当前等级不是 off 时, 按钮高亮显示 */
  const isActive = () => reasoningLevel() !== 'off';

  /** 点击外部关闭 */
  const onDocClick = (e: MouseEvent) => {
    if (!containerRef) return;
    if (!containerRef.contains(e.target as Node)) {
      setOpen(false);
    }
  };

  onMount(() => document.addEventListener('mousedown', onDocClick));
  onCleanup(() => document.removeEventListener('mousedown', onDocClick));

  const choose = (lvl: ReasoningLevel) => {
    persistReasoningLevel(lvl);
    setOpen(false);
  };

  return (
    <div ref={containerRef} class="relative inline-block">
      <button
        type="button"
        class="flex items-center gap-1.5 px-2.5 h-8 rounded-md border-none cursor-pointer transition-all duration-200 select-none bg-transparent text-white/55 text-xs font-medium hover:bg-white/[0.06] hover:text-[#7c9abf]/60"
        style={{ color: LEVEL_COLORS[reasoningLevel()] }}
        classList={{ 'is-active': isActive() }}
        title={t('chat.reasoning')}
        onClick={(e) => {
          e.stopPropagation();
          setOpen(!open());
        }}
      >
        <Icon
          name={current().iconName}
          size={15}
          class="flex items-center justify-center shrink-0"
        />
        <span class="leading-none">{t(current().labelKey)}</span>
      </button>
      <div
        class="absolute left-0 bottom-full mb-2 w-[280px] rounded-xl p-1.5 z-[1500] transition-all duration-150 ease-out origin-bottom"
        style={{
          background: 'rgba(var(--surface-bg),0.96)',
          'backdrop-filter': 'blur(12px)',
          border: '1px solid var(--border-dim)',
          'box-shadow': '0 12px 40px rgba(0, 0, 0, 0.45)',
        }}
        classList={{
          'invisible opacity-0 scale-95 -translate-y-1 pointer-events-none': !open(),
          'visible opacity-100 scale-100 translate-y-0 pointer-events-auto': open(),
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div class="px-3 pt-2 pb-2.5 border-b border-b-white/[0.05]">
          <div class="flex items-center gap-2 text-[13px] font-semibold mb-1 text-white/85">
            <Icon name="brain" size={14} />
            <span>{t('chat.reasoning')}</span>
          </div>
          <div class="text-[11px] text-white/40 leading-[1.5] pl-[22px]">
            {t('chat.reasoning.description')}
          </div>
        </div>
        <div class="flex flex-col gap-0.5 pt-1.5 pb-1">
          <For each={LEVELS}>
            {(opt) => (
              <button
                type="button"
                class="flex items-center gap-2.5 w-full px-2.5 py-2 rounded-md border-none cursor-pointer text-left transition-all duration-150 bg-transparent text-white/70 hover:bg-white/[0.05] hover:text-white/90"
                style={
                  reasoningLevel() === opt.value
                    ? { background: 'rgba(var(--text-base-rgb),0.06)' }
                    : {}
                }
                onClick={() => choose(opt.value)}
              >
                <span
                  class="flex items-center justify-center w-7 h-7 rounded-md shrink-0"
                  style={{
                    background: 'rgba(var(--text-base-rgb),0.05)',
                    color: LEVEL_COLORS[opt.value],
                  }}
                >
                  <Icon name={opt.iconName} size={14} />
                </span>
                <span class="flex flex-col grow min-w-0">
                  <span class="text-[12.5px] font-semibold leading-tight">{t(opt.labelKey)}</span>
                  <span class="text-[10.5px] text-white/40 leading-[1.4] mt-0.5">
                    {t(opt.descKey)}
                  </span>
                </span>
                <Show when={reasoningLevel() === opt.value}>
                  <Icon name="check" size={13} class="shrink-0 text-white/95 block" />
                </Show>
              </button>
            )}
          </For>
        </div>
      </div>
    </div>
  );
};

export default ReasoningButton;
