import {
  Component,
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
} from 'solid-js';
import { openSafeUrl } from '../../../core/utils/safeUrl';
import CloudAccountCard from './CloudAccountCard';
import FeedbackSection from './FeedbackSection';
import {
  loadAppConfig,
  patchAppConfig,
  setAutoStart as apiSetAutoStart,
} from '../../../core/api/config';
import { checkAppUpdate, type CheckUpdateResult } from '../../../core/api/update';
import {
  setThemeColor,
  themeColor,
  isDarkMode,
  setIsDarkMode,
  setAppUpdateAvailable,
  setAppUpdateInfo,
  setAppUpdateDismissed,
  appBackground,
  updateAppBackground,
  resetAppBackground,
  loadColorEditTarget,
  persistColorEditTarget,
  colorTargetsForKind,
  resolveColorTarget,
  primaryColorTargetForKind,
  appFont,
  persistAppFont,
  appFontStack,
  appFontSize,
  persistAppFontSize,
  type AppFontSize,
  type AppBackgroundKind,
  type ColorEditTarget,
} from '../../../core/store/store';
import {
  getRegisteredCommands,
  getShortcutKeys,
  setShortcutBinding,
  resetShortcutBinding,
  resetAllShortcutBindings,
  isShortcutModified,
  getActionByKeys,
  normalizeKeyCombo,
  formatShortcutForDisplay,
  getCommandDisplayLabel,
  getCommandDisplayDescription,
  setShortcutRecording,
  type CommandAction,
} from '../../../core/shortcuts';
import { getVersion } from '@tauri-apps/api/app';
import { convertFileSrc } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { saveAppBackground, deleteAppBackground } from '../../../core/api/appearance';
import { listInstalledFonts } from '../../../core/api/fonts';
import ColorPickerPanel from './ColorPickerPanel';
import Icon from '../../../shared/components/Icon';
import NumberInput from '../../../shared/components/NumberInput';
import Switch from '../../../shared/components/Switch';
import Dropdown from '../../../shared/components/Dropdown';
import StopSlider from '../../../shared/components/StopSlider';
import {
  btnDangerOutline,
  btnIcon,
  btnPrimaryOutline,
  btnSecondary,
  btnSecondarySm,
  btnTabXs,
  tabActive,
  tabInactive,
} from '../../../shared/components/buttonStyles';
import { locale, setLocale, t, type Locale, type TranslationKey } from '../../../core/i18n';

/**
 * Structured result returned by the backend check_app_update (mirrors src-tauri/src/commands/update.rs)
 * The `kind` field acts as the tag; the frontend shows a different hint per kind
 */

/** Labels of the color targets offered by the shared picker (order follows the store helper). */
const COLOR_TARGET_LABEL_KEYS: Record<ColorEditTarget, TranslationKey> = {
  theme: 'app.theme.title',
  solid: 'app.background.backgroundColor',
  gradientStart: 'app.background.startColor',
  gradientEnd: 'app.background.endColor',
};

/**
 * App settings page component
 * @returns {JSX.Element} The app settings page JSX element
 */
