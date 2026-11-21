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
import { For, Show, createEffect, createSignal, onCleanup, onMount, JSX } from 'solid-js';
import { Portal } from 'solid-js/web';
import Icon, { type IconName } from './Icon';
import { t } from '../../core/i18n';

export interface DropdownOption<V extends string = string> {
  value: V;
  label: string;
  icon?: IconName;
  disabled?: boolean;
  /** Render this option's label in its own CSS font stack (font pickers). */
  fontFamily?: string;
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
  /** Open the menu upward (bottom-anchored triggers like input-bar controls). */
  menuUp?: boolean;
}

export const Dropdown = <V extends string = string>(props: DropdownProps<V>) => {
  const [open, setOpen] = createSignal(false);
  const [menuStyle, setMenuStyle] = createSignal<JSX.CSSProperties>({});
  const [menuZ, setMenuZ] = createSignal(90);
  // Page-level menus must not paint inside the (transparent) navbar strip; modal
  // menus keep their overlay-relative layering. `NavBar` is h-[60px].
  const [pageLevel, setPageLevel] = createSignal(true);
  const NAVBAR_HEIGHT = 60;
  const [portalTarget, setPortalTarget] = createSignal<HTMLElement>();
  let containerRef: HTMLDivElement | undefined;
  let triggerRef: HTMLButtonElement | undefined;
  let menuRef: HTMLDivElement | undefined;
  let lastMenuKey = '';

  // Portal into the app content stacking context (Layout): page menus must sit
  // below the navbar (z-100) but above the page. Portaling to <body> would put
  // them above the whole app container regardless of z-index. Resolve after
  // mount — during the initial render pass the Layout node is not in the DOM yet.
  const current = () => props.options.find((o) => o.value === props.value);

  /**
   * Pick the menu layer: below the navbar for page-level dropdowns, above the
   * nearest fixed overlay when the trigger lives inside a modal.
   */
  const updateMenuLayer = () => {
    let overlayZ = 0;
    let node = triggerRef?.parentElement ?? null;
    while (node && node !== document.body) {
      const style = getComputedStyle(node);
      if (style.position === 'fixed') {
        const z = Number.parseInt(style.zIndex, 10);
        if (Number.isFinite(z)) overlayZ = Math.max(overlayZ, z);
      }
      node = node.parentElement;
    }
    setMenuZ(overlayZ === 0 ? 90 : overlayZ < 100 ? Math.min(95, overlayZ + 5) : overlayZ + 10);
    setPageLevel(overlayZ === 0);
  };

  /**
   * Anchor the portaled menu to the trigger in viewport coordinates.
   *
   * The menu must live in a portal: settings cards and other dropdown hosts use
   * `backdrop-filter`, which creates a stacking context — an in-flow absolute
   * menu can never paint above a later sibling card, no matter how high its
   * z-index is. Fixed positioning also survives ancestor `overflow` clipping.
   */
  const positionMenu = () => {
    if (!triggerRef) return;
    const rect = triggerRef.getBoundingClientRect();
    const pad = 8;
    const gap = 4;
    const alignRight = props.align === 'right';
    // Auto-size to content (as the old min-w-full menu did), but never exceed
    // the space between the trigger and the viewport edge.
    const maxWidth = Math.max(
      120,
      Math.min(
        window.innerWidth - pad * 2,
        alignRight ? rect.right - pad : window.innerWidth - rect.left - pad,
      ),
    );
    const available = props.menuUp
      ? rect.top - gap - (pageLevel() ? NAVBAR_HEIGHT : 0) - pad
      : window.innerHeight -
        Math.max(rect.bottom + gap, pageLevel() ? NAVBAR_HEIGHT + gap : 0) -
        pad;
    const menuTop = Math.max(rect.bottom + gap, pageLevel() ? NAVBAR_HEIGHT + gap : 0);
    const next: JSX.CSSProperties = {
      'min-width': `${Math.min(rect.width, maxWidth)}px`,
      'max-width': `${maxWidth}px`,
      'max-height': `${Math.max(120, Math.min(320, available))}px`,
    };
    if (alignRight) {
      next.left = 'auto';
      next.right = `${window.innerWidth - rect.right}px`;
    } else {
      next.right = 'auto';
      next.left = `${rect.left}px`;
    }
    if (props.menuUp) {
      next.top = 'auto';
      next.bottom = `${window.innerHeight - rect.top + gap}px`;
    } else {
      next.bottom = 'auto';
      next.top = `${menuTop}px`;
    }
    const key = `${next['min-width']}|${next['max-width']}|${next['max-height']}|${next.left}|${next.right}|${next.top}|${next.bottom}`;
    if (key === lastMenuKey) return;
    lastMenuKey = key;
    setMenuStyle(next);
  };

  // Follow the trigger every frame while open. A scroll listener still leaves
  // the menu one frame behind momentum scrolling, and transition-all animated
  // the top/left writes themselves (the classic sticky-menu lag).
  createEffect(() => {
    if (!open()) return;
    updateMenuLayer();
    let frame = requestAnimationFrame(function tick() {
      positionMenu();
      frame = requestAnimationFrame(tick);
    });
    onCleanup(() => cancelAnimationFrame(frame));
  });

  /** Close on outside click (the menu is portaled, so check it separately). */
  const onDocClick = (e: MouseEvent) => {
    const target = e.target as Node;
    if (containerRef?.contains(target) || menuRef?.contains(target)) return;
    setOpen(false);
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
    setPortalTarget(document.getElementById('app-content') ?? document.body);
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
          if (!open()) {
            updateMenuLayer();
            positionMenu();
          }
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
              <span
                class="truncate"
                style={current()!.fontFamily ? { 'font-family': current()!.fontFamily } : undefined}
              >
                {current()!.label}
              </span>
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

      <Portal mount={portalTarget()}>
        <div
          ref={menuRef}
          class={`dropdown-menu fixed rounded-[10px] p-1 overflow-y-auto ${props.menuUp ? 'origin-bottom' : 'origin-top'}`}
          style={{ ...menuStyle(), 'z-index': `${menuZ()}` }}
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
                  <Icon name={opt.icon} size={13} class="opacity-70" />
                </Show>
                <span
                  class="grow truncate"
                  style={opt.fontFamily ? { 'font-family': opt.fontFamily } : undefined}
                >
                  {opt.label}
                </span>
                <Show when={opt.value === props.value}>
                  <Icon name="check" size={12} class="text-pri" />
                </Show>
              </div>
            )}
          </For>
        </div>
      </Portal>
    </div>
  );
};

export default Dropdown;
