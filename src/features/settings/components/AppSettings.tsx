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
import { open } from '@tauri-apps/plugin-shell';
import { invoke } from '@tauri-apps/api/core';
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
  type CommandAction,
} from '../../../core/shortcuts';
import { getVersion } from '@tauri-apps/api/app';
import Icon from '../../../shared/components/Icon';
import { locale, setLocale, t, type Locale } from '../../../core/i18n';

/**
 * 后端 check_app_update 返回的结构化结果（与 src-tauri/src/commands/update.rs 一一对应）
 * 用 `kind` 字段做 tag，前端按类别显示不同提示
 */
type CheckUpdateResult =
  | { kind: 'up_to_date'; current_version: string }
  | {
      kind: 'update_available';
      info: { version: string; current_version: string; notes?: string; pub_date?: string };
    }
  | { kind: 'service_not_ready'; current_version: string; endpoint: string; reason: string }
  | { kind: 'network'; current_version: string; endpoint: string; reason: string }
  | { kind: 'failed'; current_version: string; endpoint: string; reason: string };

/**
 * 应用设置页面组件
 * @returns {JSX.Element} 应用设置页面的 JSX 元素
 */
const AppSettings: Component = () => {
  const [h, setH] = createSignal(0); // 色相 (0-360 度)
  const [s, setS] = createSignal(0); // 饱和度 (0-100%)
  const [l, setL] = createSignal(0); // 亮度 (0-100%)
  const [autoStart, setAutoStart] = createSignal(false);
  const [knowledgeEnabled, setKnowledgeEnabled] = createSignal(false);
  const [maxToolRounds, setMaxToolRounds] = createSignal(25); // 单次 Agent 最大工具调用轮数（默认 25）
  const [maxConcurrentSubagents, setMaxConcurrentSubagents] = createSignal(5); // 并发子智能体上限（默认 5）
  const [version, setVersion] = createSignal(''); // 应用版本号
  const [checkUpdating, setCheckUpdating] = createSignal(false); // 手动检查更新中
  const [checkResult, setCheckResult] = createSignal<CheckUpdateResult | null>(null); // 最近一次手动检查结果

  // ---- 快捷键设置状态 ----
  const [recordingActionId, setRecordingActionId] = createSignal<string | null>(null); // 正在录制的命令 ID
  const [conflictDialog, setConflictDialog] = createSignal<{
    actionId: string;
    newKeys: string;
    conflictActionId: string;
  } | null>(null); // 冲突确认对话框状态
  const [resetAllConfirm, setResetAllConfirm] = createSignal(false); // 重置全部确认状态
  const [langOpen, setLangOpen] = createSignal(false); // 语言切换下拉开关
  let langRef: HTMLDivElement | undefined; // 语言切换下拉容器
  let recordingCleanup: (() => void) | null = null; // 录制模式清理函数

  onMount(() => {
    const onDocMouse = (e: MouseEvent) => {
      if (langOpen() && langRef && !langRef.contains(e.target as Node)) setLangOpen(false);
    };
    document.addEventListener('mousedown', onDocMouse);
    onCleanup(() => document.removeEventListener('mousedown', onDocMouse));
  });

  /**
   * 初始化 HSL 状态和获取应用版本
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
      console.error('获取版本失败', e);
    }

    // 加载跨会话记忆 + 系统自启配置
    try {
      const cfg: Record<string, unknown> = await invoke('load_app_config');
      if (typeof cfg?.knowledgeEnabled === 'boolean') {
        setKnowledgeEnabled(cfg.knowledgeEnabled);
      }
      if (typeof cfg?.autoStartEnabled === 'boolean') {
        setAutoStart(cfg.autoStartEnabled);
      }
      if (typeof cfg?.maxToolRounds === 'number' && cfg.maxToolRounds > 0) {
        setMaxToolRounds(cfg.maxToolRounds);
      }
      if (typeof cfg?.maxConcurrentSubagents === 'number' && cfg.maxConcurrentSubagents > 0) {
        setMaxConcurrentSubagents(cfg.maxConcurrentSubagents);
      }
    } catch (e) {
      console.warn('加载应用配置失败:', e);
    }
  });

  /**
   * 监听全局主题色变化，同步更新本地 HSL 状态
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
   * 处理色相/饱和度/亮度滑块输入
   * @param {('h' | 's' | 'l')} type - 滑块类型
   * @param {number} val - 滑块数值
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
   * 手动检查更新：调用后端 check_app_update，根据结构化结果更新 toast 状态和按钮提示
   */
  const handleManualCheck = async () => {
    setCheckUpdating(true);
    setCheckResult(null);
    try {
      const result = await invoke<CheckUpdateResult>('check_app_update');
      setCheckResult(result);

      switch (result.kind) {
        case 'update_available':
          setAppUpdateInfo({
            version: result.info.version,
            currentVersion: result.info.current_version,
            notes: result.info.notes,
            pubDate: result.info.pub_date,
          });
          setAppUpdateDismissed(false);
          setAppUpdateAvailable(true);
          break;
        case 'up_to_date':
        case 'service_not_ready':
        case 'network':
        case 'failed':
          // 这几种情况都不弹左下角 Toast
          break;
      }
    } catch (e) {
      console.error('手动检查更新失败:', e);
      setCheckResult({
        kind: 'failed',
        current_version: version(),
        endpoint: '',
        reason: typeof e === 'string' ? e : e instanceof Error ? e.message : '未知错误',
      });
    } finally {
      setCheckUpdating(false);
    }
  };

  /**
   * 把结构化的检查结果翻译为显示在按钮旁的提示文字
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
   * Hex 颜色转 RGB 对象
   * @param {string} hex - Hex 颜色字符串
   * @returns {{r: number, g: number, b: number}} RGB 分量对象
   */
  const hexToRgb = (hex: string) => {
    const r = parseInt(hex.slice(1, 3), 16) || 0;
    const g = parseInt(hex.slice(3, 5), 16) || 0;
    const b = parseInt(hex.slice(5, 7), 16) || 0;
    return { r, g, b };
  };

  /**
   * Hex 颜色转 HSL 对象
   * @param {string} hex - Hex 颜色字符串
   * @returns {{h: number, s: number, l: number}} HSL 分量对象（h:0-360, s/l:0-100）
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
   * 环形色盘交互处理：将鼠标/触摸位置映射为色相角度
   * @param {MouseEvent | TouchEvent} e - 鼠标或触摸事件
   * @param {DOMRect} rect - 色环元素的边界
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
   * 计算色环指示点位置（基于当前色相）
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
   * HSL 颜色转 Hex 字符串
   * @param {number} h - 色相 (0-360)
   * @param {number} s - 饱和度 (0-100)
   * @param {number} l - 亮度 (0-100)
   * @returns {string} Hex 颜色字符串
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

  const rgb = createMemo(() => hexToRgb(themeColor())); // 当前主题色的 RGB 值

  // ---- 快捷键录制逻辑 ----

  /** 开始录制快捷键 */
  const startRecording = (actionId: string) => {
    // 先清理之前的录制
    if (recordingCleanup) {
      recordingCleanup();
      recordingCleanup = null;
    }
    setRecordingActionId(actionId);

    const onKeyDown = (e: KeyboardEvent) => {
      const combo = normalizeKeyCombo(e);
      if (!combo) return;

      e.preventDefault();
      e.stopPropagation();

      // 检查冲突
      const conflictId = getActionByKeys(combo);
      if (conflictId && conflictId !== actionId) {
        // 有冲突，弹确认框
        setConflictDialog({
          actionId,
          newKeys: combo,
          conflictActionId: conflictId,
        });
      } else {
        // 无冲突，直接设置
        setShortcutBinding(actionId, combo);
      }

      // 停止录制
      stopRecording();
    };

    document.addEventListener('keydown', onKeyDown, { capture: true });
    recordingCleanup = () => {
      document.removeEventListener('keydown', onKeyDown, { capture: true });
    };
  };

  /** 停止录制 */
  const stopRecording = () => {
    setRecordingActionId(null);
    if (recordingCleanup) {
      recordingCleanup();
      recordingCleanup = null;
    }
  };

  /** 处理冲突确认：覆盖 */
  const handleConflictOverride = () => {
    const d = conflictDialog();
    if (d) {
      setShortcutBinding(d.actionId, d.newKeys);
    }
    setConflictDialog(null);
  };

  /** 处理冲突确认：取消 */
  const handleConflictCancel = () => {
    setConflictDialog(null);
  };

  // 点击页面其他地方取消录制
  const onDocClick = (e: MouseEvent) => {
    if (recordingActionId()) {
      // 检查点击是否在快捷键徽章上
      const target = e.target as HTMLElement;
      if (
        !target.closest(
          '.inline-flex items-center px-2.5 py-1 rounded-md cursor-pointer transition-all duration-150 select-none bg-white/[0.04] border border-white/[0.06] min-w-[70px] justify-center hover:bg-white/[0.07] hover:border-white/[0.1]',
        )
      ) {
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

  // 所有注册的命令（用于设置列表显示）
  const allCommands = createMemo(() => getRegisteredCommands());

  // 按类别分组
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
    { name: t('app.theme.softBlue'), color: '#7c9abf' },
    { name: t('app.theme.ashPink'), color: '#b8929e' },
    { name: t('app.theme.warmGray'), color: '#a8a098' },
    { name: t('app.theme.sageGreen'), color: '#9aab9a' },
    { name: t('app.theme.lavender'), color: '#a89cc8' },
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
            <span class="block text-[#eee] text-[14px]">{t('app.language.title')}</span>
            <p class="text-xs text-white/35 mt-1">
              {t('app.language.description')} {t('app.language.system')}
            </p>
          </div>
          <div class="relative min-w-[150px]" ref={langRef}>
            <button
              type="button"
              class="w-full flex items-center justify-between gap-2 px-3 py-1.5 rounded-lg text-xs text-left cursor-pointer outline-none border border-[var(--border-dim)] transition-all duration-150"
              onClick={() => setLangOpen(!langOpen())}
              aria-haspopup="listbox"
              aria-expanded={langOpen()}
            >
              <span class="truncate text-white/80">
                {locale() === 'zh-CN' ? t('app.language.zhCN') : t('app.language.enUS')}
              </span>
              <span class="text-white/30 text-[10px]">&#9662;</span>
            </button>
            <div
              class="absolute z-[101] left-0 right-0 mt-1 rounded-[10px] p-1 transition-all duration-150 ease-out origin-top"
              style={{
                background: 'rgba(var(--surface-bg), 0.92)',
                'backdrop-filter': 'blur(24px) saturate(150%)',
                border: '1px solid var(--border-dim)',
                'box-shadow': '0 12px 40px rgba(0, 0, 0, 0.45)',
              }}
              classList={{
                'invisible opacity-0 scale-95 translate-y-1 pointer-events-none': !langOpen(),
                'visible opacity-100 scale-100 translate-y-0': langOpen(),
              }}
              role="listbox"
            >
              <For each={langOptions()}>
                {(opt) => {
                  const sel = () => locale() === opt.value;
                  return (
                    <div
                      class="flex items-center gap-1.5 px-2.5 py-[7px] rounded-md text-white/[0.78] cursor-pointer transition-[background,color] duration-[120ms] select-none"
                      classList={{
                        'text-white bg-pri-10': sel(),
                        'text-white/55 hover:bg-white/[0.06] hover:text-white/80': !sel(),
                      }}
                      role="option"
                      aria-selected={sel()}
                      onClick={() => {
                        setLocale(opt.value);
                        setLangOpen(false);
                      }}
                    >
                      <span class="grow truncate">{opt.label}</span>
                      <Show when={sel()}>
                        <Icon name="check" size={12} class="text-pri" />
                      </Show>
                    </div>
                  );
                }}
              </For>
            </div>
          </div>
        </div>

        <div class="flex justify-between items-center py-3 border-b border-white/5">
          <div class="pr-6">
            <span class="block text-[#eee] text-[14px]">{t('app.darkMode.title')}</span>
            <p class="text-xs text-white/35 mt-1">{t('app.darkMode.description')}</p>
          </div>
          <div class="flex rounded-md border border-white/10 overflow-hidden">
            <button
              type="button"
              class="px-3 py-1 text-xs transition-all duration-200"
              classList={{
                'bg-pri-10 text-white': !isDarkMode(),
                'text-white/35 hover:text-white/60': isDarkMode(),
              }}
              onClick={() => setIsDarkMode(false)}
            >
              {t('app.darkMode.off')}
            </button>
            <button
              type="button"
              class="px-3 py-1 text-xs transition-all duration-200"
              classList={{
                'bg-pri-10 text-white': isDarkMode(),
                'text-white/35 hover:text-white/60': !isDarkMode(),
              }}
              onClick={() => setIsDarkMode(true)}
            >
              {t('app.darkMode.on')}
            </button>
          </div>
        </div>

        <div class="flex justify-between items-center py-3 border-b border-white/5">
          <div>
            <span class="block text-[#eee] text-[14px]">{t('app.autoStart.title')}</span>
            <p class="text-xs text-white/35 mt-1">{t('app.autoStart.description')}</p>
          </div>

          <label class="relative inline-block w-[40px] h-[20px] cursor-pointer">
            <input
              class="opacity-0 w-0 h-0 peer"
              type="checkbox"
              checked={autoStart()}
              onChange={async (e) => {
                const val = e.currentTarget.checked;
                setAutoStart(val);
                try {
                  await invoke('set_auto_start', { enabled: val });
                } catch (err) {
                  console.warn('设置自启失败:', err);
                  setAutoStart(!val); // 回滚
                }
              }}
            />
            <span class="absolute inset-0 bg-dark-300 border border-dark-100 rounded-full transition-all duration-300 peer-checked:bg-pri peer-checked:border-pri after:content-[''] after:absolute after:top-0.5 after:left-0.5 after:bg-white after:w-3.5 after:h-3.5 after:rounded-full after:transition-all peer-checked:after:translate-x-5" />
          </label>
        </div>

        <div class="flex justify-between items-center py-3 border-b border-white/5">
          <div>
            <span class="block text-[#eee] text-[14px]">{t('app.knowledge.title')}</span>
            <p class="text-xs text-white/35 mt-1">{t('app.knowledge.description')}</p>
          </div>

          <label class="relative inline-block w-[40px] h-[20px] cursor-pointer">
            <input
              class="opacity-0 w-0 h-0 peer"
              type="checkbox"
              checked={knowledgeEnabled()}
              onChange={async (e) => {
                const val = e.currentTarget.checked;
                setKnowledgeEnabled(val);
                try {
                  const cfg: any = await invoke('load_app_config').catch(() => null);
                  if (cfg) {
                    await invoke('save_app_config', { config: { ...cfg, knowledgeEnabled: val } });
                  }
                } catch (err) {
                  console.warn('保存 knowledge 配置失败:', err);
                }
              }}
            />
            <span class="absolute inset-0 bg-dark-300 border border-dark-100 rounded-full transition-all duration-300 peer-checked:bg-pri peer-checked:border-pri after:content-[''] after:absolute after:top-0.5 after:left-0.5 after:bg-white after:w-3.5 after:h-3.5 after:rounded-full after:transition-all peer-checked:after:translate-x-5" />
          </label>
        </div>

        <div class="flex justify-between items-center py-3 border-b border-white/5">
          <div>
            <span class="block text-[#eee] text-[14px]">{t('app.maxToolRounds.title')}</span>
            <p class="text-xs text-white/35 mt-1">{t('app.maxToolRounds.description')}</p>
          </div>

          <input
            type="number"
            min={1}
            max={200}
            step={1}
            value={maxToolRounds()}
            class="w-[84px] h-[30px] px-2 rounded-md text-sm text-right bg-white/[0.05] border border-white/[0.1] focus:outline-none focus:border-[rgba(var(--primary-rgb),0.5)]"
            onInput={async (e) => {
              const raw = Number(e.currentTarget.value);
              const clamped = Number.isFinite(raw)
                ? Math.min(200, Math.max(1, Math.round(raw)))
                : 25;
              setMaxToolRounds(clamped);
              try {
                const cfg: any = await invoke('load_app_config').catch(() => null);
                if (cfg) {
                  await invoke('save_app_config', { config: { ...cfg, maxToolRounds: clamped } });
                }
              } catch (err) {
                console.warn('保存最大工具轮数配置失败:', err);
              }
            }}
          />
        </div>

        <div class="flex justify-between items-center py-3 border-b border-white/5">
          <div>
            <span class="block text-[#eee] text-[14px]">
              {t('app.maxConcurrentSubagents.title')}
            </span>
            <p class="text-xs text-white/35 mt-1">{t('app.maxConcurrentSubagents.description')}</p>
          </div>

          <input
            type="number"
            min={1}
            max={32}
            step={1}
            value={maxConcurrentSubagents()}
            class="w-[84px] h-[30px] px-2 rounded-md text-sm text-right bg-white/[0.05] border border-white/[0.1] focus:outline-none focus:border-[rgba(var(--primary-rgb),0.5)]"
            onInput={async (e) => {
              const raw = Number(e.currentTarget.value);
              const clamped = Number.isFinite(raw) ? Math.min(32, Math.max(1, Math.round(raw))) : 5;
              setMaxConcurrentSubagents(clamped);
              try {
                const cfg: any = await invoke('load_app_config').catch(() => null);
                if (cfg) {
                  await invoke('save_app_config', {
                    config: { ...cfg, maxConcurrentSubagents: clamped },
                  });
                }
              } catch (err) {
                console.warn('保存并发子智能体上限配置失败:', err);
              }
            }}
          />
        </div>

        <div class="flex justify-between items-center py-3 border-b border-white/5">
          <div>
            <span class="block text-[#eee] text-[14px]">{t('app.openSource.title')}</span>
            <p class="text-xs text-white/35 mt-1">{t('app.openSource.description')}</p>
          </div>

          <div
            class="flex items-center gap-2 px-4 py-2 rounded-lg cursor-pointer transition-all duration-200"
            style={{
              background: 'rgba(var(--primary-rgb), 0.18)',
              color: '#fff',
              border: '1px solid rgba(var(--primary-rgb), 0.25)',
            }}
            onClick={() => open('https://github.com/Atom112/AIO')}
            title={t('app.openSource.visit')}
          >
            <Icon src="/icons/app-logo/github.svg" class="w-5 h-5" />
            <span>GitHub</span>
          </div>
        </div>

        <div class="flex justify-between items-center py-3">
          <div class="flex-1 min-w-0 pr-4">
            <span class="block text-[#eee] text-[14px]">{t('app.update.title')}</span>
            <p
              class="text-xs mt-1"
              style={{
                color: (() => {
                  const r = checkResult();
                  if (!r) return 'rgba(var(--text-base-rgb),0.35)';
                  if (r.kind === 'update_available') return 'var(--primary-color)';
                  if (r.kind === 'service_not_ready' || r.kind === 'failed' || r.kind === 'network')
                    return '#d99';
                  return '#7c9abf';
                })(),
              }}
            >
              {checkResultMessage()}
            </p>
          </div>

          <button
            class="flex items-center gap-2 px-4 py-2 rounded-lg cursor-pointer transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed shrink-0"
            style={{
              background: 'rgba(var(--primary-rgb), 0.18)',
              color: '#fff',
              border: '1px solid rgba(var(--primary-rgb), 0.25)',
            }}
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
                          ? '2px solid #fff'
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
                  background: `linear-gradient(to right, #000, hsl(${h()}, ${s()}%, 50%), #fff)`,
                }}
                onInput={(e) => handleSliderUpdate('l', parseInt(e.currentTarget.value))}
              />
            </div>
          </div>
        </div>
      </div>

      {/* 快捷键设置面板 */}
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
            class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs transition-all duration-200"
            style={{
              background: 'rgba(var(--text-base-rgb), 0.04)',
              color: 'rgba(var(--text-base-rgb), 0.40)',
              border: '1px solid var(--border-dim)',
            }}
            onClick={() => setResetAllConfirm(true)}
            title={t('app.shortcuts.restoreDefaults')}
          >
            <Icon name="refresh" size={12} />
            <span>{t('app.shortcuts.restoreDefaults')}</span>
          </button>
        </div>

        {/* 恢复全部确认对话框 */}
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
                <button
                  class="px-4 py-1.5 rounded-[7px] text-[13px] font-medium cursor-pointer transition-all duration-150 border border-transparent cancel"
                  onClick={() => setResetAllConfirm(false)}
                >
                  {t('common.cancel')}
                </button>
                <button
                  class="px-4 py-1.5 rounded-[7px] text-[13px] font-medium cursor-pointer transition-all duration-150 border border-transparent confirm"
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

        {/* 冲突确认对话框 */}
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
                <button
                  class="px-4 py-1.5 rounded-[7px] text-[13px] font-medium cursor-pointer transition-all duration-150 border border-transparent cancel"
                  onClick={handleConflictCancel}
                >
                  {t('common.cancel')}
                </button>
                <button
                  class="px-4 py-1.5 rounded-[7px] text-[13px] font-medium cursor-pointer transition-all duration-150 border border-transparent confirm"
                  onClick={handleConflictOverride}
                >
                  {t('app.shortcuts.replace')}
                </button>
              </div>
            </div>
          </div>
        </Show>

        {/* 快捷键列表（按类别分组） */}
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
                        {/* 快捷键徽章 */}
                        <div
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

                        {/* 重置按钮（仅修改后显示） */}
                        <Show when={modified()}>
                          <button
                            class="inline-flex items-center justify-center w-[22px] h-[22px] rounded-[5px] border-none bg-white/[0.04] text-white/30 cursor-pointer transition-all duration-150 p-0 hover:bg-white/[0.08] hover:text-white/60"
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

        {/* 无命令时的空状态 */}
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
