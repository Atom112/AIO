/**
 * The app's single checkbox standard (extracted from ShareModal's filter options and
 * codified in AGENTS.md): primary-tinted box + check icon, transparent when unchecked.
 * Never use a native checkbox input element (gated by check-gates).
 *
 * Usage:
 *   <Checkbox checked={v()} onChange={setV} label={t('...')} />
 *   <Checkbox checked={v()} onChange={setV} class="items-start">...rich label...</Checkbox>
 */
import { Component, JSX, Show } from 'solid-js';
import Icon from './Icon';

export interface CheckboxProps {
  checked: boolean;
  onChange: (v: boolean) => void;
  /** Plain-text label (ignored when children are given) */
  label?: string;
  /** Extra classes on the row wrapper (e.g. items-start for multi-line rich labels) */
  class?: string;
  /** Extra inline styles on the row wrapper */
  style?: JSX.CSSProperties;
  /** Rich label content (overrides `label`) */
  children?: JSX.Element;
}

const Checkbox: Component<CheckboxProps> = (p) => (
  <label
    class={`flex items-center gap-1.5 cursor-pointer select-none hover:text-white/70 transition-colors ${p.class ?? ''}`}
    style={p.style}
    onClick={() => p.onChange(!p.checked)}
  >
    <div
      class="w-4 h-4 rounded border flex items-center justify-center shrink-0 transition-all duration-150"
      classList={{
        'bg-[rgba(var(--primary-rgb),0.25)] border-[rgba(var(--primary-rgb),0.4)]': p.checked,
        'bg-transparent border-white/[0.15] hover:border-white/[0.3]': !p.checked,
      }}
    >
      <Show when={p.checked}>
        <Icon name="check" class="w-2.5 h-2.5" style={{ color: 'rgba(var(--primary-rgb),0.9)' }} />
      </Show>
    </div>
    <Show when={p.children} fallback={<span>{p.label}</span>}>
      {p.children}
    </Show>
  </label>
);

export default Checkbox;
