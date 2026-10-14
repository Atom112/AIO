/**
 * Acrylic-style custom dropdown component
 *
 * Usage:
 *   <Dropdown
 *       value={sortKey()}
 *       onChange={setSortKey}
 *       options={[
 *           { value: 'releaseDesc', label: 'Release date ↓' },
 *           { value: 'nameAsc', label: 'Name A-Z' },
 *       ]}
 *   />
 */
import { For, Show, createSignal, onCleanup, onMount, JSX } from 'solid-js';
import Icon from './Icon';
import { t } from '../../core/i18n';

export interface DropdownOption<V extends string = string> {
  value: V;
  label: string;
  icon?: string;
  disabled?: boolean;
}

export interface DropdownProps<V extends string = string> {
  value: V;
  onChange: (value: V) => void;
  options: DropdownOption<V>[];
  /** Trigger placeholder (shown when nothing is selected) */
  placeholder?: string;
  /** Custom class applied to the trigger */
  class?: string;
  /** Custom class applied to the trigger button (e.g. height alignment in toolbars) */
  triggerClass?: string;
  /** Trigger alignment */
  align?: 'left' | 'right';
  /**
   * Visual variant: "solid" (default — dark acrylic trigger, toolbars) or
   * "ghost" (transparent trigger with border-dim — settings rows, matching
   * the language-selector style).
   */
  variant?: 'solid' | 'ghost';
  /** Custom trigger renderer (e.g. to insert icons) */
  trigger?: (current: DropdownOption<V> | undefined) => JSX.Element;
  /** Disables the whole dropdown */
  disabled?: boolean;
  /** Unique ID (used for aria) */
  id?: string;
}

export const Dropdown = <V extends string = string>(props: DropdownProps<V>) => {
  const [open, setOpen] = createSignal(false);
  let containerRef: HTMLDivElement | undefined;
  let triggerRef: HTMLButtonElement | undefined;

  const current = () => props.options.find((o) => o.value === props.value);

  /** Close on outside click */
  const onDocClick = (e: MouseEvent) => {
    if (!containerRef) return;
    if (!containerRef.contains(e.target as Node)) {
      setOpen(false);
    }
  };

  /** Escape closes, arrow keys navigate */
  const onKeyDown = (e: KeyboardEvent) => {
    if (!open()) return;
    if (e.key === 'Escape') {
      setOpen(false);
      triggerRef?.focus();
      e.preventDefault();
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const dir = e.key === 'ArrowDown' ? 1 : -1;
      const enabledOptions = props.options.filter((o) => !o.disabled);
      if (enabledOptions.length === 0) return;
      const currentIdx = enabledOptions.findIndex((o) => o.value === props.value);
      const nextIdx =
        currentIdx < 0 ? 0 : (currentIdx + dir + enabledOptions.length) % enabledOptions.length;
      props.onChange(enabledOptions[nextIdx].value);
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      setOpen(false);
    }
  };

  onMount(() => {
    document.addEventListener('click', onDocClick);
    document.addEventListener('keydown', onKeyDown);
  });
  onCleanup(() => {
    document.removeEventListener('click', onDocClick);
    document.removeEventListener('keydown', onKeyDown);
  });

  return (
    <div ref={containerRef} class={`relative inline-block ${props.class ?? ''}`}>
      <button
        ref={triggerRef}
        type="button"
        id={props.id}
        disabled={props.disabled}
        class={
          props.variant === 'ghost'
            ? // Ghost variant: transparent trigger matching the language-selector style
              'w-full flex items-center justify-between gap-2 px-3 py-1.5 rounded-lg text-xs text-left cursor-pointer outline-none border border-[var(--border-dim)] transition-all duration-150 text-white/80 hover:border-[rgba(var(--primary-rgb),0.5)]'
            : `border border-white/[0.08] rounded-lg text-white px-[14px] cursor-pointer hover:border-white/[0.14] focus:outline-none focus:border-[rgba(var(--primary-rgb),0.55)] focus:bg-[rgba(0,0,0,0.35)] focus:shadow-[0_0_0_3px_rgba(var(--primary-rgb),0.12)] flex items-center gap-1.5 w-full text-center py-1 text-xs ${props.triggerClass ?? ''}`
        }
        style={
          props.variant === 'ghost'
            ? undefined
            : {
                appearance: 'none',
                '-webkit-appearance': 'none',
                '-moz-appearance': 'none',
                'background-color': 'rgba(0, 0, 0, 0.25)',
              }
        }
        onClick={(e) => {
          e.stopPropagation();
          setOpen(!open());
        }}
        aria-haspopup="listbox"
        aria-expanded={open()}
      >
        <span class="flex-1 min-w-0 text-left">
          <Show
            when={current() || props.trigger}
            fallback={
              <span class="text-[rgba(var(--text-base-rgb),0.4)]">
                {props.placeholder ?? t('common.placeholder')}
              </span>
            }
          >
            <Show when={!props.trigger} fallback={props.trigger!(current())}>
              <span class="truncate">{current()!.label}</span>
            </Show>
          </Show>
        </span>
        {/* Chevron: inline Icon (currentColor follows the theme).
            The old data-URI SVG background never rendered: CSS vars do not
            resolve inside data-URI images, so the stroke fell back to none. */}
        <Icon
          name="chevron-down"
          size={props.variant === 'ghost' ? 10 : 14}
          class={props.variant === 'ghost' ? 'shrink-0 text-white/30' : 'shrink-0 opacity-60'}
        />
      </button>

      <div
        class={`dropdown-menu absolute z-[100] min-w-full mt-1 rounded-[10px] p-1 transition-all duration-150 ease-out origin-top ${props.align === 'right' ? 'right-0' : 'left-0'} top-full`}
        classList={{
          'invisible opacity-0 scale-95 translate-y-1 pointer-events-none': !open(),
          'visible opacity-100 scale-100 translate-y-0 pointer-events-auto': open(),
        }}
        role="listbox"
        onClick={(e) => e.stopPropagation()}
      >
        <For each={props.options}>
          {(opt) => (
            <div
              class={`flex items-center gap-1.5 px-2.5 py-[7px] rounded-md text-white/[0.78] cursor-pointer transition-[background,color] duration-[120ms] select-none hover:bg-[rgba(var(--text-base-rgb),0.06)] ${opt.value === props.value ? 'selected' : ''}`}
              role="option"
              aria-selected={opt.value === props.value}
              classList={{
                'opacity-40 pointer-events-none': opt.disabled,
                '!bg-[rgba(var(--primary-rgb),0.12)]': opt.value === props.value,
              }}
              onClick={() => {
                if (opt.disabled) return;
                props.onChange(opt.value);
                setOpen(false);
                triggerRef?.focus();
              }}
            >
              <Show when={opt.icon}>
                <Icon name={opt.icon as any} size={13} class="opacity-70" />
              </Show>
              <span class="grow truncate">{opt.label}</span>
              <Show when={opt.value === props.value}>
                <Icon name="check" size={12} class="text-pri" />
              </Show>
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

export default Dropdown;
