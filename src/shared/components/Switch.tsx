import { Show, type Component } from 'solid-js';

interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  /** Accessible label (also the hover title) */
  label?: string;
  /**
   * 'default': compact bordered track (memory panel / project settings).
   * 'glow': settings-style translucent track with a primary glow when on —
   * 阶段4 第6项（设置样板收敛）: previously six hand-copied inline variants of
   * this exact markup (ProviderList ×3, ProviderDetailPage,
   * EmbeddingModelSettings, ModelRow).
   */
  variant?: 'default' | 'glow';
  /** Stop click propagation (row-embedded switches where the parent row is itself clickable) */
  stopPropagation?: boolean;
}

const Switch: Component<SwitchProps> = (props) => {
  const handleClick = (e: MouseEvent) => {
    if (props.stopPropagation) e.stopPropagation();
    props.onChange(!props.checked);
  };

  return (
    <>
      {/* Both variants stay in JSX: a Solid component runs once, so an early
          return on props.variant would freeze the choice (solid/reactivity). */}
      <Show when={props.variant === 'glow'}>
        <button
          type="button"
          role="switch"
          aria-checked={props.checked}
          aria-label={props.label}
          title={props.label}
          disabled={props.disabled}
          class="relative inline-flex items-center h-[22px] w-[40px] rounded-full bg-white/[0.08] border border-white/[0.08] cursor-pointer shrink-0 focus:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--primary-rgb),0.25)] disabled:cursor-not-allowed disabled:opacity-50"
          style={{
            transition:
              'background 0.3s cubic-bezier(0.4, 0, 0.2, 1), border-color 0.3s ease, box-shadow 0.3s ease',
            ...(props.checked
              ? {
                  background: 'rgba(var(--primary-rgb), 0.7)',
                  'border-color': 'rgba(var(--primary-rgb), 0.5)',
                  'box-shadow': '0 0 12px rgba(var(--primary-rgb), 0.35)',
                }
              : {}),
          }}
          onClick={handleClick}
        >
          <span
            class="inline-block h-4 w-4 rounded-full bg-white transition-transform duration-300 shadow-[0_2px_6px_rgba(0,0,0,0.4)]"
            style={{ transform: props.checked ? 'translateX(21px)' : 'translateX(3px)' }}
          />
        </button>
      </Show>
      <Show when={props.variant !== 'glow'}>
        <button
          type="button"
          role="switch"
          aria-checked={props.checked}
          aria-label={props.label}
          title={props.label}
          disabled={props.disabled}
          class="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border transition-colors duration-200 focus:outline-none focus:ring-2 focus:ring-pri-50/40 disabled:cursor-not-allowed disabled:opacity-50"
          classList={{
            'bg-pri border-pri': props.checked,
            'bg-dark-200 border-dark-50': !props.checked,
          }}
          onClick={() => props.onChange(!props.checked)}
        >
          <span
            class="pointer-events-none block h-3.5 w-3.5 rounded-full bg-white shadow-sm transition-transform duration-200"
            classList={{
              'translate-x-[17px]': props.checked,
              'translate-x-[3px]': !props.checked,
            }}
          />
        </button>
      </Show>
    </>
  );
};

export default Switch;
