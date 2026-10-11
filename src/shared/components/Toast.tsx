import { createSignal, onCleanup, Show, type Component } from 'solid-js';

export interface ToastState {
  msg: string;
  ok: boolean;
}

/**
 * Shared toast state hook (Stage 4 item 6: settings-layer boilerplate consolidation).
 *
 * Previously every settings page carried its own copy of
 * `const [toast, setToast] = createSignal(...)` plus a `setToast({msg, ok})` +
 * `setTimeout(() => setToast(null), N)` pair at every report site (~19 inline
 * pairs across ProviderDetailPage / ProviderList). The shared hook:
 * - auto-clears a pending timer before the next toast (the inline copies let
 *   an old 3s timer truncate a fresh 2s toast);
 * - clears the timer on owner disposal (the inline copies fired past unmount).
 */
export function useToast(defaultMs = 3000) {
  const [toast, setToast] = createSignal<ToastState | null>(null);
  let timer: number | undefined;

  const dismiss = () => {
    if (timer !== undefined) window.clearTimeout(timer);
    timer = undefined;
    setToast(null);
  };

  /** Shows a toast; `ok` picks green/red. Replaces any toast already showing. */
  const showToast = (msg: string, ok: boolean, ms: number = defaultMs) => {
    if (timer !== undefined) window.clearTimeout(timer);
    setToast({ msg, ok });
    timer = window.setTimeout(dismiss, ms);
  };

  onCleanup(dismiss);
  return { toast, showToast, dismiss };
}

/**
 * Shared toast bubble (was a duplicated JSX block in ProviderDetailPage /
 * ProviderList, character-identical). Render inside the page's root element,
 * near the end.
 */
export const ToastBubble: Component<{ toast: ToastState | null }> = (props) => (
  <Show when={props.toast}>
    <div
      class="border border-white/[0.08] rounded-xl px-[18px] py-[10px] text-white text-[13px] font-medium shadow-[0_8px_32px_rgba(0,0,0,0.45)] fixed bottom-6 left-1/2 z-50"
      classList={{
        'text-green-300': props.toast!.ok,
        'text-red-300': !props.toast!.ok,
      }}
      style={{
        background: 'rgba(var(--surface-bg), 0.88)',
        'backdrop-filter': 'blur(20px) saturate(150%)',
        animation: 'toastIn 0.3s cubic-bezier(0.175, 0.885, 0.32, 1.275) forwards',
      }}
    >
      {props.toast!.msg}
    </div>
  </Show>
);
