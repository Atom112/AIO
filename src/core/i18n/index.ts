import { createMemo, createSignal } from 'solid-js';
import { resolveTemplate, translator } from '@solid-primitives/i18n';
import enUS from './locales/en-US.json';
import zhCN from './locales/zh-CN.json';

export type Locale = 'zh-CN' | 'en-US';
export type TranslationKey = keyof typeof enUS;
export type TranslationParams = Record<string, string | number>;

const STORAGE_KEY = 'aio-locale';
const dictionaries = {
  'zh-CN': zhCN,
  'en-US': enUS,
} as const;

function detectLocale(): Locale {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === 'zh-CN' || saved === 'en-US') return saved;
  } catch {
    // Storage can be unavailable in restricted WebViews; system detection remains safe.
  }
  const systemLocale = navigator.languages?.[0] ?? navigator.language;
  return systemLocale?.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en-US';
}

const initialLocale = detectLocale();
const [localeSignal, setLocaleSignal] = createSignal<Locale>(initialLocale);
const activeDictionary = createMemo(() => dictionaries[localeSignal()]);
// The i18n primitive subscribes to this accessor internally.
// eslint-disable-next-line solid/reactivity
const translate = translator(activeDictionary, resolveTemplate);

export const locale = localeSignal;

export function setLocale(next: Locale): void {
  setLocaleSignal(next);
  document.documentElement.lang = next;
  try {
    localStorage.setItem(STORAGE_KEY, next);
  } catch (error) {
    console.warn('[i18n] Failed to persist locale', error);
  }
}

export function t(key: TranslationKey, params?: TranslationParams): string {
  const value = translate(key, params);
  if (typeof value === 'string') return value;
  const fallback = enUS[key];
  if (import.meta.env.DEV) console.warn(`[i18n] Missing translation: ${key}`);
  return typeof fallback === 'string' ? resolveTemplate(fallback, params ?? {}) : key;
}

export function formatDateTime(
  value: Date | string | number,
  options?: Intl.DateTimeFormatOptions,
): string {
  return new Intl.DateTimeFormat(locale(), options).format(new Date(value));
}

export function formatNumber(value: number, options?: Intl.NumberFormatOptions): string {
  return new Intl.NumberFormat(locale(), options).format(value);
}

export function formatRelativeTime(value: number, unit: Intl.RelativeTimeFormatUnit): string {
  return new Intl.RelativeTimeFormat(locale(), { numeric: 'auto' }).format(value, unit);
}

export function reportError(key: TranslationKey, error: unknown): string {
  console.error(`[i18n:${key}]`, error);
  return t(key);
}

document.documentElement.lang = initialLocale;
