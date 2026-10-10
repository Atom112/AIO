// UI preferences and local persistence (theme / dark mode / reasoning level / search toggle / auto-start confirmation).

import { createSignal } from 'solid-js';

/** Theme color signal, read from localStorage or defaulted to soft-mist blue #7c9abf (must be hex — theme effects parse RGB by hex slicing). */
export const [themeColor, setThemeColor] = createSignal(
  localStorage.getItem('theme-color') || '#7c9abf',
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
