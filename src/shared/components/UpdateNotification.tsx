import { Component, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { installAppUpdate, restartApp } from '../../core/api/update';
import { listen } from '@tauri-apps/api/event';
import { EVENTS } from '../../core/api/events';
import { Transition } from 'solid-transition-group';
import Icon from './Icon';
import {
  appUpdateAvailable,
  appUpdateInfo,
  appUpdateDismissed,
  setAppUpdateDismissed,
  appUpdateDownloading,
  setAppUpdateDownloading,
  appUpdateProgress,
  setAppUpdateProgress,
  appUpdateReady,
  setAppUpdateReady,
  setIgnoredUpdateVersion,
} from '../../core/store/store';
import { formatNumber, t } from '../../core/i18n';
import { btnPrimaryOutline, btnSecondary } from '../components/buttonStyles';

/**
 * App update toast pinned to the bottom-left corner
 *
 * Shown when:
 * 1. A new version is found at startup or via manual check (appUpdateAvailable() = true)
 * 2. The user has not dismissed it this session (appUpdateDismissed() = false)
 * 3. Not hidden by the "already up to date" state
 *
 * Actions:
 * - Install now  -> calls backend `install_app_update`, shows a progress bar meanwhile
 * - Later        -> stores the version in localStorage and closes the toast
 * - Restart app  -> calls `restart_app` once the download completes
 */
const UpdateNotification: Component = () => {
  const [error, setError] = createSignal<string | null>(null);
  let unlistenProgress: (() => void) | null = null;

  /**
   * Handles "Install now": streams the download and install, showing progress
   */
  const handleUpdate = async () => {
    const info = appUpdateInfo();
    if (!info) return;

    setError(null);
    setAppUpdateDownloading(true);
    setAppUpdateProgress(0);
    setAppUpdateReady(false);

    try {
      await installAppUpdate();
      setAppUpdateProgress(1);
      setAppUpdateReady(true);
    } catch (e) {
      console.error('Update download failed:', e);
      setError(t('update.downloadFailed'));
    } finally {
      setAppUpdateDownloading(false);
    }
  };

  /**
   * Handles "Later": stores the version in localStorage and closes the toast
   */
  const handleDismiss = () => {
    const info = appUpdateInfo();
    if (info?.version) {
      setIgnoredUpdateVersion(info.version);
    }
    setAppUpdateDismissed(true);
  };

  /**
   * Handles "Restart app": calls backend `restart_app` to apply the update
   */
  const handleRestart = async () => {
    try {
      await restartApp();
    } catch (e) {
      console.error('Restart failed:', e);
      setError(t('update.restartFailed'));
    }
  };

  onMount(async () => {
    // Listen for backend download progress events (0.0 ~ 1.0)
    try {
      unlistenProgress = await listen<number>(EVENTS.appUpdateProgress, (event) => {
        const pct = typeof event.payload === 'number' ? event.payload : 0;
        setAppUpdateProgress(Math.max(0, Math.min(1, pct)));
      });
    } catch (e) {
      console.warn('Failed to listen to app-update-progress:', e);
    }
  });

  onCleanup(() => {
    if (unlistenProgress) unlistenProgress();
  });

  return (
    <Transition name="update-toast">
      <Show when={appUpdateAvailable() && !appUpdateDismissed() && appUpdateInfo()}>
        <div
          class="fixed bottom-5 left-5 z-[9999] w-[340px] rounded-2xl overflow-hidden select-none"
          style={{
            background: 'var(--acrylic-bg)',
            'backdrop-filter': 'blur(var(--acrylic-blur))',
            border: '1px solid var(--border-dim)',
            'box-shadow': '0 12px 40px rgba(0, 0, 0, 0.45)',
          }}
        >
          <div class="p-4 flex flex-col gap-3">
            {/* Title row */}
            <div class="flex items-center gap-2.5">
              <div
                class="w-9 h-9 rounded-xl flex items-center justify-center shrink-0"
                style={{
                  background: 'rgba(var(--primary-rgb), 0.18)',
                  color: 'var(--primary-color)',
                }}
              >
                <Icon src="/icons/app-logo/top.svg" class="w-5 h-5" />
              </div>
              <div class="flex-1 min-w-0">
                <div class="text-sm font-semibold text-white leading-tight">
                  {t('update.foundVersion', { version: appUpdateInfo()!.version })}
                </div>
                <div class="text-[11px] text-[rgba(var(--text-base-rgb),0.55)] mt-0.5">
                  {t('update.currentVersion', { version: appUpdateInfo()!.currentVersion || '?' })}
                </div>
              </div>
            </div>

            {/* Release notes summary (shown only when non-empty) */}
            <Show when={appUpdateInfo()!.notes}>
              <div
                class="text-[12px] text-[rgba(var(--text-base-rgb),0.73)] leading-relaxed rounded-lg p-2.5 max-h-[88px] overflow-y-auto"
                style={{
                  background: 'rgba(var(--text-base-rgb), 0.04)',
                  border: '1px solid var(--border-dim)',
                  'white-space': 'pre-wrap',
                  'word-break': 'break-word',
                }}
              >
                {appUpdateInfo()!.notes!.length > 220
                  ? appUpdateInfo()!.notes!.slice(0, 220) + '…'
                  : appUpdateInfo()!.notes}
              </div>
            </Show>

            {/* Error message */}
            <Show when={error()}>
              <div
                class="text-[12px] rounded-lg px-3 py-2"
                style={{
                  background: 'rgba(255, 80, 80, 0.1)',
                  color: 'var(--color-danger)',
                  border: '1px solid rgba(255, 80, 80, 0.2)',
                }}
              >
                {error()}
              </div>
            </Show>

            {/* Progress bar: shown only while downloading or awaiting restart */}
            <Show when={appUpdateDownloading() || appUpdateReady()}>
              <div class="flex flex-col gap-1.5">
                <div class="flex justify-between text-[11px] text-[rgba(var(--text-base-rgb),0.55)]">
                  <span>{appUpdateReady() ? t('update.ready') : t('update.downloading')}</span>
                  <span class="font-mono">
                    {formatNumber(Math.round(appUpdateProgress() * 100))}%
                  </span>
                </div>
                <div
                  class="h-1.5 rounded-full overflow-hidden"
                  style={{ background: 'rgba(var(--text-base-rgb), 0.08)' }}
                >
                  <div
                    class="h-full rounded-full transition-all duration-200"
                    style={{
                      width: `${Math.round(appUpdateProgress() * 100)}%`,
                      background:
                        'linear-gradient(90deg, var(--primary-color), color-mix(in srgb, var(--primary-color), white 30%))',
                    }}
                  />
                </div>
              </div>
            </Show>

            {/* Action buttons */}
            <div class="flex gap-2 mt-1">
              <Show
                when={!appUpdateReady()}
                fallback={
                  <button class={'flex-1 ' + btnPrimaryOutline} onClick={handleRestart}>
                    {t('update.restart')}
                  </button>
                }
              >
                <button
                  class={'flex-1 ' + btnPrimaryOutline}
                  disabled={appUpdateDownloading()}
                  onClick={handleUpdate}
                >
                  {appUpdateDownloading() ? t('update.downloading') : t('update.install')}
                </button>
                <button class={btnSecondary} onClick={handleDismiss}>
                  {t('update.later')}
                </button>
              </Show>
            </div>
          </div>
        </div>
      </Show>
    </Transition>
  );
};

export default UpdateNotification;
