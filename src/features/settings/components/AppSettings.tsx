import {
  Component,
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
  untrack,
} from 'solid-js';
import { openSafeUrl } from '../../../core/utils/safeUrl';
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
import Icon from '../../../shared/components/Icon';
import NumberInput from '../../../shared/components/NumberInput';
import Switch from '../../../shared/components/Switch';
import Dropdown from '../../../shared/components/Dropdown';
import {
  btnIcon,
  btnPrimary,
  btnSecondary,
  btnSecondarySm,
  btnTabXs,
  tabActive,
  tabInactive,
} from '../../../shared/components/buttonStyles';
import { locale, setLocale, t, type Locale } from '../../../core/i18n';

/**
 * Structured result returned by the backend check_app_update (mirrors src-tauri/src/commands/update.rs)
 * The `kind` field acts as the tag; the frontend shows a different hint per kind
 */

/**
 * App settings page component
 * @returns {JSX.Element} The app settings page JSX element
 */
const AppSettings: Component = () => {
  const [h, setH] = createSignal(0); // Hue (0-360 degrees)
  const [s, setS] = createSignal(0); // Saturation (0-100%)
  const [l, setL] = createSignal(0); // Lightness (0-100%)
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
   * Initialize HSL state and fetch the app version
   */
  onMount(async () => {
    const initialHsl = hexToHsl(themeColor());
    setH(initialHsl.h);
    setS(initialHsl.s);
    setL(initialHsl.l);

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
   * Track global theme color changes and sync local HSL state
   */
  createEffect(() => {
    const currentHex = themeColor();

    const shouldUpdate = untrack(() => {
      const mappedHex = hslToHex(h(), s(), l());
      return currentHex.toLowerCase() !== mappedHex.toLowerCase();
    });

    if (shouldUpdate) {
      const currentHsl = hexToHsl(currentHex);
      setH(currentHsl.h);
      setS(currentHsl.s);
      setL(currentHsl.l);
    }
  });

  /**
   * Handle hue/saturation/lightness slider input
   * @param {('h' | 's' | 'l')} type - Slider type
   * @param {number} val - Slider value
   */
  const handleSliderUpdate = (type: 'h' | 's' | 'l', val: number) => {
    let nextH = h();
    let nextS = s();
    let nextL = l();

    if (type === 'h') {
      setH(val);
      nextH = val;
    } else if (type === 's') {
      setS(val);
      nextS = val;
    } else if (type === 'l') {
      setL(val);
      nextL = val;
    }

    const nextHex = hslToHex(nextH, nextS, nextL);
    setThemeColor(nextHex);
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
    if (!r) return t('app.update.description');
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

  /**
   * Convert a Hex color to an RGB object
   * @param {string} hex - Hex color string
   * @returns {{r: number, g: number, b: number}} RGB component object
   */
  const hexToRgb = (hex: string) => {
    const r = parseInt(hex.slice(1, 3), 16) || 0;
    const g = parseInt(hex.slice(3, 5), 16) || 0;
    const b = parseInt(hex.slice(5, 7), 16) || 0;
    return { r, g, b };
  };

  /**
   * Convert a Hex color to an HSL object
   * @param {string} hex - Hex color string
   * @returns {{h: number, s: number, l: number}} HSL component object (h: 0-360, s/l: 0-100)
   */
  const hexToHsl = (hex: string) => {
    let { r, g, b } = hexToRgb(hex);
    r /= 255;
    g /= 255;
    b /= 255;

    const max = Math.max(r, g, b),
      min = Math.min(r, g, b);
    let h = 0,
      s = 0;
    const l = (max + min) / 2;

    if (max !== min) {
      const d = max - min;
      s = l > 0.5 ? d / (2 - max - min) : d / (max + min);

      if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
      else if (max === g) h = (b - r) / d + 2;
      else h = (r - g) / d + 4;

      h /= 6;
    }

    return {
      h: Math.round(h * 360),
      s: Math.round(s * 100),
      l: Math.round(l * 100),
    };
  };

  /**
   * Ring color-wheel interaction: map the mouse/touch position to a hue angle
   * @param {MouseEvent | TouchEvent} e - Mouse or touch event
   * @param {DOMRect} rect - Bounds of the ring element
   */
  const handleRingInteraction = (e: MouseEvent | TouchEvent, rect: DOMRect) => {
    const centerX = rect.left + rect.width / 2;
    const centerY = rect.top + rect.height / 2;
    const clientX = 'touches' in e ? e.touches[0].clientX : (e as MouseEvent).clientX;
    const clientY = 'touches' in e ? e.touches[0].clientY : (e as MouseEvent).clientY;
    const angle = Math.atan2(clientY - centerY, clientX - centerX);
    let degree = (angle * 180) / Math.PI + 90;
    if (degree < 0) degree += 360;
    handleSliderUpdate('h', Math.round(degree));
  };

  /**
   * Compute the ring indicator position (from the current hue)
   */
  const pointerStyle = createMemo(() => {
    const rad = ((h() - 90) * Math.PI) / 180;
    const radius = 102;
    const x = Math.cos(rad) * radius;
    const y = Math.sin(rad) * radius;

    return {
      transform: `translate(${x}px, ${y}px) translate(-50%, -50%)`,
    };
  });

  /**
   * Convert an HSL color to a Hex string
   * @param {number} h - Hue (0-360)
   * @param {number} s - Saturation (0-100)
   * @param {number} l - Lightness (0-100)
   * @returns {string} Hex color string
   */
  const hslToHex = (h: number, s: number, l: number) => {
    l /= 100;
    const a = (s * Math.min(l, 1 - l)) / 100;

    const f = (n: number) => {
      const k = (n + h / 30) % 12;
      const color = l - a * Math.max(Math.min(k - 3, 9 - k, 1), -1);
      return Math.round(255 * color)
        .toString(16)
        .padStart(2, '0');
    };

    return `#${f(0)}${f(8)}${f(4)}`;
  };

  const rgb = createMemo(() => hexToRgb(themeColor())); // RGB value of the current theme color

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

  const presetThemes = [
    { name: t('app.theme.softBlue'), color: 'var(--color-pri)' },
    { name: t('app.theme.ashPink'), color: '#b8929e' }, // gate-allow: color #b8929e
    { name: t('app.theme.warmGray'), color: '#a8a098' }, // gate-allow: color #a8a098
    { name: t('app.theme.sageGreen'), color: '#9aab9a' }, // gate-allow: color #9aab9a
    { name: t('app.theme.lavender'), color: '#a89cc8' }, // gate-allow: color #a89cc8
  ];

  const langOptions = createMemo(() => [
    { value: 'zh-CN' as Locale, label: t('app.language.zhCN') },
    { value: 'en-US' as Locale, label: t('app.language.enUS') },
  ]);

  return (
    <div class="flex flex-col gap-[15px] box-border">
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
            <p class="text-xs text-white/35 mt-1">
              {t('app.language.description')} {t('app.language.system')}
            </p>
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
            <p class="text-xs text-white/35 mt-1">{t('app.darkMode.description')}</p>
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
            <p class="text-xs text-white/35 mt-1">{t('app.autoStart.description')}</p>
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
            <p class="text-xs text-white/35 mt-1">{t('app.openSource.description')}</p>
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
            <p
              class="text-xs mt-1"
              style={{
                color: (() => {
                  const r = checkResult();
                  if (!r) return 'rgba(var(--text-base-rgb),0.35)';
                  if (r.kind === 'update_available') return 'var(--primary-color)';
                  if (r.kind === 'service_not_ready' || r.kind === 'failed' || r.kind === 'network')
                    return '#d99'; // gate-allow: color return #d99
                  return 'var(--color-pri)';
                })(),
              }}
            >
              {checkResultMessage()}
            </p>
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

      <div
        class="bg-[rgb(255_255_255/0.04)] rounded-xl p-6 animate-row-in"
        style={{
          'backdrop-filter': 'blur(var(--acrylic-blur))',
          border: '1px solid var(--acrylic-border)',
          'border-radius': 'var(--acrylic-radius)',
          'animation-delay': '30ms',
        }}
      >
        <div class="flex justify-between items-center mb-5">
          <h3 class="m-0 text-base text-white">{t('app.theme.title')}</h3>
        </div>

        <div
          class="flex flex-col gap-[10px]"
          style={{
            '--h': h(),
            '--s': `${s()}%`,
            '--l': `${l()}%`,
          }}
        >
          <div class="grid grid-cols-[100px_240px_100px] gap-8 items-center justify-center py-5">
            <div class="flex flex-col gap-3">
              <div class="bg-white/5 border border-white/10 p-[10px] rounded-[10px] text-center">
                <span class="block text-[16px] text-white/35 mb-2 font-bold">R</span>
                <div class="font-mono text-[16px] text-white font-bold">{rgb().r}</div>
              </div>
              <div class="bg-white/5 border border-white/10 p-[10px] rounded-[10px] text-center">
                <span class="block text-[16px] text-white/35 mb-2 font-bold">G</span>
                <div class="font-mono text-[16px] text-white font-bold">{rgb().g}</div>
              </div>
              <div class="bg-white/5 border border-white/10 p-[10px] rounded-[10px] text-center">
                <span class="block text-[16px] text-white/35 mb-2 font-bold">B</span>
                <div class="font-mono text-[16px] text-white font-bold">{rgb().b}</div>
              </div>
            </div>

            <div class="relative w-[220px] h-[220px] flex items-center justify-center">
              <div
                class="w-full h-full rounded-full cursor-pointer transition-all duration-300"
                style={{
                  background:
                    'conic-gradient(hsl(0deg var(--s) var(--l)), \n                                        hsl(60deg var(--s) var(--l)), \n                                        hsl(120deg var(--s) var(--l)), \n                                        hsl(180deg var(--s) var(--l)), \n                                        hsl(240deg var(--s) var(--l)), \n                                        hsl(300deg var(--s) var(--l)), \n                                        hsl(360deg var(--s) var(--l)))',
                  mask: 'radial-gradient(transparent 59.5%, black 60.5%)',
                }}
                onPointerDown={(e) => {
                  const target = e.currentTarget;
                  const rect = target.getBoundingClientRect();
                  target.setPointerCapture(e.pointerId);
                  handleRingInteraction(e as any, rect);
                  const onPointerMove = (ev: PointerEvent) => {
                    handleRingInteraction(ev as any, rect);
                  };
                  const onPointerUp = (ev: PointerEvent) => {
                    target.releasePointerCapture(ev.pointerId);
                    target.removeEventListener('pointermove', onPointerMove);
                    target.removeEventListener('pointerup', onPointerUp);
                  };
                  target.addEventListener('pointermove', onPointerMove);
                  target.addEventListener('pointerup', onPointerUp);
                }}
              />

              <div
                class="absolute w-[110px] h-[110px] rounded-full flex flex-col items-center justify-center shadow-[0_0_20px_var(--primary-color)] border-2 border-white/20 z-20"
                style={{ background: themeColor() }}
              >
                <span class="text-[14px] font-extrabold text-white">
                  {themeColor().toUpperCase()}
                </span>
              </div>

              {/* gate-allow: color hue knob glow (hue picker UI primitive, white base) */}
              <div
                class="absolute top-1/2 left-1/2 w-[18px] h-[18px] border-[3px] border-white rounded-full shadow-[0_0_5px_#fff,inset_0_0_10px_#fff] pointer-events-none z-[3]"
                style={pointerStyle()}
              />
            </div>

            <div class="flex flex-col gap-3">
              <For each={presetThemes}>
                {(theme) => (
                  <div
                    class="w-[40px] h-[40px] rounded-full cursor-pointer transition-transform duration-200 border-2 border-transparent hover:scale-110"
                    onClick={() => setThemeColor(theme.color)}
                    style={{
                      background: theme.color,
                      border:
                        themeColor().toLowerCase() === theme.color.toLowerCase()
                          ? '2px solid #fff' // gate-allow: color 2px solid #fff
                          : '2px solid transparent',
                    }}
                  />
                )}
              </For>
            </div>
          </div>

          <div class="mt-[25px] px-[20px]">
            <div class="mb-2">
              <label class="block text-[12px] text-white/40 mb-[10px] text-center">
                {t('app.theme.saturation')}
              </label>
              <input
                type="range"
                min="0"
                max="100"
                value={s()}
                class="custom-slider sat-slider"
                style={{
                  background: `linear-gradient(to right, hsl(${h()}, 0%, ${l()}%), hsl(${h()}, 100%, ${l()}%))`,
                }}
                onInput={(e) => handleSliderUpdate('s', parseInt(e.currentTarget.value))}
              />
            </div>

            <div class="mb-2">
              <label class="block text-[12px] text-white/40 mb-[10px] text-center">
                {t('app.theme.lightness')}
              </label>
              <input
                type="range"
                min="0"
                max="100"
                value={l()}
                class="custom-slider light-slider"
                style={{
                  background: `linear-gradient(to right, #000, hsl(${h()}, ${s()}%, 50%), #fff)`, // gate-allow: color hue bar black/white anchors
                }}
                onInput={(e) => handleSliderUpdate('l', parseInt(e.currentTarget.value))}
              />
            </div>
          </div>
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
            <p class="text-xs text-white/35 mt-1">{t('app.shortcuts.description')}</p>
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
                  class={btnPrimary}
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
                <button class={btnPrimary} onClick={handleConflictOverride}>
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
    </div>
  );
};

export default AppSettings;
