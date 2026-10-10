/**
 * Number stepper input with themed ±1 arrows
 *
 * The WebView-native number spinners are solid triangles that clash with the acrylic theme,
 * and self-drawn ::-webkit-inner-spin-button backgrounds do not render in WebView2.
 * Native arrows are therefore hidden globally in the "Number input" section of index.css;
 * this component draws the stepper arrows as thin chevrons matching the app icon style.
 *
 * Arrow clicks / keyboard up-down keys / typed input are all clamped to [min, max];
 * the arrow buttons stay out of the tab order and prevent default on mousedown to keep input focus.
 */
import { Component } from 'solid-js';
import Icon from './Icon';

const inputClass =
  'w-full h-[30px] pl-2 pr-5 rounded-md text-sm text-right bg-white/[0.05] border border-white/[0.1] focus:outline-none focus:border-[rgba(var(--primary-rgb),0.5)]';

const stepBtnClass =
  'flex items-center justify-center h-[10px] bg-transparent border-none p-0 cursor-pointer transition-colors duration-200 text-[rgba(var(--text-base-rgb),0.4)] hover:text-[rgba(var(--text-base-rgb),0.9)]';

const NumberInput: Component<{
  value: number;
  min: number;
  max: number;
  onInput: (value: number) => void;
  /** Fallback value used when input is invalid (empty/non-numeric); defaults to min */
  fallback?: number;
  /** Container width class; defaults to w-[84px] */
  class?: string;
}> = (props) => {
  const clamp = (raw: number) =>
    Number.isFinite(raw)
      ? Math.min(props.max, Math.max(props.min, Math.round(raw)))
      : (props.fallback ?? props.min);

  return (
    <div class={`relative ${props.class ?? 'w-[84px]'}`}>
      <input
        type="number"
        class={inputClass}
        min={props.min}
        max={props.max}
        step={1}
        value={props.value}
        onInput={(e) => props.onInput(clamp(Number(e.currentTarget.value)))}
      />
      <div class="absolute right-1.5 top-1/2 -translate-y-1/2 flex flex-col">
        <button
          type="button"
          tabIndex={-1}
          class={stepBtnClass}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => props.onInput(clamp(props.value + 1))}
        >
          <Icon name="chevron-up" size={9} />
        </button>
        <button
          type="button"
          tabIndex={-1}
          class={stepBtnClass}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => props.onInput(clamp(props.value - 1))}
        >
          <Icon name="chevron-down" size={9} />
        </button>
      </div>
    </div>
  );
};

export default NumberInput;
