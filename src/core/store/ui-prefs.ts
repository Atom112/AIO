// UI 偏好与本地持久化（主题/深色/推理强度/搜索开关/头像/自启确认）—— 从 store.ts 拆出。

import { createSignal } from 'solid-js';
import { readFile } from '@tauri-apps/plugin-fs';

/** 全局用户头像状态信号，默认使用系统默认头像 */
export const [globalUserAvatar, setGlobalUserAvatar] = createSignal('/icons/app-logo/user.svg');
/** 主题颜色信号，从本地存储读取或使用默认柔雾蓝 #7c9abf（必须为 hex——主题效果按 hex 切片解析 RGB） */
export const [themeColor, setThemeColor] = createSignal(
  localStorage.getItem('theme-color') || '#7c9abf',
);
/** 深色模式信号，从本地存储读取 */
export const [isDarkMode, setIsDarkMode] = createSignal(
  localStorage.getItem('dark-mode') === 'true',
);

/** 推理强度 (off=关闭 / low=轻度 / medium=中度 / high=深度) */
export type ReasoningLevel = 'off' | 'low' | 'medium' | 'high';
const REASONING_KEY = 'chat-reasoning-level';
export const [reasoningLevel, setReasoningLevel] = createSignal<ReasoningLevel>(
  (localStorage.getItem(REASONING_KEY) as ReasoningLevel) || 'off',
);
export const persistReasoningLevel = (lvl: ReasoningLevel) => {
  setReasoningLevel(lvl);
  localStorage.setItem(REASONING_KEY, lvl);
};
/** 网页搜索开关 */
const WEB_SEARCH_KEY = 'chat-web-search';
export const [webSearchEnabled, setWebSearchEnabled] = createSignal<boolean>(
  localStorage.getItem(WEB_SEARCH_KEY) === 'true',
);
export const persistWebSearch = (enabled: boolean) => {
  setWebSearchEnabled(enabled);
  localStorage.setItem(WEB_SEARCH_KEY, String(enabled));
};

/** 缓存上一次生成的 Blob URL，用于内存管理 */
let lastBlobUrl: string | null = null;

/** M13: 用户是否已确认允许自动启动本地推理引擎（首次启动需用户确认） */
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

/**
 * 从指定路径加载用户头像
 * @param input - 头像数据源，可以是 Base64 字符串或文件系统路径
 * @returns 头像的可访问 URL 字符串
 */
export const loadAvatarFromPath = async (input: string): Promise<string> => {
  if (!input) return '/icons/app-logo/user.svg';

  // 直接返回已可访问的 URL（Base64 / 网络地址）
  if (
    input.startsWith('data:image') ||
    input.startsWith('http://') ||
    input.startsWith('https://')
  ) {
    return input;
  }

  try {
    const contents = await readFile(input);
    const ext = input.split('.').pop()?.toLowerCase();
    const mime = ext === 'svg' ? 'image/svg+xml' : `image/${ext === 'jpg' ? 'jpeg' : ext}`;
    const blob = new Blob([contents], { type: mime });

    // 释放之前的 Blob URL 以避免内存泄漏
    if (lastBlobUrl) {
      URL.revokeObjectURL(lastBlobUrl);
    }

    lastBlobUrl = URL.createObjectURL(blob);
    return lastBlobUrl;
  } catch (e) {
    console.error('从物理路径加载头像失败:', e, '路径:', input);
    return '/icons/app-logo/user.svg';
  }
};
