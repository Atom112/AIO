/**
 * Generic mode/approval selector — a strict visual replica of the chat page's
 * AgentModeSelector, parameterized over an arbitrary value set (used for
 * harness config options: agent-reported modes, spawn-time approval/sandbox
 * options, and the Claude native permission mode).
 * Trigger: icon + current label, tinted when off-default; popup opens upward
 * with a header row, value rows and a check mark on the active value.
 */
import { Component, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import Icon, { type IconName } from '../../../shared/components/Icon';

export interface HarnessModeSelectOption {
  value: string;
  label: string;
}

interface HarnessModeSelectProps {
  /** Trigger + row accent icon (chat page convention: one icon per control). */
  icon: IconName;
  /** Popup header text. */
  title: string;
  options: HarnessModeSelectOption[];
  /** Current value ('' = unset → the first option's look). */
  value: string;
  /** Trigger icon overrides per value id (e.g. claude permission modes). */
  valueIcons?: Record<string, IconName>;
  onChange: (value: string) => void;
}

const HarnessModeSelect: Component<HarnessModeSelectProps> = (props) => {
  const [open, setOpen] = createSignal(false);
  let containerRef: HTMLDivElement | undefined;

  const isActive = () => props.value !== '' && props.value !== props.options[0]?.value;
  const currentLabel = () =>
    props.options.find((o) => o.value === props.value)?.label ?? props.title;
  const currentIcon = () => (props.value && props.valueIcons?.[props.value]) || props.icon;

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
        class="flex items-center gap-1.5 px-2.5 h-8 rounded-md border-none cursor-pointer transition-all duration-200 select-none bg-transparent text-white/40 text-xs font-medium hover:bg-white/[0.06] hover:text-[var(--color-pri)]/60"
        classList={{ 'is-active': isActive() }}
        style={{
          color: isActive() ? 'rgba(var(--primary-rgb),0.85)' : undefined,
        }}
        title={props.title}
        onClick={(e) => {
          e.stopPropagation();
          setOpen(!open());
        }}
      >
        <Icon name={currentIcon()} size={15} class="flex items-center justify-center shrink-0" />
        <span class="leading-none truncate max-w-[90px]">{currentLabel()}</span>
      </button>

      {/* Dropdown panel (chat AgentModeSelector replica: upward, acrylic) */}
      <div
        class="absolute bottom-full left-0 mb-2 z-[41] min-w-[190px] rounded-xl overflow-hidden transition-all duration-150 ease-out origin-bottom"
        style={{
          background: 'var(--popup-bg)',
          border: '1px solid var(--border-dim)',
          'backdrop-filter': 'var(--popup-blur)',
          'box-shadow': '0 -8px 30px rgba(0,0,0,0.4)',
        }}
        classList={{
          'invisible opacity-0 scale-95 -translate-y-1 pointer-events-none': !open(),
          'visible opacity-100 scale-100 translate-y-0 pointer-events-auto': open(),
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest"
          style={{
            color: 'rgba(var(--text-base-rgb),0.35)',
            background: 'rgba(var(--text-base-rgb),0.04)',
            'border-bottom': '1px solid var(--border-dim)',
          }}
        >
          {props.title}
        </div>
        <div class="py-1 max-h-[320px] overflow-y-auto scrollbar-thin">
          <For each={props.options}>
            {(opt) => (
              <button
                type="button"
                class="w-full flex items-center gap-2.5 px-3 py-2 text-left transition-colors cursor-pointer border-none bg-transparent"
                style={{ color: 'rgba(var(--text-base-rgb),0.75)' }}
                classList={{ '!bg-[rgba(var(--primary-rgb),0.12)]': props.value === opt.value }}
                onClick={() => {
                  props.onChange(opt.value);
                  setOpen(false);
                }}
                onMouseEnter={(e) => {
                  if (props.value !== opt.value)
                    e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.05)';
                }}
                onMouseLeave={(e) => {
                  if (props.value !== opt.value) e.currentTarget.style.background = 'transparent';
                }}
              >
                <div class="flex-1 min-w-0 text-sm font-medium truncate">{opt.label}</div>
                <Show when={props.value === opt.value}>
                  <Icon
                    name="check"
                    size={13}
                    class="shrink-0"
                    style={{ color: 'rgba(var(--primary-rgb),0.8)' }}
                  />
                </Show>
              </button>
            )}
          </For>
        </div>
      </div>
    </div>
  );
};

export default HarnessModeSelect;
