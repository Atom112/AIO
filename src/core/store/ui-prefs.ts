// UI preferences and local persistence (theme / dark mode / reasoning level / search toggle / auto-start confirmation).

import { createSignal } from 'solid-js';
import type { AppBackgroundKind } from './app-background';

/** Canonical soft-mist blue: the default theme color and the "soft blue" preset value. */
export const DEFAULT_THEME_COLOR = '#7c9abf';
const HEX_COLOR_RE = /^#[0-9a-fA-F]{6}$/;

/**
 * Coerce a persisted theme color to `#rrggbb`.
 * Older builds could persist a CSS var reference (the soft-blue preset used
 * `var(--color-pri)`), which made every hex parse return black — heal it on load.
 * @param value raw localStorage value
 * @returns a valid hex color (lowercased) or the default soft-mist blue
 */
export const normalizeThemeColor = (value: string | null | undefined): string =>
  typeof value === 'string' && HEX_COLOR_RE.test(value) ? value.toLowerCase() : DEFAULT_THEME_COLOR;

/** Theme color signal, read from localStorage or defaulted to soft-mist blue `#7c9abf` (must be hex — theme effects parse RGB by hex slicing). */
export const [themeColor, setThemeColor] = createSignal(
  normalizeThemeColor(localStorage.getItem('theme-color')),
);
/** Dark mode signal, read from localStorage. */
export const [isDarkMode, setIsDarkMode] = createSignal(
  localStorage.getItem('dark-mode') === 'true',
);

/** Reasoning effort (off / low / medium / high). */
export type ReasoningLevel = 'off' | 'low' | 'medium' | 'high';
const REASONING_KEY = 'chat-reasoning-level';
export const [reasoningLevel, setReasoningLevel] = createSignal<ReasoningLevel>(
  (localStorage.getItem(REASONING_KEY) as ReasoningLevel) || 'off',
);
export const persistReasoningLevel = (lvl: ReasoningLevel) => {
  setReasoningLevel(lvl);
  localStorage.setItem(REASONING_KEY, lvl);
};
/** Web search toggle. */
const WEB_SEARCH_KEY = 'chat-web-search';
export const [webSearchEnabled, setWebSearchEnabled] = createSignal<boolean>(
  localStorage.getItem(WEB_SEARCH_KEY) === 'true',
);
export const persistWebSearch = (enabled: boolean) => {
  setWebSearchEnabled(enabled);
  localStorage.setItem(WEB_SEARCH_KEY, String(enabled));
};

/** App font family (empty = the default system stack). */
const APP_FONT_KEY = 'aio-app-font';
/**
 * Coerce a stored/selected font family.
 * @param value raw localStorage value or dropdown selection
 * @returns trimmed family name without control characters (empty when unset)
 */
export const normalizeAppFont = (value: string | null | undefined): string => {
  if (typeof value !== 'string') return '';
  return [...value]
    .filter((ch) => ch.charCodeAt(0) >= 0x20 && ch.charCodeAt(0) !== 0x7f)
    .join('')
    .trim()
    .slice(0, 120);
};
export const [appFont, setAppFont] = createSignal(
  normalizeAppFont(localStorage.getItem(APP_FONT_KEY)),
);
/** Persist the app font family; empty removes the override. */
export const persistAppFont = (font: string): void => {
  const normalized = normalizeAppFont(font);
  setAppFont(normalized);
  try {
    if (normalized) localStorage.setItem(APP_FONT_KEY, normalized);
    else localStorage.removeItem(APP_FONT_KEY);
  } catch {
    /* restricted storage: the session keeps the selection */
  }
};
/**
 * CSS `font-family` value for a stored family name (quoted + system fallbacks).
 * @param font family name; empty yields an empty string (no override)
 */
export const appFontStack = (font: string): string => {
  const normalized = normalizeAppFont(font);
  if (!normalized) return '';
  return `${JSON.stringify(normalized)}, ui-sans-serif, system-ui, sans-serif`;
};

/** App font-size stops. `standard` is the current/default size (scale 1). */
export type AppFontSize = 'small' | 'standard' | 'large' | 'xlarge';

const APP_FONT_SIZE_KEY = 'aio-app-font-size';

const APP_FONT_SIZE_SCALES: Record<AppFontSize, number> = {
  small: 0.875,
  standard: 1,
  large: 1.125,
  xlarge: 1.25,
};

