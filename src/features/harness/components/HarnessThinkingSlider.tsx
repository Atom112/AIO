/**
 * Harness thinking-level slider — a strict visual replica of the chat page's
 * ReasoningButton, generalized to an arbitrary ordered stop set (each harness
 * tool reports its own levels: claude low/medium/high, codex
 * minimal/low/medium/high, omp off/auto/high, ...). Trigger shows the current
 * level with a position-mapped icon and hue; the popup is the same drag/snap
 * slider with gradient fill, star field, ticks, thumb glow and stop labels.
 */
import { Component, For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import Icon, { type IconName } from '../../../shared/components/Icon';

export interface HarnessThinkingStop {
  value: string;
  label: string;
}

interface HarnessThinkingSliderProps {
  title: string;
  stops: HarnessThinkingStop[];
  /** Current value ('' = unset → first stop). */
  value: string;
  onChange: (value: string) => void;
}

/** Position-mapped icon per stop (chat: bolt/sparkles/lightbulb/brain). */
const STOP_ICONS: IconName[] = ['bolt', 'sparkles', 'lightbulb', 'brain'];
/** Position-mapped hue per stop: stop 0 is muted; the rest walk low→high. */
const stopColor = (i: number, n: number) => {
  if (i === 0) return 'rgba(var(--text-base-rgb),0.55)';
  const ratio = n <= 1 ? 1 : i / (n - 1);
  if (ratio <= 0.5) return 'var(--color-reasoning-low)';
  if (ratio < 1) return 'var(--color-reasoning-medium)';
  return 'var(--color-reasoning-high)';
};
const stopIcon = (i: number, n: number): IconName =>
  STOP_ICONS[
    Math.min(STOP_ICONS.length - 1, Math.round((i / Math.max(n - 1, 1)) * (STOP_ICONS.length - 1)))
  ];

const HarnessThinkingSlider: Component<HarnessThinkingSliderProps> = (props) => {
  const [open, setOpen] = createSignal(false);
  const [dragging, setDragging] = createSignal(false);
  let containerRef: HTMLDivElement | undefined;
  let trackRef: HTMLDivElement | undefined;

  const count = () => Math.max(props.stops.length, 1);
  const activeIndex = () => {
    const idx = props.stops.findIndex((s) => s.value === props.value);
    return idx >= 0 ? idx : 0;
  };
  const current = () => props.stops[activeIndex()] ?? props.stops[0];
  const activeColor = () => stopColor(activeIndex(), count());
  const isActive = () => activeIndex() > 0;

  /** Stop position as a percentage of the track (evenly spaced stops). */
  const pos = (i: number) => (count() <= 1 ? 0 : (i / (count() - 1)) * 100);

  const fillGradient = () => {
    if (!isActive()) return 'none';
    const c = activeColor();
    return `linear-gradient(90deg, color-mix(in srgb, ${c} 30%, transparent), ${c})`;
  };

  const thumbGlow = () => {
    if (!isActive()) return '0 1px 4px rgba(0, 0, 0, 0.4)';
    const c = activeColor();
    return `0 0 0 2px rgba(var(--surface-bg),0.9), 0 0 12px 2px color-mix(in srgb, ${c} 60%, transparent)`;
  };

  /** Star-field dots layered over the fill (foreground-colored sparkles). */
  const STAR_DOTS =
    'radial-gradient(rgba(var(--text-base-rgb),0.95) 0.6px, transparent 1px),' +
    'radial-gradient(rgba(var(--text-base-rgb),0.55) 0.6px, transparent 1px),' +
    'radial-gradient(rgba(var(--text-base-rgb),0.75) 0.5px, transparent 1px)';

  /** Snap the level to the stop nearest the pointer x position. */
  const applyPointer = (clientX: number) => {
    if (!trackRef) return;
    const rect = trackRef.getBoundingClientRect();
    const ratio = Math.min(1, Math.max(0, (clientX - rect.left) / Math.max(rect.width, 1)));
    const stop = props.stops[Math.round(ratio * (count() - 1))];
    if (stop) props.onChange(stop.value);
  };

  const onDocClick = (e: MouseEvent) => {
    if (!containerRef) return;
    if (!containerRef.contains(e.target as Node)) setOpen(false);
  };
  onMount(() => document.addEventListener('mousedown', onDocClick));
  onCleanup(() => document.removeEventListener('mousedown', onDocClick));

  return (
    <div ref={containerRef} class="relative inline-block">
      <button
        type="button"
        class="flex items-center gap-1.5 px-2.5 h-8 rounded-md border-none cursor-pointer transition-all duration-200 select-none bg-transparent text-white/55 text-xs font-medium hover:bg-white/[0.06] hover:text-[var(--color-pri)]/60"
        style={{ color: activeColor() }}
        classList={{ 'is-active': isActive() }}
        title={props.title}
        onClick={(e) => {
          e.stopPropagation();
          setOpen(!open());
        }}
      >
        <Icon
          name={stopIcon(activeIndex(), count())}
          size={15}
          class="flex items-center justify-center shrink-0"
        />
        <span class="leading-none">{current()?.label}</span>
      </button>
      <div
        class="absolute left-0 bottom-full mb-2 w-[240px] rounded-xl p-1.5 z-[1500] transition-all duration-150 ease-out origin-bottom"
        style={{
          background: 'var(--popup-bg)',
          'backdrop-filter': 'var(--popup-blur)',
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
          <span>{props.title}</span>
        </div>
        {/* slider: pointer drag + click snap + arrow-key stepping */}
        <div
          class="relative mx-3 h-8 cursor-pointer touch-none select-none outline-none"
          role="slider"
          tabindex="0"
          aria-valuemin={0}
          aria-valuemax={count() - 1}
          aria-valuenow={activeIndex()}
          aria-label={props.title}
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
              props.onChange(props.stops[idx - 1].value);
            } else if (e.key === 'ArrowRight' && idx < count() - 1) {
              e.preventDefault();
              props.onChange(props.stops[idx + 1].value);
            }
          }}
        >
          <div
            ref={trackRef}
            class="absolute left-0 right-0 top-1/2 h-2 -translate-y-1/2 rounded-full bg-[rgba(var(--text-base-rgb),0.12)]"
          >
            {/* level-hue gradient fill with a twinkling star field (hidden at stop 0) */}
            <div
              class="absolute left-0 top-0 h-full overflow-hidden rounded-full transition-[width] duration-200 ease-out"
              style={{ width: `${pos(activeIndex())}%`, background: fillGradient() }}
            >
              <Show when={isActive()}>
                <div
                  class="absolute inset-0"
                  style={{
                    'background-image': STAR_DOTS,
                    'background-size': '11px 11px, 17px 17px, 23px 23px',
                    'background-position': '0 1px, 5px 6px, 11px 3px',
                    animation: 'reasoning-twinkle 2.6s ease-in-out infinite',
                  }}
                />
              </Show>
            </div>
            {/* stop ticks */}
            <For each={props.stops}>
              {(_, i) => (
                <span
                  class="pointer-events-none absolute top-1/2 h-1 w-1 -translate-x-1/2 -translate-y-1/2 rounded-full transition-colors duration-200"
                  style={{
                    left: `${pos(i())}%`,
                    background:
                      isActive() && i() <= activeIndex()
                        ? 'rgba(var(--text-base-rgb),0.9)'
                        : 'rgba(var(--text-base-rgb),0.25)',
                  }}
                />
              )}
            </For>
          </div>
          <div
            class="absolute top-1/2 h-4 w-4 -translate-x-1/2 -translate-y-1/2 rounded-full bg-[rgb(var(--text-base-rgb))] transition-[left,transform,box-shadow] duration-200 ease-out"
            classList={{ 'scale-125': dragging() }}
            style={{ left: `${pos(activeIndex())}%`, 'box-shadow': thumbGlow() }}
          />
        </div>
        {/* stop labels, centered under each stop; the selected one takes its hue */}
        <div class="relative mx-3 h-5 pb-1">
          <For each={props.stops}>
            {(stop, i) => (
              <button
                type="button"
                class="absolute top-0 -translate-x-1/2 rounded px-1 py-0.5 text-[11px] leading-none cursor-pointer border-none bg-transparent transition-colors duration-150"
                classList={{
                  'font-semibold': props.value === stop.value || (!props.value && i() === 0),
                  'text-white/45 hover:text-white/75': !(
                    props.value === stop.value ||
                    (!props.value && i() === 0)
                  ),
                }}
                style={{
                  left: `${pos(i())}%`,
                  color:
                    activeIndex() === i() && (props.value || i() === 0)
                      ? stopColor(i(), count())
                      : undefined,
                }}
                onClick={() => {
                  props.onChange(stop.value);
                  setOpen(false);
                }}
              >
                {stop.label}
              </button>
            )}
          </For>
        </div>
      </div>
    </div>
  );
};

export default HarnessThinkingSlider;
