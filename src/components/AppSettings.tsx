import { Component, createEffect, createMemo, createSignal, For, onMount, untrack } from 'solid-js';
import { open } from '@tauri-apps/plugin-shell';
import './AppSettings.css';
import { setThemeColor, themeColor } from '../store/store';
import { getVersion } from '@tauri-apps/api/app';

/**
 * 应用设置页面组件
 * 
 * @component
 * @description 提供应用配置界面，包括主题色彩自定义（HSL调色）、系统自启设置、
 *              版本信息展示和 GitHub 仓库链接。使用 Tauri API 与宿主系统交互。
 * 
 * @returns {JSX.Element} 应用设置页面的 JSX 元素
 */
const AppSettings: Component = () => {

    /** 色相 (Hue): 0-360 度，控制颜色的基本色调 */
    const [h, setH] = createSignal(0);
    /** 饱和度 (Saturation): 0-100%，控制颜色鲜艳程度 */
    const [s, setS] = createSignal(0);
    /** 亮度 (Lightness): 0-100%，控制颜色明暗（当前固定，未提供滑块调节） */
    const [l, setL] = createSignal(0);
    /** 开机自启开关状态：true 表示随系统启动 */
    const [autoStart, setAutoStart] = createSignal(true);
    /** 应用版本号，默认 1.0.0，挂载后从 Tauri API 获取真实版本 */
    const [version, setVersion] = createSignal('');

    /**
     * 组件挂载时执行：初始化 HSL 状态和获取应用版本
     */
    onMount(async () => {
        // 将全局主题色（Hex）转换为 HSL，用于初始化滑块位置
        const initialHsl = hexToHsl(themeColor());
        setH(initialHsl.h);
        setS(initialHsl.s);
        setL(initialHsl.l);

        // 获取应用版本号，失败时保持默认值并打印错误
        try {
            const v = await getVersion();
            setVersion(v);
        } catch (e) {
            console.error("获取版本失败", e);
        }
    });

    createEffect(() => {
        const currentHex = themeColor(); // 订阅全局主题色变化

        // 使用 untrack 读取本地 h,s,l，防止创建循环依赖
        // 比较当前 Hex 与本地 HSL 转换后的 Hex，判断是否需要更新
        const shouldUpdate = untrack(() => {
            const mappedHex = hslToHex(h(), s(), l());
            return currentHex.toLowerCase() !== mappedHex.toLowerCase();
        });

        // 仅当外部主题色与本地状态不一致时才更新（避免覆盖用户正在拖动的滑块）
        if (shouldUpdate) {
            const currentHsl = hexToHsl(currentHex);
            setH(currentHsl.h);
            setS(currentHsl.s);
            setL(currentHsl.l);
        }
    });

    /**
     * 滑块更新处理器：处理色相/饱和度滑块的输入事件
     * 
     * @param {('h' | 's')} type - 滑块类型：'h' 色相 或 's' 饱和度
     * @param {number} val - 滑块当前数值
     * 
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
     * 
     * @param {string} hex Hex 颜色字符串
     * @returns {{r: number, g: number, b: number}} RGB 分量对象，解析失败返回 0
     */
    const hexToRgb = (hex: string) => {
        const r = parseInt(hex.slice(1, 3), 16) || 0;
        const g = parseInt(hex.slice(3, 5), 16) || 0;
        const b = parseInt(hex.slice(5, 7), 16) || 0;
        return { r, g, b };
    };

    /**
     * Hex 颜色转 HSL 对象
     * 
     * @param {string} hex - Hex 颜色字符串
     * @returns {{h: number, s: number, l: number}} HSL 分量对象（h:0-360, s/l:0-100）
     */
    const hexToHsl = (hex: string) => {
        let { r, g, b } = hexToRgb(hex);
        // 归一化到 [0,1]
        r /= 255; g /= 255; b /= 255;

        const max = Math.max(r, g, b), min = Math.min(r, g, b);
        let h = 0, s = 0, l = (max + min) / 2;

        // 非灰度色才计算色相和饱和度
        if (max !== min) {
            const d = max - min;
            s = l > 0.5 ? d / (2 - max - min) : d / (max + min);

            // 计算色相（0-6 范围，后续转角度）
            if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
            else if (max === g) h = (b - r) / d + 2;
            else h = (r - g) / d + 4;

            h /= 6;
        }

        // 转换为标准 HSL 单位
        return {
            h: Math.round(h * 360),
            s: Math.round(s * 100),
            l: Math.round(l * 100)
        };
    };

    /**
     * 环形色盘交互处理器：将鼠标或触摸在色环上的位置映射为色相（Hue）角度并更新主题色
     *
     * @param {MouseEvent | TouchEvent} e - 鼠标或触摸事件
     * @param {DOMRect} rect - 色环元素的边界（getBoundingClientRect()）
     * @returns {void}
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

    // 计算指示点的位置
    const pointerStyle = createMemo(() => {
        // 将色相值转换为弧度，减去 90 度是因为 conic-gradient 起点在 12 点方向
        const rad = ((h() - 90) * Math.PI) / 180;
        const radius = 102;
        const x = Math.cos(rad) * radius;
        const y = Math.sin(rad) * radius;

        return {
            // 第一个 translate 处理环形位移，第二个 translate(-50%, -50%) 确保圆点中心对齐坐标
            transform: `translate(${x}px, ${y}px) translate(-50%, -50%)`
        };
    });

    /**
     * HSL 颜色转 Hex 字符串
     *  
     * @param {number} h - 色相 (0-360)
     * @param {number} s - 饱和度 (0-100)
     * @param {number} l - 亮度 (0-100)
     * @returns {string} Hex 颜色字符串
     */
    const hslToHex = (h: number, s: number, l: number) => {
        l /= 100; // 亮度归一化
        // 计算色度相关参数
        const a = (s * Math.min(l, 1 - l)) / 100;

        // 辅助函数：根据角度偏移计算颜色通道
        const f = (n: number) => {
            const k = (n + h / 30) % 12; // 每 30 度一个分段
            const color = l - a * Math.max(Math.min(k - 3, 9 - k, 1), -1);
            return Math.round(255 * color).toString(16).padStart(2, '0');
        };

        return `#${f(0)}${f(8)}${f(4)}`;
    };

    /** 
     * 记忆化 RGB 值：基于当前主题色自动计算 RGB 分量
     * 用于 RGB 输入框展示
     */
    const rgb = createMemo(() => hexToRgb(themeColor()));

    /** 预设主题列表：包含名称和 Hex 颜色值 */
    const presetThemes = [
        { name: '极光青', color: '#90D0E0' },
        { name: '樱花粉', color: '#F5BDE6' },
        { name: '翡翠绿', color: '#A6DA95' },
        { name: '紫罗兰', color: '#B0B0F0' },
        { name: '夕阳橙', color: '#F5A97F' },
    ];

    return (
        <div class="app-settings-container">
            <div class="settings-card">
                <div class="card-header">
                    <h3>应用状态</h3>
                    <div class="version-wrapper">
                        <span class="version-label">版本号:</span>
                        <div class="version-badge">v{version()}</div>
                    </div>
                </div>

                <div class="setting-item-row">
                    <div class="item-label">
                        <span>系统自启</span>
                        <p class="item-desc">随系统启动自动运行应用</p>
                    </div>

                    <label class="switch">
                        <input
                            type="checkbox"
                            checked={autoStart()}
                            onChange={(e) => setAutoStart(e.currentTarget.checked)}
                        />
                        <span class="slider"></span>
                    </label>
                </div>

                <div class="setting-item-row">
                    <div class="item-label">
                        <span>开源主页</span>
                        <p class="item-desc">访问 GitHub 仓库获取最新动态</p>
                    </div>

                    <div
                        class="github-link"
                        onClick={() => open('https://github.com/Atom112/AIO')}
                        title="访问 GitHub"
                    >
                        <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor">
                            <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.43.372.823 1.102.823 2.222 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
                        </svg>
                        <span>GitHub</span>
                    </div>
                </div>
            </div>

            <div class="settings-card cool-theme-card">
                <div class="card-header">
                    <h3>视觉主题</h3>
                </div>

                <div class="picker-controls" style={{
                    "--h": h(),
                    "--s": `${s()}%`,
                    "--l": `${l()}%`
                }}>
                    <div class="cool-picker-layout">
                        <div class="rgb-sidebar">
                            <div class="rgb-field"><span>R</span><div class="rgb-value">{rgb().r}</div></div>
                            <div class="rgb-field"><span>G</span><div class="rgb-value">{rgb().g}</div></div>
                            <div class="rgb-field"><span>B</span><div class="rgb-value">{rgb().b}</div></div>
                        </div>

                        <div class="hue-ring-container">
                            <div
                                class="hue-ring"
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
                                class="center-preview"
                                style={{ background: themeColor() }}
                            >
                                <span class="hex-text">{themeColor().toUpperCase()}</span>
                            </div>

                            <div class="hue-pointer" style={pointerStyle()} />
                        </div>

                        <div class="preset-sidebar">
                            <For each={presetThemes}>
                                {(theme) => (
                                    <div
                                        class="strip-item"
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

                    <div class="bottom-controls">
                        <div class="control-group">
                            <label>饱和度 (Saturation)</label>
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

                        <div class="control-group">
                            <label>亮度 (Lightness)</label>
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