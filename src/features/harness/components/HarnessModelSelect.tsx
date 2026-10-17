/**
 * Harness model selector — a strict visual replica of the chat page's
 * ModelSelector, parameterized over the agent-reported value set: icon-only
 * trigger (white circle with the current model's provider logo), upward popup
 * with a header row and logo-circle rows (`provider · name` labels carry the
 * provider prefix per the harness requirement), check mark on the active one.
 */
import { Component, createSignal, For, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import { getLogo } from '../../../core/utils/modelLogo';

export interface HarnessModelOption {
  value: string;
  label: string;
  /** Provider prefix parsed from a `provider/model` value id (drives the logo). */
  provider?: string;
}

interface HarnessModelSelectProps {
  title: string;
  options: HarnessModelOption[];
  value: string;
  /** Fallback trigger logo (the tool's brand logo) when no provider resolves. */
  toolLogo?: string | null;
  onChange: (value: string) => void;
}

/** Provider logo for a value: provider id first, then the model-name rules. */
const optionLogo = (opt: HarnessModelOption) =>
  getLogo(opt.provider ?? null, opt.label) ?? getLogo(opt.provider ?? null, opt.value);

const HarnessModelSelect: Component<HarnessModelSelectProps> = (props) => {
  const [open, setOpen] = createSignal(false);

  const currentOpt = () => props.options.find((o) => o.value === props.value);
  const triggerLogo = () => {
    const cur = currentOpt();
    if (cur) {
      const l = optionLogo(cur);
      if (l) return l;
    }
    return props.toolLogo ?? undefined;
  };

  return (
    <div class="relative">
      {/* Trigger: only the current model's logo in a white circle (chat parity) */}
      <button
        class="flex items-center justify-center bg-transparent border-none rounded-md cursor-pointer p-1 transition-all duration-200"
        title={currentOpt()?.label ?? props.title}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((o) => !o);
        }}
        onMouseEnter={(e) => {
          e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.06)';
        }}
        onMouseLeave={(e) => {
          if (!open()) e.currentTarget.style.background = 'transparent';
        }}
      >
        <Show when={triggerLogo()} fallback={<Icon name="model" size={18} class="opacity-50" />}>
          <div class="w-[22px] h-[22px] bg-white rounded-full flex items-center justify-center shrink-0 shadow-sm">
            <img src={triggerLogo()!} alt="logo" class="w-[15px] h-[15px] object-contain" />
          </div>
        </Show>
      </button>

      {/* Overlay catches outside clicks while open */}
      <Show when={open()}>
        <div class="fixed inset-0 z-[40]" onClick={() => setOpen(false)} />
      </Show>

      {/* Popup panel (chat ModelSelector replica: upward, acrylic, scrollable) */}
      <div
        class="absolute bottom-full left-0 mb-2 z-[41] w-[300px] rounded-xl overflow-hidden flex flex-col transition-all duration-200 ease-out origin-bottom"
        classList={{
          'opacity-0 scale-95 translate-y-1 pointer-events-none': !open(),
          'opacity-100 scale-100 translate-y-0 pointer-events-auto': open(),
        }}
        style={{
          background: 'var(--popup-bg)',
          border: '1px solid var(--border-dim)',
          'backdrop-filter': 'var(--popup-blur)',
          'box-shadow': '0 -8px 30px rgba(0,0,0,0.4)',
          'max-height': '340px',
        }}
      >
        <div
          class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest sticky top-0"
          style={{
            color: 'rgba(var(--text-base-rgb),0.35)',
            background: 'rgba(var(--text-base-rgb),0.04)',
            'border-bottom': '1px solid var(--border-dim)',
          }}
        >
          {props.title}
        </div>
        <div class="flex flex-col overflow-y-auto scrollbar-thin">
          <div class="p-1.5">
            <For each={props.options}>
              {(opt) => (
                <div
                  class="flex items-center gap-2 p-2 rounded-lg cursor-pointer select-none transition-all"
                  style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}
                  classList={{
                    '!bg-[rgba(var(--primary-rgb),0.12)]': props.value === opt.value,
                  }}
                  onClick={() => {
                    props.onChange(opt.value);
                    setOpen(false);
                  }}
                  onMouseEnter={(e) => {
                    if (props.value !== opt.value) {
                      e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.06)';
                      e.currentTarget.style.color = 'white';
                    }
                  }}
                  onMouseLeave={(e) => {
                    if (props.value !== opt.value) {
                      e.currentTarget.style.background = 'transparent';
                      e.currentTarget.style.color = 'rgba(var(--text-base-rgb),0.6)';
                    }
                  }}
                >
                  <div class="w-6 h-6 bg-white rounded-full flex items-center justify-center shrink-0">
                    <Show
                      when={optionLogo(opt)}
                      fallback={
                        <span
                          class="text-[11px] font-bold"
                          style={{ color: 'var(--color-dark-700)' }}
                        >
                          {(opt.provider || opt.label).charAt(0).toUpperCase()}
                        </span>
                      }
                    >
                      <img
                        src={optionLogo(opt)!}
                        alt="logo"
                        class="w-[14px] h-[14px] object-contain"
                      />
                    </Show>
                  </div>
                  <div class="flex-1 min-w-0">
                    <div class="text-[13px] text-white font-medium truncate">{opt.label}</div>
                  </div>
                  <Show when={props.value === opt.value}>
                    <Icon
                      name="check"
                      size={14}
                      class="shrink-0"
                      style={{ color: 'rgba(var(--primary-rgb),0.9)' }}
                    />
                  </Show>
                </div>
              )}
            </For>
          </div>
        </div>
      </div>
    </div>
  );
};

export default HarnessModelSelect;
