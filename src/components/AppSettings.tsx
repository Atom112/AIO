import { Component, createEffect, createMemo, createSignal, For, onMount, untrack } from 'solid-js';
import { open } from '@tauri-apps/plugin-shell';
import { setThemeColor, themeColor } from '../store/store';
import { getVersion } from '@tauri-apps/api/app';
import Icon from './Icon';

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
        { name: '极光青', color: '#90D0E0' },
        { name: '樱花粉', color: '#F5BDE6' },
        { name: '翡翠绿', color: '#A6DA95' },
        { name: '紫罗兰', color: '#B0B0F0' },
        { name: '夕阳橙', color: '#F5A97F' },
    ];

    return (
        <div class="flex flex-col gap-[15px] box-border">
            <div class="bg-[rgb(255_255_255/0.04)] glow-border rounded-xl p-6 transition-colors duration-300">
                <div class="flex justify-between items-center mb-5">
                    <h3 class="m-0 text-base text-white">应用状态</h3>
                    <div class="flex items-center gap-2">
                        <span class="text-xs text-[#888] font-medium">版本号:</span>
                        <div class="bg-pri text-black text-base font-bold px-2.5 py-0.5 rounded-full font-mono whitespace-nowrap"
                            style="font-family: 'JetBrains Mono', monospace;">
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
                        class="flex items-center gap-2 bg-pri-5 text-[#ccc] px-4 py-2 rounded-lg cursor-pointer border border-pri transition-all duration-200 hover:bg-pri-50 hover:text-black"
                        onClick={() => open('https://github.com/Atom112/AIO')}
                        title="访问 GitHub"
                    >
                        <Icon src="/icons/app-logo/github.svg" class="w-5 h-5" />
                        <span>GitHub</span>
                    </div>
                </div>
            </div>

            <div class="bg-[rgb(255_255_255/0.04)] glow-border rounded-xl p-6 transition-colors duration-300">
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