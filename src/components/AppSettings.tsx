import { Component, createEffect, createMemo, createSignal, For, onMount, Show, untrack } from 'solid-js';
import { open } from '@tauri-apps/plugin-shell';
import { invoke } from '@tauri-apps/api/core';
import {
    setThemeColor,
    themeColor,
    setAppUpdateAvailable,
    setAppUpdateInfo,
    setAppUpdateDismissed,
    modelsCatalog,
    modelsCatalogSource,
    modelsCatalogPath,
    modelsCatalogVersion,
    modelsCatalogGeneratedAt,
    setModelsCatalog,
    setModelsCatalogSource,
    setModelsCatalogPath,
    setModelsCatalogVersion,
    setModelsCatalogGeneratedAt,
} from '../store/store';
import { updateModelsCatalog, refreshModelsCatalog, getCatalogMeta, formatRelativeTime } from '../utils/models';
import { getVersion } from '@tauri-apps/api/app';
import Icon from './Icon';

/**
 * 后端 check_app_update 返回的结构化结果（与 src-tauri/src/commands/update.rs 一一对应）
 * 用 `kind` 字段做 tag，前端按类别显示不同提示
 */
type CheckUpdateResult =
    | { kind: 'up_to_date'; current_version: string }
    | { kind: 'update_available'; info: { version: string; current_version: string; notes?: string; pub_date?: string } }
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
    const [autoStart, setAutoStart] = createSignal(true); // 系统自启开关状态
    const [version, setVersion] = createSignal(''); // 应用版本号
    const [checkUpdating, setCheckUpdating] = createSignal(false); // 手动检查更新中
    const [checkResult, setCheckResult] = createSignal<CheckUpdateResult | null>(null); // 最近一次手动检查结果
    const [endpointDisplay, setEndpointDisplay] = createSignal<string>(''); // 调试展示用：当前 endpoint

    const [catalogUpdating, setCatalogUpdating] = createSignal(false); // 模型数据更新中
    const [catalogUpdateResult, setCatalogUpdateResult] = createSignal<{ ok: boolean; msg: string; ts: number } | null>(null);
    const [catalogUrlDisplay, setCatalogUrlDisplay] = createSignal<string>(''); // catalog 下载端点

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
            console.error("获取版本失败", e);
        }

        // 加载当前 endpoint 用于调试展示
        try {
            const eps = await invoke<string[]>('get_updater_endpoint');
            setEndpointDisplay(eps.join(', '));
        } catch (e) {
            console.warn('获取 endpoint 失败:', e);
        }

        // 加载 catalog URL 用于调试展示
        try {
            const u = await invoke<string>('get_catalog_url');
            setCatalogUrlDisplay(u);
        } catch (e) {
            console.warn('获取 catalog URL 失败:', e);
        }
    });

    /**
     * 手动检查并更新模型元数据
     */
    const handleUpdateCatalog = async () => {
        setCatalogUpdating(true);
        setCatalogUpdateResult(null);
        try {
            const result = await updateModelsCatalog();
            if (result.success) {
                setCatalogUpdateResult({
                    ok: true,
                    msg: `已更新 · v${result.version} · ${result.modelCount} 个模型 · ${result.elapsedMs}ms · ${(result.bytes / 1024).toFixed(0)} KB`,
                    ts: Date.now(),
                });
                // 刷新 store 中的元数据
                const cat = modelsCatalog();
                if (cat) {
                    const meta = getCatalogMeta();
                    setModelsCatalog(cat);
                    setModelsCatalogSource(meta.source);
                    setModelsCatalogPath(meta.path);
                    setModelsCatalogVersion(meta.version);
                    setModelsCatalogGeneratedAt(meta.generatedAt);
                }
            } else {
                setCatalogUpdateResult({
                    ok: false,
                    msg: result.error ?? '更新失败',
                    ts: Date.now(),
                });
            }
        } catch (e) {
            setCatalogUpdateResult({
                ok: false,
                msg: typeof e === 'string' ? e : (e instanceof Error ? e.message : '未知错误'),
                ts: Date.now(),
            });
        } finally {
            setCatalogUpdating(false);
        }
    };

    /** catalog 来源的人类可读描述 */
    const catalogSourceLabel = (): string => {
        switch (modelsCatalogSource()) {
            case 'appdata':       return 'AppData 缓存';
            case 'bundled':       return '应用内置';
            case 'dev_fallback':  return '开发模式 (node_modules)';
            case 'empty':         return '无';
        }
    };

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
                endpoint: endpointDisplay(),
                reason: typeof e === 'string' ? e : (e instanceof Error ? e.message : '未知错误'),
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
        if (!r) return '检查 AIO 是否有新版本发布';
        switch (r.kind) {
            case 'up_to_date':         return '当前已是最新版本';
            case 'update_available':   return '已发现新版本，左下角查看详情';
            case 'service_not_ready':  return '当前 release 尚未配置自动更新服务';
            case 'network':            return '网络错误，无法连接更新服务器';
            case 'failed':             return '检查失败，请稍后重试';
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
        r /= 255; g /= 255; b /= 255;

        const max = Math.max(r, g, b), min = Math.min(r, g, b);
        let h = 0, s = 0, l = (max + min) / 2;

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
            l: Math.round(l * 100)
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
            transform: `translate(${x}px, ${y}px) translate(-50%, -50%)`
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
            return Math.round(255 * color).toString(16).padStart(2, '0');
        };

        return `#${f(0)}${f(8)}${f(4)}`;
    };

    const rgb = createMemo(() => hexToRgb(themeColor())); // 当前主题色的 RGB 值

    const presetThemes = [
        { name: '柔雾蓝', color: '#7c9abf' },
        { name: '灰粉', color: '#b8929e' },
        { name: '暖灰', color: '#a8a098' },
        { name: '鼠尾绿', color: '#9aab9a' },
        { name: '薰衣草', color: '#a89cc8' },
    ];

    return (
        <div class="flex flex-col gap-[15px] box-border">
            <div class="rounded-xl p-6" style="background: rgba(18, 22, 35, 0.6); backdrop-filter: blur(30px); border: 1px solid rgba(255, 255, 255, 0.06); box-shadow: 0 8px 32px rgba(0, 0, 0, 0.2);">
                <div class="flex justify-between items-center mb-5">
                    <h3 class="m-0 text-base text-white">应用状态</h3>
                    <div class="flex items-center gap-2">
                        <span class="text-xs text-[#888] font-medium">版本号:</span>
                        <div class="text-base font-bold px-2.5 py-0.5 rounded-full font-mono whitespace-nowrap"
                            style="background: rgba(124,154,191,0.15); color: rgba(255,255,255,0.8); font-family: 'JetBrains Mono', monospace;">
                            v{version()}
                        </div>
                    </div>
                </div>

                <div class="flex justify-between items-center py-3 border-b border-white/5">
                    <div>
                        <span class="block text-[#eee] text-[14px]">系统自启</span>
                        <p class="text-xs text-[#777] mt-1">随系统启动自动运行应用</p>
                    </div>

                    <label class="relative inline-block w-[40px] h-[20px] cursor-pointer">
                        <input
                            class="opacity-0 w-0 h-0 peer"
                            type="checkbox"
                            checked={autoStart()}
                            onChange={(e) => setAutoStart(e.currentTarget.checked)}
                        />
                        <span class="absolute inset-0 bg-dark-300 border border-dark-100 rounded-full transition-all duration-300 peer-checked:bg-pri peer-checked:border-pri after:content-[''] after:absolute after:top-0.5 after:left-0.5 after:bg-white after:w-3.5 after:h-3.5 after:rounded-full after:transition-all peer-checked:after:translate-x-5"></span>
                    </label>
                </div>

                <div class="flex justify-between items-center py-3 border-b border-white/5">
                    <div>
                        <span class="block text-[#eee] text-[14px]">开源主页</span>
                        <p class="text-xs text-[#777] mt-1">访问 GitHub 仓库获取最新动态</p>
                    </div>

                    <div
                        class="flex items-center gap-2 px-4 py-2 rounded-lg cursor-pointer transition-all duration-200"
                        style="background: rgba(124,154,191,0.08); color: rgba(255,255,255,0.5); border: 1px solid rgba(124,154,191,0.08);"
                        onClick={() => open('https://github.com/Atom112/AIO')}
                        title="访问 GitHub"
                    >
                        <Icon src="/icons/app-logo/github.svg" class="w-5 h-5" />
                        <span>GitHub</span>
                    </div>
                </div>

                <div class="flex justify-between items-center py-3">
                    <div class="flex-1 min-w-0 pr-4">
                        <span class="block text-[#eee] text-[14px]">版本更新</span>
                        <p
                            class="text-xs mt-1"
                            style={{
                                color: (() => {
                                    const r = checkResult();
                                    if (!r) return '#777';
                                    if (r.kind === 'update_available') return 'var(--primary-color)';
                                    if (r.kind === 'service_not_ready' || r.kind === 'failed' || r.kind === 'network') return '#d99';
                                    return '#7c9abf';
                                })(),
                            }}
                        >
                            {checkResultMessage()}
                        </p>
                        <Show when={checkResult() && checkResult()!.kind !== 'update_available' && checkResult()!.kind !== 'up_to_date'}>
                            <p class="text-[11px] text-[#666] mt-1 break-all leading-relaxed">
                                {checkResult()!.kind === 'service_not_ready' && '💡 '}
                                {(checkResult() as { reason?: string }).reason}
                            </p>
                        </Show>
                    </div>

                    <button
                        class="flex items-center gap-2 px-4 py-2 rounded-lg cursor-pointer transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed shrink-0"
                        style={{
                            background: 'rgba(var(--primary-rgb), 0.18)',
                            color: 'var(--primary-color)',
                            border: '1px solid rgba(var(--primary-rgb), 0.25)',
                        }}
                        disabled={checkUpdating()}
                        onClick={handleManualCheck}
                        title="手动检查更新"
                    >
                        <Icon src="/icons/app-logo/switch-arrows.svg" class="w-4 h-4" />
                        <span class="text-sm font-medium">
                            {checkUpdating() ? '检查中…' : '检查更新'}
                        </span>
                    </button>
                </div>

                <Show when={endpointDisplay()}>
                    <div
                        class="mt-2 px-3 py-2 rounded-lg text-[11px] font-mono leading-relaxed"
                        style={{
                            background: 'rgba(255, 255, 255, 0.03)',
                            color: 'rgba(255, 255, 255, 0.45)',
                            border: '1px solid rgba(255, 255, 255, 0.05)',
                            'word-break': 'break-all',
                        }}
                        title="当前 tauri.conf.json 中配置的更新清单地址"
                    >
                        <span style="color: rgba(255,255,255,0.3)">endpoint:</span> {endpointDisplay()}
                    </div>
                </Show>
            </div>

            <div class="rounded-xl p-6" style="background: rgba(18, 22, 35, 0.6); backdrop-filter: blur(30px); border: 1px solid rgba(255, 255, 255, 0.06); box-shadow: 0 8px 32px rgba(0, 0, 0, 0.2);">
                <div class="flex justify-between items-center mb-5">
                    <h3 class="m-0 text-base text-white">模型元数据库</h3>
                    <div class="flex items-center gap-2">
                        <span class="text-xs text-[#888]">来源:</span>
                        <div class="text-xs px-2.5 py-0.5 rounded-full font-medium"
                            style={{
                                background: 'rgba(124,154,191,0.15)',
                                color: 'rgba(255,255,255,0.7)',
                                'font-family': "'JetBrains Mono', monospace",
                            }}>
                            {catalogSourceLabel()}
                        </div>
                    </div>
                </div>

                <div class="grid grid-cols-3 gap-3 mb-4">
                    <div class="bg-white/5 border border-white/10 rounded-lg p-3">
                        <div class="text-[10px] text-[#888] uppercase tracking-wider mb-1">厂商</div>
                        <div class="text-xl font-bold text-white font-mono">
                            {modelsCatalog()?.providerCount ?? 0}
                        </div>
                    </div>
                    <div class="bg-white/5 border border-white/10 rounded-lg p-3">
                        <div class="text-[10px] text-[#888] uppercase tracking-wider mb-1">模型</div>
                        <div class="text-xl font-bold text-white font-mono">
                            {modelsCatalog()?.modelCount ?? 0}
                        </div>
                    </div>
                    <div class="bg-white/5 border border-white/10 rounded-lg p-3">
                        <div class="text-[10px] text-[#888] uppercase tracking-wider mb-1">版本</div>
                        <div class="text-base font-bold text-white font-mono truncate" title={modelsCatalogVersion() ?? ''}>
                            {modelsCatalogVersion() ? `v${modelsCatalogVersion()}` : '—'}
                        </div>
                    </div>
                </div>

                <div class="flex justify-between items-center py-2">
                    <div class="flex-1 min-w-0 pr-4">
                        <span class="block text-[#eee] text-[14px]">检查数据更新</span>
                        <p
                            class="text-xs mt-1"
                            style={{
                                color: (() => {
                                    const r = catalogUpdateResult();
                                    if (!r) return '#777';
                                    return r.ok ? 'var(--primary-color)' : '#d99';
                                })(),
                            }}
                        >
                            {(() => {
                                const r = catalogUpdateResult();
                                if (r) return r.msg;
                                const gen = modelsCatalogGeneratedAt();
                                if (gen) return `数据生成于 ${formatRelativeTime(gen)}`;
                                return '点击右侧按钮从 aio-models-data 拉取最新数据';
                            })()}
                        </p>
                    </div>

                    <button
                        class="flex items-center gap-2 px-4 py-2 rounded-lg cursor-pointer transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed shrink-0"
                        style={{
                            background: 'rgba(var(--primary-rgb), 0.18)',
                            color: 'var(--primary-color)',
                            border: '1px solid rgba(var(--primary-rgb), 0.25)',
                        }}
                        disabled={catalogUpdating()}
                        onClick={handleUpdateCatalog}
                        title="从 aio-models-data 仓库拉取最新模型元数据"
                    >
                        <Icon src="/icons/app-logo/switch-arrows.svg" class="w-4 h-4" />
                        <span class="text-sm font-medium">
                            {catalogUpdating() ? '下载中…' : '检查数据更新'}
                        </span>
                    </button>
                </div>

                <Show when={catalogUrlDisplay()}>
                    <div
                        class="mt-3 px-3 py-2 rounded-lg text-[11px] font-mono leading-relaxed"
                        style={{
                            background: 'rgba(255, 255, 255, 0.03)',
                            color: 'rgba(255, 255, 255, 0.45)',
                            border: '1px solid rgba(255, 255, 255, 0.05)',
                            'word-break': 'break-all',
                        }}
                        title="catalog 数据下载端点 (aio-models-data 仓库 raw)"
                    >
                        <span style="color: rgba(255,255,255,0.3)">source URL:</span> {catalogUrlDisplay()}
                    </div>
                </Show>

                <Show when={modelsCatalogPath()}>
                    <div
                        class="mt-2 px-3 py-2 rounded-lg text-[11px] font-mono leading-relaxed"
                        style={{
                            background: 'rgba(255, 255, 255, 0.03)',
                            color: 'rgba(255, 255, 255, 0.35)',
                            border: '1px solid rgba(255, 255, 255, 0.05)',
                            'word-break': 'break-all',
                        }}
                        title="当前生效的 catalog 本地路径"
                    >
                        <span style="color: rgba(255,255,255,0.25)">local path:</span> {modelsCatalogPath()}
                    </div>
                </Show>
            </div>

            <div class="bg-[rgb(255_255_255/0.04)] glow-border rounded-xl p-6">
                <div class="flex justify-between items-center mb-5">
                    <h3 class='m-0 text-base text-white'>视觉主题</h3>
                </div>

                <div class="flex flex-col gap-[10px]" style={{
                    "--h": h(),
                    "--s": `${s()}%`,
                    "--l": `${l()}%`
                }}>
                    <div class="grid grid-cols-[100px_240px_100px] gap-8 items-center justify-center py-5">
                        <div class="flex flex-col gap-3">
                            <div class="bg-white/5 border border-white/10 p-[10px] rounded-[10px] text-center">
                                <span class="block text-[16px] text-gray-500 mb-2 font-bold">R</span>
                                <div class="font-mono text-[16px] text-white font-bold">{rgb().r}</div>
                            </div>
                            <div class="bg-white/5 border border-white/10 p-[10px] rounded-[10px] text-center">
                                <span class="block text-[16px] text-gray-500 mb-2 font-bold">G</span>
                                <div class="font-mono text-[16px] text-white font-bold">{rgb().g}</div>
                            </div>
                            <div class="bg-white/5 border border-white/10 p-[10px] rounded-[10px] text-center">
                                <span class="block text-[16px] text-gray-500 mb-2 font-bold">B</span>
                                <div class="font-mono text-[16px] text-white font-bold">{rgb().b}</div>
                            </div>
                        </div>

                        <div class="relative w-[220px] h-[220px] flex items-center justify-center">
                            <div
                                class="w-full h-full rounded-full cursor-pointer transition-all duration-300"
                                style="background: conic-gradient(hsl(0deg var(--s) var(--l)), 
                                        hsl(60deg var(--s) var(--l)), 
                                        hsl(120deg var(--s) var(--l)), 
                                        hsl(180deg var(--s) var(--l)), 
                                        hsl(240deg var(--s) var(--l)), 
                                        hsl(300deg var(--s) var(--l)), 
                                        hsl(360deg var(--s) var(--l))); 
                                        mask: radial-gradient(transparent 59.5%, black 60.5%);"
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
                                <span class="text-[14px] font-extrabold text-white">{themeColor().toUpperCase()}</span>
                            </div>

                            <div class="absolute top-1/2 left-1/2 w-[18px] h-[18px] border-[3px] border-white rounded-full shadow-[0_0_5px_#fff,inset_0_0_10px_#fff] pointer-events-none z-[3]"
                                style={pointerStyle()} />
                        </div>

                        <div class="flex flex-col gap-3">
                            <For each={presetThemes}>
                                {(theme) => (
                                    <div
                                        class="w-[40px] h-[40px] rounded-full cursor-pointer transition-transform duration-200 border-2 border-transparent hover:scale-110"
                                        onClick={() => setThemeColor(theme.color)}
                                        style={{
                                            background: theme.color,
                                            border: themeColor().toLowerCase() === theme.color.toLowerCase()
                                                ? '2px solid #fff'
                                                : '2px solid transparent'
                                        }}
                                    />
                                )}
                            </For>
                        </div>
                    </div>

                    <div class="mt-[25px] px-[20px]">
                        <div class="mb-2">
                            <label class="block text-[12px] text-[#666] mb-[10px] text-center">饱和度 (Saturation)</label>
                            <input
                                type="range"
                                min="0" max="100"
                                value={s()}
                                class="custom-slider sat-slider"
                                style={{
                                    background: `linear-gradient(to right, hsl(${h()}, 0%, ${l()}%), hsl(${h()}, 100%, ${l()}%))`
                                }}
                                onInput={(e) => handleSliderUpdate('s', parseInt(e.currentTarget.value))}
                            />
                        </div>

                        <div class="mb-2">
                            <label class="block text-[12px] text-[#666] mb-[10px] text-center">亮度 (Lightness)</label>
                            <input
                                type="range"
                                min="0" max="100"
                                value={l()}
                                class="custom-slider light-slider"
                                style={{
                                    background: `linear-gradient(to right, #000, hsl(${h()}, ${s()}%, 50%), #fff)`
                                }}
                                onInput={(e) => handleSliderUpdate('l', parseInt(e.currentTarget.value))}
                            />
                        </div>
                    </div>
                </div>
            </div>
        </div>
    );
};

export default AppSettings;