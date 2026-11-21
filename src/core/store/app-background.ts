// App background customization (single source of truth).
//
// The light/default theme paints `--app-bg` (four corner glows over a dark base) on the
// root `.app-container`; Layout renders a `.app-bg-layer` on top of it when the user
// picks a solid color, a two-color gradient or a custom image. The dark theme keeps
// its own fixed background — this domain is explicitly light-mode only.
//
// Persistence is localStorage-only: the image itself never lives here (a data URL
// would blow the 5 MB quota and the original path could point at a file the user
// later moves). The backend copies the picked file into the app data `backgrounds`
// directory and returns that stable path; the frontend only stores the path string.

import { createSignal, type JSX } from 'solid-js';

/** Which custom layer (if any) replaces/overlays the default `--app-bg`. */
export type AppBackgroundKind = 'default' | 'solid' | 'gradient' | 'image';

/** Persisted background settings; every field is independently validated on load. */
export interface AppBackgroundSettings {
  kind: AppBackgroundKind;
  /** `#rrggbb` solid fill. */
  solidColor: string;
  /** `#rrggbb` linear-gradient start color. */
  gradientStart: string;
  /** `#rrggbb` linear-gradient end color. */
  gradientEnd: string;
  /** Linear-gradient angle in degrees (0-360). */
  gradientAngle: number;
  /** Absolute path of the copied image inside the app data `backgrounds` directory. */
  imagePath: string | null;
  /** Custom layer alpha in percent: <100 lets the default `--app-bg` show through. */
  opacity: number;
}

/** Defaults: the built-in look (no custom layer). Colors mirror `--app-bg` in index.css. */
export const DEFAULT_APP_BACKGROUND: AppBackgroundSettings = {
  kind: 'default',
  // gate-allow: color default solid fill mirrors the --app-bg base in index.css
  solidColor: '#2f2f2f',
  // gate-allow: color default gradient start mirrors the --app-bg base in index.css
  gradientStart: '#2f2f2f',
  // gate-allow: color default gradient end pairs with the --app-bg base tone
  gradientEnd: '#0f172a',
  gradientAngle: 135,
  imagePath: null,
  opacity: 100,
};

const STORAGE_KEY = 'aio-app-background-v1';
const HEX_COLOR_RE = /^#[0-9a-fA-F]{6}$/;
const KINDS: readonly AppBackgroundKind[] = ['default', 'solid', 'gradient', 'image'];

const clampNumber = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

const asColor = (value: unknown, fallback: string): string =>
  typeof value === 'string' && HEX_COLOR_RE.test(value) ? value.toLowerCase() : fallback;

const asNumber = (value: unknown, fallback: number, min: number, max: number): number => {
  if (typeof value !== 'number' || !Number.isFinite(value)) return fallback;
  return clampNumber(Math.round(value), min, max);
};

/**
 * Coerce persisted/partial input into valid settings.
 * Accepts an already-parsed object or a raw JSON string; corrupted JSON, unknown
 * kinds, non-hex colors and out-of-range numbers fall back field by field instead
 * of resetting everything (one broken field must not wipe the user's whole setup).
 * @param input raw localStorage payload or a partial settings object
 * @returns fully populated settings, never null
 */
export function normalizeAppBackground(input: unknown): AppBackgroundSettings {
  let source: unknown = input;
  if (typeof source === 'string') {
    try {
      source = JSON.parse(source);
    } catch {
      return { ...DEFAULT_APP_BACKGROUND };
    }
  }
  if (source === null || typeof source !== 'object' || Array.isArray(source)) {
    return { ...DEFAULT_APP_BACKGROUND };
  }
  const raw = source as Record<string, unknown>;
  const kind = KINDS.includes(raw.kind as AppBackgroundKind)
    ? (raw.kind as AppBackgroundKind)
    : DEFAULT_APP_BACKGROUND.kind;
  return {
    kind,
    solidColor: asColor(raw.solidColor, DEFAULT_APP_BACKGROUND.solidColor),
    gradientStart: asColor(raw.gradientStart, DEFAULT_APP_BACKGROUND.gradientStart),
    gradientEnd: asColor(raw.gradientEnd, DEFAULT_APP_BACKGROUND.gradientEnd),
    gradientAngle: asNumber(raw.gradientAngle, DEFAULT_APP_BACKGROUND.gradientAngle, 0, 360),
    imagePath: typeof raw.imagePath === 'string' && raw.imagePath.length > 0 ? raw.imagePath : null,
    opacity: asNumber(raw.opacity, DEFAULT_APP_BACKGROUND.opacity, 0, 100),
  };
}

const loadPersisted = (): AppBackgroundSettings => {
  try {
    if (typeof localStorage === 'undefined') return { ...DEFAULT_APP_BACKGROUND };
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? normalizeAppBackground(raw) : { ...DEFAULT_APP_BACKGROUND };
  } catch {
    // Storage can be unavailable in restricted WebViews / Node tests — defaults stay valid.
    return { ...DEFAULT_APP_BACKGROUND };
  }
};

/** Active background settings (reactive; drives Layout's custom layer). */
export const [appBackground, setAppBackground] =
  createSignal<AppBackgroundSettings>(loadPersisted());

/**
 * Merge a patch into the settings, normalize the result and persist it.
 * @param patch subset of settings changed by the settings card
 */
export const updateAppBackground = (patch: Partial<AppBackgroundSettings>): void => {
  const next = normalizeAppBackground({ ...appBackground(), ...patch });
  setAppBackground(next);
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // Non-fatal: the session still reflects the change.
  }
};

/** Restore the built-in background (drops the persisted entry entirely). */
export const resetAppBackground = (): void => {
  setAppBackground({ ...DEFAULT_APP_BACKGROUND });
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // Non-fatal.
  }
};

/**
 * Inline style for the `.app-bg-layer` element.
 * Keys are hyphenated CSS property names: Solid's style binding writes them through
 * `CSSStyleDeclaration.setProperty`, so no camelCase conversion is needed. Extends
 * `JSX.CSSProperties` so the object can be passed straight to `style=`.
 */
export interface BackgroundLayerStyle extends JSX.CSSProperties {
  background?: string;
  'background-image'?: string;
  'background-size'?: string;
  'background-position'?: string;
  'background-repeat'?: string;
  opacity?: string;
}

/**
 * Compute the custom background layer style.
 * @param settings active settings
 * @param dark whether the dark theme is active (the dark theme keeps its own background)
 * @param resolveAsset maps a stored image path to an asset-protocol URL (`convertFileSrc`)
 * @returns the layer style, or null when no custom layer should be rendered
 */
export function buildBackgroundLayerStyle(
  settings: AppBackgroundSettings,
  dark: boolean,
  resolveAsset: (path: string) => string,
): BackgroundLayerStyle | null {
  if (dark || settings.kind === 'default') return null;
  const opacity = `${clampNumber(settings.opacity, 0, 100) / 100}`;
  switch (settings.kind) {
    case 'solid':
      return { background: settings.solidColor, opacity };
    case 'gradient':
      return {
        background: `linear-gradient(${settings.gradientAngle}deg, ${settings.gradientStart}, ${settings.gradientEnd})`,
        opacity,
      };
    case 'image': {
      if (!settings.imagePath) return null;
      return {
        'background-image': `url("${resolveAsset(settings.imagePath)}")`,
        'background-size': 'cover',
        'background-position': 'center',
        'background-repeat': 'no-repeat',
        opacity,
      };
    }
    default:
      return null;
  }
}
