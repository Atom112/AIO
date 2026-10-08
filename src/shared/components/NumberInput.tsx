/**
 * 数字步进输入框（主题化 ±1 箭头）
 *
 * WebView 原生 number 上下箭头是实心大三角，与亚克力主题不搭；
 * 且 ::-webkit-inner-spin-button 伪元素自绘背景在 WebView2 中不渲染。
 * 故原生箭头由 index.css「Number 输入」区统一隐藏，本组件用与应用图标
 * 同风格的细线 chevron 渲染步进箭头。
 *
 * 点击箭头 / 键盘上下键 / 直接输入均会 clamp 到 [min, max]；
 * 箭头按钮不参与 Tab 序，mousedown 时阻止默认行为以保留输入框焦点。
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
  /** 输入无效（空/非数字）时的回退值，默认 min */
  fallback?: number;
  /** 容器宽度类，默认 w-[84px] */
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
