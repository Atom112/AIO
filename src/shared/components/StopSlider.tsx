/**
 * Neutral stop slider (same geometry as the reasoning-intensity slider, without
 * its per-level colors or star-field particles). Used by numeric/enumerated
 * settings such as the app font size.
 *
 * Pointer drag snaps to the nearest stop; arrow keys step one stop. The caller
 * owns the value and persistence.
 */

import { Component, For, createMemo, createSignal } from 'solid-js';

export interface StopSliderOption {
  value: string;
  label: string;
}

interface StopSliderProps {
  value: string;
  stops: ReadonlyArray<StopSliderOption>;
  onChange: (value: string) => void;
  ariaLabel: string;
}

const StopSlider: Component<StopSliderProps> = (props) => {
  const [dragging, setDragging] = createSignal(false);
  let trackRef: HTMLDivElement | undefined;

  const activeIndex = createMemo(() =>
    Math.max(
      0,
      props.stops.findIndex((stop) => stop.value === props.value),
    ),
  );

  /** Stop position as a percentage of the track (evenly spaced stops). */
  const pos = (index: number) =>
    props.stops.length <= 1 ? 0 : (index / (props.stops.length - 1)) * 100;

  /** Snap to the stop nearest the pointer x position. */
  const applyPointer = (clientX: number) => {
    if (!trackRef || props.stops.length === 0) return;
    const rect = trackRef.getBoundingClientRect();
    const ratio = Math.min(1, Math.max(0, (clientX - rect.left) / Math.max(rect.width, 1)));
    const idx = Math.round(ratio * (props.stops.length - 1));
    const next = props.stops[idx];
    if (next && next.value !== props.value) props.onChange(next.value);
  };

  return (
    <div class="w-full text-white">
      <div
        class="relative h-8 cursor-pointer touch-none select-none outline-none"
        role="slider"
        tabindex="0"
        aria-valuemin={0}
        aria-valuemax={Math.max(0, props.stops.length - 1)}
        aria-valuenow={activeIndex()}
        aria-valuetext={props.stops[activeIndex()]?.label}
        aria-label={props.ariaLabel}
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
          } else if (e.key === 'ArrowRight' && idx < props.stops.length - 1) {
            e.preventDefault();
            props.onChange(props.stops[idx + 1].value);
          } else if (e.key === 'Home') {
            e.preventDefault();
            props.onChange(props.stops[0]?.value ?? props.value);
          } else if (e.key === 'End') {
            e.preventDefault();
            props.onChange(props.stops[props.stops.length - 1]?.value ?? props.value);
          }
        }}
      >
        <div
          ref={trackRef}
          class="absolute left-0 right-0 top-1/2 h-2 -translate-y-1/2 rounded-full"
          style={{ background: 'color-mix(in srgb, currentColor 12%, transparent)' }}
        >
          <div
            class="absolute left-0 top-0 h-full overflow-hidden rounded-full transition-[width] duration-200 ease-out"
            style={{
              width: `${pos(activeIndex())}%`,
              background: 'color-mix(in srgb, currentColor 38%, transparent)',
            }}
          />
          <For each={props.stops}>
            {(_, i) => (
              <span
                class="pointer-events-none absolute top-1/2 h-1 w-1 -translate-x-1/2 -translate-y-1/2 rounded-full transition-colors duration-200"
                style={{
                  left: `${pos(i())}%`,
                  background:
                    i() <= activeIndex()
                      ? 'color-mix(in srgb, currentColor 85%, transparent)'
                      : 'color-mix(in srgb, currentColor 25%, transparent)',
                }}
              />
            )}
          </For>
        </div>
        <div
          class="absolute top-1/2 h-4 w-4 -translate-x-1/2 -translate-y-1/2 rounded-full bg-[rgb(var(--text-base-rgb))] transition-[left,transform,box-shadow] duration-200 ease-out"
          classList={{ 'scale-125': dragging() }}
          style={{
            left: `${pos(activeIndex())}%`,
            'box-shadow': '0 1px 4px color-mix(in srgb, currentColor 35%, transparent)',
          }}
        />
      </div>
      <div class="relative h-5">
        <For each={props.stops}>
          {(stop, i) => (
            <button
              type="button"
              class="absolute top-0 -translate-x-1/2 whitespace-nowrap rounded px-1 py-0.5 text-[11px] leading-none cursor-pointer border-none bg-transparent transition-colors duration-150"
              classList={{
                'font-semibold text-white/85': stop.value === props.value,
                'text-white/45 hover:text-white/75': stop.value !== props.value,
              }}
              style={{ left: `${pos(i())}%` }}
              onClick={() => props.onChange(stop.value)}
            >
              {stop.label}
            </button>
          )}
        </For>
      </div>
    </div>
  );
};

export default StopSlider;