/** Coerce a stored/selected font size to a known stop (invalid → standard). */
export const normalizeAppFontSize = (value: string | null | undefined): AppFontSize => {
  return value === 'small' || value === 'large' || value === 'xlarge' || value === 'standard'
    ? value
    : 'standard';
};

export const [appFontSize, setAppFontSize] = createSignal<AppFontSize>(
  normalizeAppFontSize(localStorage.getItem(APP_FONT_SIZE_KEY)),
);

/** Persist the app font-size stop. */
export const persistAppFontSize = (size: AppFontSize): void => {
  const normalized = normalizeAppFontSize(size);
  setAppFontSize(normalized);
  try {
    localStorage.setItem(APP_FONT_SIZE_KEY, normalized);
  } catch {
    /* restricted storage: the session keeps the selection */
  }
};

/** Multiplier applied to the app's text-size tokens (spacing stays unchanged). */
export const appFontScale = (size: AppFontSize): number => APP_FONT_SIZE_SCALES[size] ?? 1;

/** Whether the user has confirmed allowing auto-start of the local inference engine (first start requires confirmation). */
const AUTO_START_KEY = 'aio-local-auto-start-confirmed';
export const isLocalAutoStartConfirmed = (): boolean => {
  try {
    return localStorage.getItem(AUTO_START_KEY) === '1';
  } catch {
    return false;
  }
};
export const setLocalAutoStartConfirmed = (): void => {
  try {
    localStorage.setItem(AUTO_START_KEY, '1');
  } catch {
    /* ignore */
  }
};

/** Local model market download source preference (P3：localStorage 裸调收编——组件层经此域 API). */
type DownloadSourcePref = 'auto' | 'modelscope' | 'huggingface';
const DOWNLOAD_SOURCE_KEY = 'aio-local-model-source';
export const loadDownloadSourcePref = (): DownloadSourcePref => {
  try {
    const v = localStorage.getItem(DOWNLOAD_SOURCE_KEY);
    if (v === 'auto' || v === 'modelscope' || v === 'huggingface') return v;
  } catch {
    /* ignore */
  }
  return 'auto';
};
export const saveDownloadSourcePref = (s: DownloadSourcePref): void => {
  try {
    localStorage.setItem(DOWNLOAD_SOURCE_KEY, s);
  } catch {
    /* ignore */
  }
};

/**
 * Color target of the shared color picker in "Settings → App settings".
 * 'theme' edits the theme color; the other three edit the background colors, so the
 * control is no longer owned by the theme card alone.
 */
export type ColorEditTarget = 'theme' | 'solid' | 'gradientStart' | 'gradientEnd';
const COLOR_TARGET_KEY = 'aio-color-edit-target';
export const loadColorEditTarget = (): ColorEditTarget => {
  try {
    const v = localStorage.getItem(COLOR_TARGET_KEY);
    if (v === 'theme' || v === 'solid' || v === 'gradientStart' || v === 'gradientEnd') return v;
  } catch {
    /* ignore */
  }
  return 'theme';
};
export const persistColorEditTarget = (target: ColorEditTarget): void => {
  try {
    localStorage.setItem(COLOR_TARGET_KEY, target);
  } catch {
    /* ignore */
  }
};

/**
 * Color targets offered for a background kind, in display order. The first entry is
 * the kind's primary target: the background color for solid/gradient, the theme
 * color otherwise (default/image have no background color to edit).
 */
export const colorTargetsForKind = (kind: AppBackgroundKind): ColorEditTarget[] => {
  if (kind === 'solid') return ['solid', 'theme'];
  if (kind === 'gradient') return ['gradientStart', 'gradientEnd', 'theme'];
  return ['theme'];
};

/**
 * Keep the picker on a target the current kind actually offers.
 * @param kind active background kind
 * @param current target currently selected
 * @returns `current` when still offered, otherwise the kind's primary target
 */
export const resolveColorTarget = (
  kind: AppBackgroundKind,
  current: ColorEditTarget,
): ColorEditTarget => {
  const offered = colorTargetsForKind(kind);
  return offered.includes(current) ? current : offered[0];
};

/** The kind's primary color target (its own background color, or the theme color). */
export const primaryColorTargetForKind = (kind: AppBackgroundKind): ColorEditTarget =>
  colorTargetsForKind(kind)[0];
