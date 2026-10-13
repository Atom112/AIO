/**
 * Reasoning intensity button (GPT-style slider)
 *
 * Sits left of the file upload button in the chat input; clicking opens a popup with a
 * horizontal 4-stop slider (off / low / medium / high). Clicking or dragging snaps to
 * the nearest stop; arrow keys step between stops. The selected level is attached to
 * the request as a native reasoning-effort parameter for reasoning-capable models
 * (see modelSupportsReasoningEffort) — the popup itself only edits the preference.
 */
import { Component, For, createSignal, onCleanup, onMount } from 'solid-js';
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
  iconName: 'bolt' | 'sparkles' | 'brain' | 'lightbulb';
}

const LEVELS: LevelOption[] = [
  {
    value: 'off',
    labelKey: 'chat.reasoning.off',
    iconName: 'bolt',
  },
  {
    value: 'low',
    labelKey: 'chat.reasoning.low',
    iconName: 'sparkles',
  },
  {
    value: 'medium',
    labelKey: 'chat.reasoning.medium',
    iconName: 'lightbulb',
  },
  {
    value: 'high',
    labelKey: 'chat.reasoning.high',
    iconName: 'brain',
  },
];

/** Trigger accent per level: off = muted text; active levels = primary. */
const LEVEL_COLORS: Record<ReasoningLevel, string> = {
  off: 'rgba(var(--text-base-rgb),0.55)',
  low: 'var(--color-pri)',
  medium: 'var(--color-pri)',
  high: 'var(--color-pri)',
};

const ReasoningButton: Component = () => {
  const [open, setOpen] = createSignal(false);
  const [dragging, setDragging] = createSignal(false);
  let containerRef: HTMLDivElement | undefined;
  let trackRef: HTMLDivElement | undefined;

  const current = () => LEVELS.find((l) => l.value === reasoningLevel()) || LEVELS[0];
  const activeIndex = () =>
    Math.max(
      0,
      LEVELS.findIndex((l) => l.value === reasoningLevel()),
    );

  /** Button is highlighted when the current level is not off */
  const isActive = () => reasoningLevel() !== 'off';

  /** Stop position as a percentage of the track (evenly spaced stops). */
  const pos = (i: number) => (i / (LEVELS.length - 1)) * 100;

  /** Snap the level to the stop nearest the pointer x position. */
  const applyPointer = (clientX: number) => {
    if (!trackRef) return;
    const rect = trackRef.getBoundingClientRect();
    const ratio = Math.min(1, Math.max(0, (clientX - rect.left) / Math.max(rect.width, 1)));
    persistReasoningLevel(LEVELS[Math.round(ratio * (LEVELS.length - 1))].value);
  };

  /** Close on outside click */
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
        class="flex items-center gap-1.5 px-2.5 h-8 rounded-md border-none cursor-pointer transition-all duration-200 select-none bg-transparent text-white/55 text-xs font-medium hover:bg-white/[0.06] hover:text-[var(--color-pri)]/60"
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
        class="absolute left-0 bottom-full mb-2 w-[240px] rounded-xl p-1.5 z-[1500] transition-all duration-150 ease-out origin-bottom"
        style={{
          background: 'rgba(var(--surface-bg),0.96)',
          'backdrop-filter': 'blur(10px)',
          border: '1px solid var(--border-dim)',
          'box-shadow': '0 12px 40px rgba(0, 0, 0, 0.45)',
        }}
        classList={{
          'invisible opacity-0 scale-95 -translate-y-1 pointer-events-none': !open(),
          'visible opacity-100 scale-100 translate-y-0 pointer-events-auto': open(),
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div class="flex items-center gap-2 px-3 pt-2 pb-1 text-[13px] font-semibold text-white/85">
          <Icon name="brain" size={14} />
          <span>{t('chat.reasoning')}</span>
        </div>
        {/* slider: pointer drag + click snap + arrow-key stepping */}
        <div
          class="relative mx-3 h-8 cursor-pointer touch-none select-none outline-none"
          role="slider"
          tabindex="0"
          aria-valuemin={0}
          aria-valuemax={LEVELS.length - 1}
          aria-valuenow={activeIndex()}
          aria-label={t('chat.reasoning')}
          onPointerDown={(e) => {
            e.currentTarget.setPointerCapture(e.pointerId);
            setDragging(true);
            applyPointer(e.clientX);
          }}
          onPointerMove={(e) => {
            if (dragging()) applyPointer(e.clientX);
          }}
          onPointerUp={() => setDragging(false)}
          onPointerCancel={() => setDragging(false)}
          onKeyDown={(e) => {
            const idx = activeIndex();
            if (e.key === 'ArrowLeft' && idx > 0) {
              e.preventDefault();
              persistReasoningLevel(LEVELS[idx - 1].value);
            } else if (e.key === 'ArrowRight' && idx < LEVELS.length - 1) {
              e.preventDefault();
              persistReasoningLevel(LEVELS[idx + 1].value);
            }
          }}
        >
          <div
            ref={trackRef}
            class="absolute left-0 right-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-[rgba(var(--text-base-rgb),0.12)]"
          >
            <div
              class="absolute left-0 top-0 h-full rounded-full bg-[var(--color-pri)]"
              style={{ width: `${pos(activeIndex())}%` }}
            />
          </div>
          <div
            class="absolute top-1/2 h-4 w-4 -translate-x-1/2 -translate-y-1/2 rounded-full bg-[rgb(var(--text-base-rgb))] shadow-md"
            style={{ left: `${pos(activeIndex())}%` }}
          />
        </div>
        {/* stop labels, centered under each stop */}
        <div class="relative mx-3 h-5 pb-1">
          <For each={LEVELS}>
            {(opt, i) => (
              <button
                type="button"
                class="absolute top-0 -translate-x-1/2 rounded px-1 py-0.5 text-[11px] leading-none cursor-pointer border-none bg-transparent transition-colors duration-150"
                classList={{
                  'text-[var(--color-pri)] font-semibold': reasoningLevel() === opt.value,
                  'text-white/45 hover:text-white/75': reasoningLevel() !== opt.value,
                }}
                style={{ left: `${pos(i())}%` }}
                onClick={() => choose(opt.value)}
              >
                {t(opt.labelKey)}
              </button>
            )}
          </For>
        </div>
      </div>
    </div>
  );
};

export default ReasoningButton;
