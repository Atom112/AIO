// App update state (self-contained domain: signals + localStorage ignored-version record).

import { createSignal } from 'solid-js';

// ====== App update state ======

/** Whether an app update is available. */
export const [appUpdateAvailable, setAppUpdateAvailable] = createSignal(false);

/** Latest release metadata (version, release notes, publish date). */
export interface AppUpdateInfoData {
  version: string;
  currentVersion?: string;
  notes?: string;
  pubDate?: string;
}

/** Latest release metadata. */
export const [appUpdateInfo, setAppUpdateInfo] = createSignal<AppUpdateInfoData | null>(null);

/** Whether the user already dismissed the update notice this session (prevents it from reappearing after a brief flash). */
export const [appUpdateDismissed, setAppUpdateDismissed] = createSignal(false);

/** Whether an update is downloading/installing. */
export const [appUpdateDownloading, setAppUpdateDownloading] = createSignal(false);
/** Download progress (0.0 ~ 1.0). */
export const [appUpdateProgress, setAppUpdateProgress] = createSignal(0);
/** Update fully downloaded, waiting for the user to restart. */
export const [appUpdateReady, setAppUpdateReady] = createSignal(false);

const IGNORED_UPDATE_VERSION_KEY = 'aio_ignored_update_version';

/**
 * Read the user's "ignore this version" record.
 * @returns The ignored version (v prefix stripped), or null if none.
 */
export const getIgnoredUpdateVersion = (): string | null => {
  try {
    return localStorage.getItem(IGNORED_UPDATE_VERSION_KEY);
  } catch {
    return null;
  }
};

/**
 * Record the user's "ignore this version" choice, or clear it.
 * @param version - The version to ignore; pass null to clear.
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
