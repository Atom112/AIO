// 应用更新状态 —— 从 store.ts 拆出（自包含域：仅信号 + localStorage 忽略版本）。

import { createSignal } from 'solid-js';

// ====== 应用更新状态 ======

/** 是否有可用的应用更新 */
export const [appUpdateAvailable, setAppUpdateAvailable] = createSignal(false);

/** 最新版本元信息（版本号、Release notes、发布时间） */
export interface AppUpdateInfoData {
  version: string;
  currentVersion?: string;
  notes?: string;
  pubDate?: string;
}

/** 最新版本元信息 */
export const [appUpdateInfo, setAppUpdateInfo] = createSignal<AppUpdateInfoData | null>(null);

/** 用户在当前会话中是否已经手动关闭过更新提示（避免短暂显示后又跳出来） */
export const [appUpdateDismissed, setAppUpdateDismissed] = createSignal(false);

/** 是否正在下载/安装更新 */
export const [appUpdateDownloading, setAppUpdateDownloading] = createSignal(false);
/** 下载进度 (0.0 ~ 1.0) */
export const [appUpdateProgress, setAppUpdateProgress] = createSignal(0);
/** 更新已下载完毕，等待用户重启 */
export const [appUpdateReady, setAppUpdateReady] = createSignal(false);

const IGNORED_UPDATE_VERSION_KEY = 'aio_ignored_update_version';

/**
 * 读取用户"忽略此版本"的记录
 * @returns 被忽略的最新版本号（带 v 前缀已剥离），没有则返回 null
 */
export const getIgnoredUpdateVersion = (): string | null => {
  try {
    return localStorage.getItem(IGNORED_UPDATE_VERSION_KEY);
  } catch {
    return null;
  }
};

/**
 * 记录用户"忽略此版本"或清除记录
 * @param version - 要忽略的版本号；传 null 则清除
 */
export const setIgnoredUpdateVersion = (version: string | null): void => {
  try {
    if (version) {
      localStorage.setItem(IGNORED_UPDATE_VERSION_KEY, version);
    } else {
      localStorage.removeItem(IGNORED_UPDATE_VERSION_KEY);
    }
  } catch (e) {
    console.warn('保存更新忽略记录失败:', e);
  }
};