const AppSettings: Component = () => {
  const [autoStart, setAutoStart] = createSignal(false);
  const [maxToolRounds, setMaxToolRounds] = createSignal(25); // Max tool-call rounds per agent turn (0 = unlimited; default 25)
  const [maxConcurrentSubagents, setMaxConcurrentSubagents] = createSignal(5); // Max concurrent subagents (0 = unlimited; default 5)

  const [version, setVersion] = createSignal(''); // App version
  const [checkUpdating, setCheckUpdating] = createSignal(false); // Manual update check in progress
  const [checkResult, setCheckResult] = createSignal<CheckUpdateResult | null>(null); // Result of the last manual check

  // ---- Shortcut settings state ----
  const [recordingActionId, setRecordingActionId] = createSignal<string | null>(null); // Command ID currently being recorded
  const [conflictDialog, setConflictDialog] = createSignal<{
    actionId: string;
    newKeys: string;
    conflictActionId: string;
  } | null>(null); // Conflict confirmation dialog state
  const [resetAllConfirm, setResetAllConfirm] = createSignal(false); // Reset-all confirmation state

  // ---- App background (light mode) state ----
  const [backgroundError, setBackgroundError] = createSignal(''); // Last image save/delete failure
  const backgroundKinds: { kind: AppBackgroundKind; labelKey: TranslationKey }[] = [
    { kind: 'default', labelKey: 'app.background.default' },
    { kind: 'solid', labelKey: 'app.background.solid' },
    { kind: 'gradient', labelKey: 'app.background.gradient' },
    { kind: 'image', labelKey: 'app.background.image' },
  ];
  const backgroundSliderTrack = {
    background:
      'linear-gradient(to right, rgba(var(--text-base-rgb), 0.08), rgba(var(--text-base-rgb), 0.4))',
  };

  // ---- App font state ----
  const [installedFonts, setInstalledFonts] = createSignal<string[]>([]);
  const [fontsLoading, setFontsLoading] = createSignal(false);
  const [fontScanError, setFontScanError] = createSignal('');
  /** System default + scanned families (+ the persisted one if it is no longer installed). */
  const fontOptions = createMemo(() => {
    const options: Array<{ value: string; label: string; fontFamily?: string }> = [
      { value: '', label: t('app.font.system') },
    ];
    const list = installedFonts();
    for (const font of list) {
      options.push({ value: font, label: font, fontFamily: appFontStack(font) });
    }
    const current = appFont();
    if (current && !list.includes(current)) {
      options.push({ value: current, label: current, fontFamily: appFontStack(current) });
    }
    return options;
  });
  /** App font-size stops; `standard` is the current default size. */
  const fontSizeOptions = createMemo(() => [
    { value: 'small' as AppFontSize, label: t('app.font.size.small') },
    { value: 'standard' as AppFontSize, label: t('app.font.size.standard') },
    { value: 'large' as AppFontSize, label: t('app.font.size.large') },
    { value: 'xlarge' as AppFontSize, label: t('app.font.size.xlarge') },
  ]);
  const scanInstalledFonts = async () => {
    setFontsLoading(true);
    setFontScanError('');
    try {
      setInstalledFonts(await listInstalledFonts());
    } catch (e) {
      setFontScanError(String(e));
    } finally {
      setFontsLoading(false);
    }
  };

  // ---- Shared color picker target (theme color or one of the background colors) ----
  const [colorTarget, setColorTarget] = createSignal<ColorEditTarget>(loadColorEditTarget());
  const colorTargetOptions = createMemo(() =>
    colorTargetsForKind(appBackground().kind).map((id) => ({
      id,
      labelKey: COLOR_TARGET_LABEL_KEYS[id],
    })),
  );
  /** Currently edited color (drives the picker's ring/sliders/hex). */
  const targetColor = () => {
    switch (colorTarget()) {
      case 'solid':
        return appBackground().solidColor;
      case 'gradientStart':
        return appBackground().gradientStart;
      case 'gradientEnd':
        return appBackground().gradientEnd;
      default:
        return themeColor();
    }
  };
  /** Persist a color into whichever domain owns the active target. */
  const applyTargetColor = (hex: string) => {
    switch (colorTarget()) {
      case 'solid':
        updateAppBackground({ solidColor: hex });
        break;
      case 'gradientStart':
        updateAppBackground({ gradientStart: hex });
        break;
      case 'gradientEnd':
        updateAppBackground({ gradientEnd: hex });
        break;
      default:
        setThemeColor(hex);
    }
  };
  const selectColorTarget = (target: ColorEditTarget) => {
    setColorTarget(target);
    persistColorEditTarget(target);
  };
  // Switching the background kind can remove the active target (e.g. gradient-only
  // colors after switching to solid); fall back to the kind's primary target so the
  // picker always shows a color the current settings actually own.
  createEffect(() => {
    const resolved = resolveColorTarget(appBackground().kind, colorTarget());
    if (resolved !== colorTarget()) selectColorTarget(resolved);
  });

  let recordingCleanup: (() => void) | null = null; // Recording-mode cleanup function

  // ---- FE-07: debounced single-IPC config patches ----
  // A keystroke no longer triggers a full load→save round-trip: only the changed
  // field is sent (backend merges it atomically in `patch_app_config`), debounced
  // per field so fast typing collapses into one call.
  const patchTimers: Record<string, ReturnType<typeof setTimeout>> = {};
  const schedulePatch = (field: string, run: () => Promise<void>, delayMs = 400) => {
    clearTimeout(patchTimers[field]);
    patchTimers[field] = setTimeout(() => {
      delete patchTimers[field];
      void run();
    }, delayMs);
  };
  onCleanup(() => {
    Object.values(patchTimers).forEach((id) => clearTimeout(id));
  });

  /**
   * Fetch the app version and load the persistent app config.
   */
  onMount(async () => {
    void scanInstalledFonts();

    try {
      const v = await getVersion();
      setVersion(v);
    } catch (e) {
      console.error('Failed to fetch version', e);
    }

    // Load the auto-start config
    try {
      const cfg = await loadAppConfig();
      if (typeof cfg?.autoStartEnabled === 'boolean') {
        setAutoStart(cfg.autoStartEnabled);
      }
      if (typeof cfg?.maxToolRounds === 'number') {
        setMaxToolRounds(cfg.maxToolRounds);
      }
      if (typeof cfg?.maxConcurrentSubagents === 'number') {
        setMaxConcurrentSubagents(cfg.maxConcurrentSubagents);
      }
    } catch (e) {
      console.warn('Failed to load app config:', e);
    }
  });

  /**
   * Pick an image through the native dialog, copy it into app data and switch the
   * background to it. The previously stored copy (if any) is deleted best-effort.
   */
  const chooseBackgroundImage = async () => {
    try {
      const picked = await open({
        multiple: false,
        directory: false,
        filters: [
          { name: t('app.background.imageFilter'), extensions: ['png', 'jpg', 'jpeg', 'webp'] },
        ],
      });
      if (typeof picked !== 'string') return;
      const previous = appBackground().imagePath;
      const stored = await saveAppBackground(picked);
      updateAppBackground({ kind: 'image', imagePath: stored });
      if (previous && previous !== stored) {
        void deleteAppBackground(previous).catch((e) =>
          console.error('delete_app_background failed:', e),
        );
      }
      setBackgroundError('');
    } catch (e) {
      setBackgroundError(String(e));
    }
  };

  /** Drop the stored image, then fall back to the built-in background. */
  const clearBackgroundImage = async () => {
    const current = appBackground().imagePath;
    if (current) {
      try {
        await deleteAppBackground(current);
      } catch (e) {
        console.error('delete_app_background failed:', e);
      }
    }
    updateAppBackground({ imagePath: null, kind: 'default' });
    setBackgroundError('');
  };

  /** Reset every background setting (removing the stored image first). */
  const resetBackground = () => {
    const current = appBackground().imagePath;
    if (current) {
      void deleteAppBackground(current).catch((e) =>
        console.error('delete_app_background failed:', e),
      );
    }
    resetAppBackground();
    setBackgroundError('');
  };

  /**
   * Manual update check: calls backend check_app_update, then updates toast state and button hint from the structured result
   */
  const handleManualCheck = async () => {
    setCheckUpdating(true);
    setCheckResult(null);
    try {
      const result: CheckUpdateResult = await checkAppUpdate();
      setCheckResult(result);

      switch (result.kind) {
        case 'update_available':
          setAppUpdateInfo({
            version: result.info.version,
            currentVersion: result.info.currentVersion,
            notes: result.info.notes,
            pubDate: result.info.pubDate,
          });
          setAppUpdateDismissed(false);
          setAppUpdateAvailable(true);
          break;
        case 'up_to_date':
        case 'service_not_ready':
        case 'network':
        case 'failed':
          // These kinds never show the bottom-left toast
          break;
      }
    } catch (e) {
      console.error('Manual update check failed:', e);
      setCheckResult({
        kind: 'failed',
        current_version: version(),
        endpoint: '',
        reason:
          typeof e === 'string' ? e : e instanceof Error ? e.message : t('common.unknownError'),
      });
    } finally {
      setCheckUpdating(false);
    }
  };

  /**
   * Map the structured check result to the hint text shown next to the button
   */
  const checkResultMessage = (): string => {
    const r = checkResult();
    if (!r) return '';
    switch (r.kind) {
      case 'up_to_date':
        return t('app.update.latest');
      case 'update_available':
        return t('app.update.found');
      case 'service_not_ready':
        return t('app.update.unavailable');
      case 'network':
        return t('app.update.network');
      case 'failed':
        return t('error.update');
    }
  };

  // ---- Shortcut recording logic ----

  /** Start recording a shortcut */
  const startRecording = (actionId: string) => {
    // Clean up any previous recording first
    if (recordingCleanup) {
      recordingCleanup();
      recordingCleanup = null;
    }
    setRecordingActionId(actionId);
    // P3（录制互斥）：通知全局键盘处理器让位
    setShortcutRecording(true);

    const onKeyDown = (e: KeyboardEvent) => {
      const combo = normalizeKeyCombo(e);
      if (!combo) return;

      e.preventDefault();
      e.stopPropagation();

      // Check for conflicts
      const conflictId = getActionByKeys(combo);
      if (conflictId && conflictId !== actionId) {
        // Conflict found: show the confirmation dialog
        setConflictDialog({
          actionId,
          newKeys: combo,
          conflictActionId: conflictId,
        });
      } else {
        // No conflict: apply directly
        setShortcutBinding(actionId, combo);
      }

      // Stop recording
      stopRecording();
    };

    document.addEventListener('keydown', onKeyDown, { capture: true });
    recordingCleanup = () => {
      document.removeEventListener('keydown', onKeyDown, { capture: true });
    };
  };

  /** Stop recording */
  const stopRecording = () => {
    setShortcutRecording(false);
    setRecordingActionId(null);
    if (recordingCleanup) {
      recordingCleanup();
      recordingCleanup = null;
    }
  };

  /** Handle conflict confirmation: override */
  const handleConflictOverride = () => {
    const d = conflictDialog();
    if (d) {
      setShortcutBinding(d.actionId, d.newKeys);
    }
    setConflictDialog(null);
  };

  /** Handle conflict confirmation: cancel */
  const handleConflictCancel = () => {
    setConflictDialog(null);
  };

  // Clicking elsewhere on the page cancels recording
  const onDocClick = (e: MouseEvent) => {
    if (recordingActionId()) {
      // Check whether the click is on a shortcut badge.
      // FE-02: data-* 属性选择器——整段 Tailwind class 串不是合法 CSS 选择器
      // （closest 直接抛 SyntaxError 且无 catch，录制永远无法通过点击取消，
      // Escape 也会被误录成绑定）
      const target = e.target as HTMLElement;
      if (!target.closest('[data-shortcut-badge]')) {
        stopRecording();
      }
    }
  };

  onMount(() => {
    document.addEventListener('click', onDocClick);
  });

  onCleanup(() => {
    document.removeEventListener('click', onDocClick);
    if (recordingCleanup) recordingCleanup();
  });

  // All registered commands (displayed in the settings list)
  const allCommands = createMemo(() => getRegisteredCommands());

  // Grouped by category
  const groupedCommands = createMemo(() => {
    const groups: { category: string; label: string; items: CommandAction[] }[] = [];
    const seen = new Set<string>();
    const CAT_LABELS: Record<string, string> = {
      navigation: t('command.category.navigation'),
      chat: t('command.category.chat'),
      sidebar: t('command.category.sidebar'),
      global: t('command.category.global'),
    };

    for (const cmd of allCommands()) {
      if (!seen.has(cmd.category)) {
        seen.add(cmd.category);
        groups.push({
          category: cmd.category,
          label: CAT_LABELS[cmd.category] || cmd.category,
          items: [],
        });
      }
      groups.find((g) => g.category === cmd.category)!.items.push(cmd);
    }
    return groups;
  });

  const langOptions = createMemo(() => [
    { value: 'zh-CN' as Locale, label: t('app.language.zhCN') },
    { value: 'en-US' as Locale, label: t('app.language.enUS') },
  ]);

  return (
    <div class="flex flex-col gap-[15px] box-border">
      <CloudAccountCard />
      <div
        class="rounded-xl p-6 animate-row-in"
        style={{
          background: 'rgba(var(--text-base-rgb), 0.035)',
          'backdrop-filter': 'blur(16px)',
          border: '1px solid var(--border-dim)',
        }}
      >
        <div class="flex justify-between items-center mb-5">
          <h3 class="m-0 text-base text-white">{t('app.status.title')}</h3>
          <div class="flex items-center gap-2">
            <span class="text-xs text-white/45 font-medium">{t('app.status.version')}:</span>
            <div
              class="text-base font-bold px-2.5 py-0.5 rounded-full font-mono whitespace-nowrap"
              style={{
                background: 'rgba(var(--text-base-rgb), 0.035)',
                color: 'rgba(var(--text-base-rgb),0.8)',
                'font-family': "'JetBrains Mono', monospace",
              }}
            >
              v{version()}
            </div>
          </div>
        </div>

        <div class="flex justify-between items-center py-3 border-b border-white/5">
          <div class="pr-6">
            <span class="block text-[rgba(var(--text-base-rgb),0.93)] text-[14px]">
              {t('app.language.title')}
            </span>
          </div>
          <Dropdown
            variant="ghost"
            class="min-w-[150px]"
            value={locale()}
            onChange={(v) => setLocale(v)}
            options={langOptions()}
          />
        </div>

        <div class="flex justify-between items-center py-3 border-b border-white/5">
          <div class="pr-6">
            <span class="block text-[rgba(var(--text-base-rgb),0.93)] text-[14px]">
              {t('app.darkMode.title')}
            </span>
          </div>
          <div class="flex rounded-md border border-white/10 overflow-hidden">
            <button
              type="button"
              class={btnTabXs}
              classList={{ [tabActive]: !isDarkMode(), [tabInactive]: isDarkMode() }}
              onClick={() => setIsDarkMode(false)}
            >
              {t('app.darkMode.off')}
            </button>
            <button
              type="button"
              class={btnTabXs}
              classList={{ [tabActive]: isDarkMode(), [tabInactive]: !isDarkMode() }}
              onClick={() => setIsDarkMode(true)}
            >
              {t('app.darkMode.on')}
            </button>
          </div>
        </div>

        <div class="flex justify-between items-center py-3 border-b border-white/5">
          <div>
            <span class="block text-[rgba(var(--text-base-rgb),0.93)] text-[14px]">
              {t('app.autoStart.title')}
            </span>
          </div>

          {/* FE-10: shared Switch (last hand-copied toggle in the settings pages) */}
          <Switch
            checked={autoStart()}
            onChange={(val) => {
              setAutoStart(val);
              void (async () => {
                try {
                  await apiSetAutoStart(val);
                } catch (err) {
                  console.warn('Failed to set auto-start:', err);
                  setAutoStart(!val); // Roll back
                }
              })();
            }}
            variant="glow"
            label={t('app.autoStart.title')}
          />
        </div>

        <div class="flex justify-between items-center py-3 border-b border-white/5">
          <div>
            <span class="block text-[rgba(var(--text-base-rgb),0.93)] text-[14px]">
              {t('app.maxToolRounds.title')}
            </span>
            <p class="text-xs text-white/35 mt-1">{t('app.maxToolRounds.description')}</p>
          </div>

          <NumberInput
            value={maxToolRounds()}
            min={-1}
            max={2000}
            fallback={25}
            onInput={(clamped) => {
              // 0 is forbidden: snap it PAST in the direction of travel — stepping up
              // from -1 lands on 1, stepping down from 1 (or typing 0) lands on -1
              const v = clamped === 0 ? (maxToolRounds() < 0 ? 1 : -1) : clamped;
              setMaxToolRounds(v);
              schedulePatch('maxToolRounds', async () => {
                try {
                  await patchAppConfig({ maxToolRounds: v });
                } catch (err) {
                  console.warn('Failed to save max-tool-rounds config:', err);
                }
              });
            }}
          />
        </div>

        <div class="flex justify-between items-center py-3 border-b border-white/5">
          <div>
            <span class="block text-[rgba(var(--text-base-rgb),0.93)] text-[14px]">
              {t('app.maxConcurrentSubagents.title')}
            </span>
            <p class="text-xs text-white/35 mt-1">{t('app.maxConcurrentSubagents.description')}</p>
          </div>

          <NumberInput
            value={maxConcurrentSubagents()}
            min={-1}
            max={32}
            fallback={5}
            onInput={(clamped) => {
              // 0 is forbidden: snap it PAST in the direction of travel — stepping up
              // from -1 lands on 1, stepping down from 1 (or typing 0) lands on -1
              const v = clamped === 0 ? (maxConcurrentSubagents() < 0 ? 1 : -1) : clamped;
              setMaxConcurrentSubagents(v);
              schedulePatch('maxConcurrentSubagents', async () => {
                try {
                  await patchAppConfig({ maxConcurrentSubagents: v });
                } catch (err) {
                  console.warn('Failed to save concurrent-subagent cap config:', err);
                }
              });
            }}
          />
        </div>

        <div class="flex justify-between items-center py-3 border-b border-white/5">
          <div>
            <span class="block text-[rgba(var(--text-base-rgb),0.93)] text-[14px]">
              {t('app.openSource.title')}
            </span>
          </div>

          <button
            type="button"
            class={btnSecondarySm + ' min-w-[112px]'}
            onClick={() => void openSafeUrl('https://github.com/Atom112/AIO')}
            title={t('app.openSource.visit')}
          >
            <Icon src="/icons/app-logo/github.svg" class="w-4 h-4" />
            <span class="text-sm font-medium">GitHub</span>
          </button>
        </div>

        <div class="flex justify-between items-center py-3">
          <div class="flex-1 min-w-0 pr-4">
            <span class="block text-[rgba(var(--text-base-rgb),0.93)] text-[14px]">
              {t('app.update.title')}
            </span>
            <Show when={checkResult()}>
              <p
                class="text-xs mt-1"
                style={{
                  color: (() => {
                    const r = checkResult()!;
                    if (r.kind === 'update_available') return 'var(--primary-color)';
                    if (
                      r.kind === 'service_not_ready' ||
                      r.kind === 'failed' ||
                      r.kind === 'network'
                    )
                      return '#d99'; // gate-allow: color return #d99
                    return 'var(--color-pri)';
                  })(),
                }}
              >
                {checkResultMessage()}
              </p>
            </Show>
          </div>

          <button
            class={btnSecondarySm + ' min-w-[112px]'}
            disabled={checkUpdating()}
            onClick={handleManualCheck}
            title={t('app.update.check')}
          >
            <Icon src="/icons/app-logo/switch-arrows.svg" class="w-4 h-4" />
            <span class="text-sm font-medium">
              {checkUpdating() ? t('app.update.checking') : t('app.update.check')}
            </span>
          </button>
        </div>
      </div>

      {/* App background (custom layer, light mode only) + shared color picker */}
      <div
        class="bg-[rgb(255_255_255/0.04)] rounded-xl p-6 animate-row-in"
        style={{
          'backdrop-filter': 'blur(var(--acrylic-blur))',
          border: '1px solid var(--acrylic-border)',
          'border-radius': 'var(--acrylic-radius)',
          'animation-delay': '45ms',
        }}
      >
        <div class="flex justify-between items-center mb-5">
          <h3 class="m-0 text-base text-white">{t('app.background.title')}</h3>
          <Show when={isDarkMode()}>
            <span class="text-[11px] text-white/40">{t('app.background.lightOnly')}</span>
          </Show>
        </div>

        <div class="flex flex-col gap-4">
          <div class="flex items-center gap-1 p-1 rounded-lg bg-white/[0.04] border border-white/10 w-fit">
            <For each={backgroundKinds}>
              {(option) => (
                <button
                  class={`${btnTabXs} ${appBackground().kind === option.kind ? tabActive : tabInactive}`}
                  onClick={() => {
                    if (appBackground().kind === option.kind) return;
                    updateAppBackground({ kind: option.kind });
                    // Switching the settings content also switches what the picker edits.
                    selectColorTarget(primaryColorTargetForKind(option.kind));
                  }}
                >
                  {t(option.labelKey)}
                </button>
              )}
            </For>
          </div>

          <div class="flex flex-col gap-3">
            <div class="flex items-center gap-1 p-1 rounded-lg bg-white/[0.04] border border-white/10 w-fit">
              <For each={colorTargetOptions()}>
                {(option) => (
                  <button
                    class={`${btnTabXs} ${colorTarget() === option.id ? tabActive : tabInactive}`}
                    onClick={() => selectColorTarget(option.id)}
                  >
                    {t(option.labelKey)}
                  </button>
                )}
              </For>
            </div>
            <ColorPickerPanel value={targetColor()} onChange={applyTargetColor} />
          </div>

          <Show when={appBackground().kind === 'gradient'}>
            <div class="flex items-center gap-3">
              <span class="text-[12px] text-white/50 w-[64px]">{t('app.background.angle')}</span>
              <input
                type="range"
                min="0"
                max="360"
                step="15"
                value={appBackground().gradientAngle}
                class="flex-1"
                style={backgroundSliderTrack}
                onInput={(e) =>
                  updateAppBackground({ gradientAngle: parseInt(e.currentTarget.value, 10) })
                }
              />
              <span class="font-mono text-[12px] text-white/70 w-[42px] text-right">
                {`${appBackground().gradientAngle}°`}
              </span>
            </div>
          </Show>

          <Show when={appBackground().kind === 'image'}>
            <div class="flex items-center gap-3">
              <Show when={appBackground().imagePath}>
                {(path) => (
                  <img
                    src={convertFileSrc(path())}
                    alt=""
                    class="w-24 h-12 object-cover rounded-lg border border-white/10"
                  />
                )}
              </Show>
              <button class={btnSecondarySm} onClick={chooseBackgroundImage}>
                {appBackground().imagePath
                  ? t('app.background.changeImage')
                  : t('app.background.chooseImage')}
              </button>
              <Show when={appBackground().imagePath}>
                <button class={btnDangerOutline} onClick={clearBackgroundImage}>
                  {t('app.background.clearImage')}
                </button>
              </Show>
            </div>
          </Show>

          <Show when={appBackground().kind !== 'default'}>
            <div class="flex items-center gap-3">
              <span class="text-[12px] text-white/50 w-[64px]">{t('app.background.opacity')}</span>
              <input
                type="range"
                min="0"
                max="100"
                value={appBackground().opacity}
                class="flex-1"
                style={backgroundSliderTrack}
                onInput={(e) =>
                  updateAppBackground({ opacity: parseInt(e.currentTarget.value, 10) })
                }
              />
              <span class="font-mono text-[12px] text-white/70 w-[42px] text-right">
                {`${appBackground().opacity}%`}
              </span>
            </div>
          </Show>

          <Show when={backgroundError()}>
            <div class="text-[11px] text-danger break-all">{backgroundError()}</div>
          </Show>

          <div>
            <button class={btnSecondarySm} onClick={resetBackground}>
              <Icon name="refresh" size={12} />
              <span>{t('app.background.reset')}</span>
            </button>
          </div>
        </div>
      </div>

      {/* App font */}
      <div
        class="bg-[rgb(255_255_255/0.04)] rounded-xl p-6 animate-row-in"
        style={{
          'backdrop-filter': 'blur(var(--acrylic-blur))',
          border: '1px solid var(--acrylic-border)',
          'border-radius': 'var(--acrylic-radius)',
          'animation-delay': '50ms',
        }}
      >
        <div class="flex justify-between items-center mb-5">
          <h3 class="m-0 text-base text-white">{t('app.font.title')}</h3>
          <button
            class={btnSecondarySm}
            disabled={fontsLoading()}
            onClick={() => void scanInstalledFonts()}
          >
            <Icon name="refresh" size={12} />
            <span>{t('app.font.scan')}</span>
          </button>
        </div>

        <div class="flex flex-col gap-3">
          <Dropdown
            value={appFont()}
            onChange={(value) => persistAppFont(value)}
            options={fontOptions()}
            placeholder={t('app.font.system')}
            variant="ghost"
            class="w-full max-w-[380px]"
          />
          <div class="w-full max-w-[380px]">
            <span class="block text-[12px] text-white/40 mb-1">{t('app.font.size.title')}</span>
            <StopSlider
              value={appFontSize()}
              stops={fontSizeOptions()}
              onChange={(value) => persistAppFontSize(value as AppFontSize)}
              ariaLabel={t('app.font.size.title')}
            />
          </div>
          <div class="rounded-lg border border-white/10 bg-white/[0.03] px-4 py-3">
            <span class="block text-[12px] text-white/40 mb-1">{t('app.font.previewLabel')}</span>
            <div
              class="text-white/80 truncate"
              style={{
                'font-family': appFontStack(appFont()) || undefined,
                'font-size': 'calc(15px * var(--app-font-scale))',
              }}
            >
              {t('app.font.previewText')}
            </div>
          </div>
          <Show when={fontScanError()}>
            <div class="text-[11px] text-danger break-all">{fontScanError()}</div>
          </Show>
          <Show when={!fontsLoading() && installedFonts().length === 0 && !fontScanError()}>
            <div class="text-[11px] text-white/35">{t('app.font.empty')}</div>
          </Show>
        </div>
      </div>

      {/* Shortcut settings panel */}
      <div
        class="bg-[rgb(255_255_255/0.04)] rounded-xl p-6 animate-row-in"
        style={{
          'backdrop-filter': 'blur(var(--acrylic-blur))',
          border: '1px solid var(--acrylic-border)',
          'border-radius': 'var(--acrylic-radius)',
          'animation-delay': '60ms',
        }}
      >
        <div class="flex justify-between items-center mb-5">
          <div>
            <h3 class="m-0 text-base text-white">{t('app.shortcuts.title')}</h3>
          </div>
          <button
            class={btnSecondarySm}
            onClick={() => setResetAllConfirm(true)}
            title={t('app.shortcuts.restoreDefaults')}
          >
            <Icon name="refresh" size={12} />
            <span>{t('app.shortcuts.restoreDefaults')}</span>
          </button>
        </div>

        {/* Reset-all confirmation dialog */}
        <Show when={resetAllConfirm()}>
          <div
            class="fixed inset-0 z-[10000] flex items-center justify-center bg-black/40 backdrop-blur-sm"
            onClick={() => setResetAllConfirm(false)}
          >
            <div
              class="rounded-xl px-6 py-5 max-w-[380px] bg-[rgba(var(--surface-alt-bg),0.97)] border border-white/[0.08] shadow-[0_12px_40px_rgba(0,0,0,0.5)]"
              style={{ animation: 'command-palette-slide-in 0.15s cubic-bezier(0.16, 1, 0.3, 1)' }}
              onClick={(e) => e.stopPropagation()}
            >
              <p class="text-[13px] text-white/75 leading-[1.5] m-0 mb-4">
                {t('app.shortcuts.resetMessage')}
              </p>
              <div class="flex justify-end gap-2">
                <button class={btnSecondary} onClick={() => setResetAllConfirm(false)}>
                  {t('common.cancel')}
                </button>
                <button
                  class={btnPrimaryOutline}
                  onClick={() => {
                    resetAllShortcutBindings();
                    setResetAllConfirm(false);
                  }}
                >
                  {t('common.confirm')}
                </button>
              </div>
            </div>
          </div>
        </Show>

        {/* Conflict confirmation dialog */}
        <Show when={conflictDialog()}>
          <div
            class="fixed inset-0 z-[10000] flex items-center justify-center bg-black/40 backdrop-blur-sm"
            onClick={handleConflictCancel}
          >
            <div
              class="rounded-xl px-6 py-5 max-w-[380px] bg-[rgba(var(--surface-alt-bg),0.97)] border border-white/[0.08] shadow-[0_12px_40px_rgba(0,0,0,0.5)]"
              style={{ animation: 'command-palette-slide-in 0.15s cubic-bezier(0.16, 1, 0.3, 1)' }}
              onClick={(e) => e.stopPropagation()}
            >
              <p class="text-[13px] text-white/75 leading-[1.5] m-0 mb-4">
                {t('app.shortcuts.conflictMessage', {
                  keys: conflictDialog()!.newKeys,
                  action: (() => {
                    const found = getRegisteredCommands().find(
                      (c) => c.id === conflictDialog()!.conflictActionId,
                    );
                    return found
                      ? getCommandDisplayLabel(found)
                      : conflictDialog()!.conflictActionId;
                  })(),
                })}
              </p>
              <div class="flex justify-end gap-2">
                <button class={btnSecondary} onClick={handleConflictCancel}>
                  {t('common.cancel')}
                </button>
                <button class={btnPrimaryOutline} onClick={handleConflictOverride}>
                  {t('app.shortcuts.replace')}
                </button>
              </div>
            </div>
          </div>
        </Show>

        {/* Shortcut list (grouped by category) */}
        <For each={groupedCommands()}>
          {(group) => (
            <div class="mb-4 last:mb-0">
              <div class="text-xs font-medium mb-2 uppercase tracking-wider text-white/30">
                {group.label}
              </div>
              <For each={group.items}>
                {(cmd) => {
                  const keys = () => getShortcutKeys(cmd.id);
                  const modified = () => isShortcutModified(cmd.id);
                  const isRecording = () => recordingActionId() === cmd.id;

                  return (
                    <div
                      class="flex items-center justify-between py-2.5 px-3 rounded-lg transition-colors duration-150 mb-[4px] last:mb-0"
                      style={{
                        background: 'rgba(var(--text-base-rgb), 0.02)',
                        border: '1px solid var(--border-dim)',
                      }}
                    >
                      <div class="flex-1 min-w-0 mr-4">
                        <span class="block text-[13px] text-white/85 font-medium">
                          {getCommandDisplayLabel(cmd)}
                        </span>
                        <span class="block text-[11px] text-white/40 mt-0.5 leading-relaxed">
                          {getCommandDisplayDescription(cmd)}
                        </span>
                      </div>
                      <div class="flex items-center gap-2 shrink-0">
                        {/* Shortcut badge (data-shortcut-badge is the FE-02 anchor used by the onDocClick selector) */}
                        <div
                          data-shortcut-badge=""
                          class={`inline-flex items-center px-2.5 py-1 rounded-md cursor-pointer transition-all duration-150 select-none bg-white/[0.04] border border-white/[0.06] min-w-[70px] justify-center hover:bg-white/[0.07] hover:border-white/[0.1]${isRecording() ? ' recording' : ''}${modified() ? ' modified' : ''}`}
                          onClick={() => {
                            if (isRecording()) {
                              stopRecording();
                            } else {
                              startRecording(cmd.id);
                            }
                          }}
                          title={
                            isRecording()
                              ? t('app.shortcuts.recordingCancel')
                              : t('app.shortcuts.change')
                          }
                        >
                          <Show
                            when={!isRecording()}
                            fallback={
                              // gate-allow: color hue preview in white (screenshot preview UI primitive)
                              <span class="text-[11px] font-medium" style={{ color: '#fff' }}>
                                {t('app.shortcuts.recording')}
                              </span>
                            }
                          >
                            <Show
                              when={keys()}
                              fallback={
                                <span class="text-[11px] text-white/25 italic">
                                  {t('app.shortcuts.unset')}
                                </span>
                              }
                            >
                              <span class="text-xs font-medium text-white/70 tracking-[0.02em]">
                                {formatShortcutForDisplay(keys())}
                              </span>
                            </Show>
                          </Show>
                        </div>

                        {/* Reset button (shown only when modified) */}
                        <Show when={modified()}>
                          <button
                            class={'w-[22px] h-[22px] text-white/30 hover:text-white/60 ' + btnIcon}
                            onClick={(e) => {
                              e.stopPropagation();
                              resetShortcutBinding(cmd.id);
                            }}
                            title={t('app.shortcuts.restoreOne')}
                          >
                            <Icon name="refresh" size={11} />
                          </button>
                        </Show>
                      </div>
                    </div>
                  );
                }}
              </For>
            </div>
          )}
        </For>

        {/* Empty state when there are no commands */}
        <Show when={allCommands().length === 0}>
          <div
            class="text-center py-8 text-[13px]"
            style={{ color: 'rgba(var(--text-base-rgb), 0.3)' }}
          >
            {t('app.shortcuts.empty')}
          </div>
        </Show>
      </div>

      {/* Alpha feedback section (environment snapshot + GitHub channels + diagnostics) */}
      <FeedbackSection />
    </div>
  );
};

export default AppSettings;
