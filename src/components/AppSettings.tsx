/**
 * ============================================================================
 * 文件功能摘要
 * ============================================================================
 * 
 * 【核心功能】
 * Tauri 桌面应用的设置页面组件，提供应用状态管理、主题色彩自定义、
 * 系统自启配置和更新日志展示等功能。支持 HSL 色彩空间的实时调色，
 * 包含预设主题快速切换和色相/饱和度滑块精细调节。
 * 
 * 【数据流流向】
 * ┌─────────────────────────────────────────────────────────────────────────┐
 * │  外部数据流入                                                            │
 * │  ├── themeColor (Signal) ← 从 '../store/store' 导入，全局主题色状态      │
 * │  └── getVersion() ← Tauri API，获取应用版本号                            │
 * │                                                                          │
 * │  用户交互输出                                                            │
 * │  ├── setThemeColor() → 写入全局 Store，同步更新应用主题色                │
 * │  ├── setAutoStart() → 本地 Signal（TODO: 需接入系统自启API）             │
 * │  └── open() → Tauri shell 插件，调用系统浏览器打开 GitHub 链接           │
 * │                                                                          │
 * │  本地状态管理 (SolidJS Signals)                                          │
 * │  ├── h, s, l: 色相/饱和度/亮度，用于滑块实时控制                         │
 * │  ├── autoStart: 开机自启开关状态                                         │
 * │  └── version: 应用版本号                                                 │
 * │                                                                          │
 * │  本地文件                                                                │
 * │  └── 导入 AppSettings.css 样式文件                                       │
 * └─────────────────────────────────────────────────────────────────────────┘
 * 
 * 【组件层级】
 * AppSettings (本组件)
 * ├── 应用状态卡片 (版本号、自启开关、GitHub链接)
 * ├── 视觉主题卡片 (色彩预览、预设主题、HSL滑块)
 * └── 更新日志卡片 (静态文本列表)
 * 
 * 【关键技术点】
 * - 使用 untrack() 防止 createEffect 死循环
 * - HSL ↔ Hex 双向颜色空间转换
 * - SolidJS 响应式系统：createMemo 派生 RGB/HSL 值
 * ============================================================================
 */

// SolidJS 核心响应式 API
import {
    Component,      // 组件类型定义
    createEffect,   // 创建响应式副作用
    createMemo,     // 创建记忆化计算值
    createSignal,   // 创建响应式状态
    For,            // 列表渲染组件
    onMount,        // 组件挂载生命周期钩子
    untrack         // 在响应式上下文中读取非响应式值
} from 'solid-js';

// Tauri 插件：调用系统默认程序打开外部链接
import { open } from '@tauri-apps/plugin-shell';
// 本地样式文件
import './AppSettings.css';
// 全局状态管理：主题色状态及 Setter 函数
import { setThemeColor, themeColor } from '../store/store';
// Tauri API：获取应用版本信息
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
    // ==================== 本地状态定义 ====================

    /** 色相 (Hue): 0-360 度，控制颜色的基本色调 */
    const [h, setH] = createSignal(0);
    /** 饱和度 (Saturation): 0-100%，控制颜色鲜艳程度 */
    const [s, setS] = createSignal(0);
    /** 亮度 (Lightness): 0-100%，控制颜色明暗（当前固定，未提供滑块调节） */
    const [l, setL] = createSignal(0);
    /** 开机自启开关状态：true 表示随系统启动 */
    const [autoStart, setAutoStart] = createSignal(true);
    /** 应用版本号，默认 1.0.0，挂载后从 Tauri API 获取真实版本 */
    const [version, setVersion] = createSignal('1.0.0');

    // ==================== 生命周期钩子 ====================

    /**
     * 组件挂载时执行：初始化 HSL 状态和获取应用版本
     * 
     * 数据流：
     * 1. 从全局 Store 读取当前主题色（Hex格式）
     * 2. 转换为 HSL 格式并设置到本地 Signal（h, s, l）
     * 3. 异步调用 Tauri getVersion() 获取版本号
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

    // ==================== 响应式副作用 ====================

    /**
     * 主题色同步 Effect：监听全局 themeColor 变化，同步更新本地 HSL 状态
     * 
     * 【核心逻辑】
     * 当全局主题色被外部修改（如其他组件或预设主题点击）时，自动更新滑块位置。
     * 使用 untrack() 避免在拖动滑块时触发此 Effect（防止抽搐/死循环）。
     * 
     * 数据流：themeColor(Store) → hexToHsl() → setH/setS/setL → 滑块UI更新
     */
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

    // ==================== 事件处理函数 ====================

    /**
     * 滑块更新处理器：处理色相/饱和度滑块的输入事件
     * 
     * 【设计目的】
     * 直接操作本地 Signal 确保滑块响应流畅，避免 Hex↔HSL 反复转换导致的精度损失。
     * 更新完成后立即同步到全局 Store，触发应用主题色更新。
     * 
     * @param {('h' | 's')} type - 滑块类型：'h' 色相 或 's' 饱和度
     * @param {number} val - 滑块当前数值
     * 
     * 数据流：滑块输入 → setH/setS → 读取最新 h,s,l → hslToHex() → setThemeColor() → 全局更新
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

    // ==================== 颜色转换工具函数 ====================

    /**
     * Hex 颜色转 RGB 对象
     * 
     * @param {string} hex - Hex 颜色字符串（如 "#08ddf9"）
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
     * 算法说明：
     * 1. 将 RGB 归一化到 [0,1]
     * 2. 计算 max/min 确定亮度 l
     * 3. 根据 max-min 差值计算饱和度 s
     * 4. 根据 max 所属通道计算色相 h（0-360度）
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
     * 算法说明：使用 HSL 转 RGB 的通用公式，通过辅助函数 f(n) 计算各通道值
     * 
     * @param {number} h - 色相 (0-360)
     * @param {number} s - 饱和度 (0-100)
     * @param {number} l - 亮度 (0-100)
     * @returns {string} Hex 颜色字符串（如 "#08ddf9"）
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

    // ==================== 派生状态（Memo）====================

    /** 
     * 记忆化 RGB 值：基于当前主题色自动计算 RGB 分量
     * 用于 RGB 输入框展示
     */
    const rgb = createMemo(() => hexToRgb(themeColor()));

    // ==================== 静态数据 ====================

    /** 预设主题列表：包含名称和 Hex 颜色值 */
    const presetThemes = [
        { name: '极光青', color: '#90D0E0' },
        { name: '樱花粉', color: '#F5BDE6' },
        { name: '翡翠绿', color: '#A6DA95' },
        { name: '紫罗兰', color: '#B0B0F0' },
        { name: '夕阳橙', color: '#F5A97F' },
    ];

    // ==================== 渲染逻辑 ====================

    return (
        // 主容器：应用设置页面根元素
        <div class="app-settings-container">

            {/* ==================== 卡片 1: 应用状态 ==================== */}
            <div class="settings-card">
                <div class="card-header">
                    <h3>📱 应用状态</h3>
                    <div class="version-wrapper">
                        <span class="version-label">版本号:</span>
                        {/* 动态渲染版本号，从 Tauri API 获取 */}
                        <div class="version-badge">v{version()}-Beta</div>
                    </div>
                </div>

                {/* 设置项：系统自启开关 */}
                <div class="setting-item-row">
                    <div class="item-label">
                        <span>系统自启</span>
                        <p class="item-desc">随系统启动自动运行应用</p>
                    </div>
                    {/* 自定义 Switch 开关组件 */}
                    <label class="switch">
                        <input
                            type="checkbox"
                            checked={autoStart()}
                            onChange={(e) => setAutoStart(e.currentTarget.checked)}
                        />
                        <span class="slider"></span>
                    </label>
                </div>

                {/* 设置项：GitHub 开源主页链接 */}
                <div class="setting-item-row">
                    <div class="item-label">
                        <span>开源主页</span>
                        <p class="item-desc">访问 GitHub 仓库获取最新动态</p>
                    </div>
                    {/* 点击调用 Tauri open() 在系统浏览器中打开链接 */}
                    <div
                        class="github-link"
                        onClick={() => open('https://github.com/Atom112/AIO')}
                        title="访问 GitHub"
                    >
                        {/* GitHub Logo SVG */}
                        <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor">
                            <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.43.372.823 1.102.823 2.222 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
                        </svg>
                        <span>GitHub</span>
                    </div>
                </div>
            </div>

            {/* ==================== 卡片 2: 视觉主题 ==================== */}
            <div class="settings-card cool-theme-card">
                <div class="card-header">
                    <h3>🎨 视觉主题</h3>
                </div>

                <div class="picker-controls" style={{
                    "--h": h(),
                    "--s": `${s()}%`,
                    "--l": `${l()}%`
                }}>
                    {/* 三栏布局 */}
                    <div class="cool-picker-layout">

                        {/* 左侧：RGB 值 */}
                        <div class="rgb-sidebar">
                            <div class="rgb-field"><span>R</span><div class="rgb-value">{rgb().r}</div></div>
                            <div class="rgb-field"><span>G</span><div class="rgb-value">{rgb().g}</div></div>
                            <div class="rgb-field"><span>B</span><div class="rgb-value">{rgb().b}</div></div>
                        </div>

                        {/* 中间：色相环 + 预览 */}
                        <div class="hue-ring-container">
                            {/* 色相环主体 */}
                            <div
                                class="hue-ring"
                                // 使用 Pointer Events 统一处理鼠标和触摸
                                onPointerDown={(e) => {
                                    const target = e.currentTarget;
                                    const rect = target.getBoundingClientRect();

                                    // 激活捕获：即便鼠标移出元素，后续 move/up 事件仍发给此元素
                                    target.setPointerCapture(e.pointerId);

                                    handleRingInteraction(e as any, rect);

                                    const onPointerMove = (ev: PointerEvent) => {
                                        handleRingInteraction(ev as any, rect);
                                    };

                                    const onPointerUp = (ev: PointerEvent) => {
                                        // 释放捕获并移除监听
                                        target.releasePointerCapture(ev.pointerId);
                                        target.removeEventListener('pointermove', onPointerMove);
                                        target.removeEventListener('pointerup', onPointerUp);
                                    };

                                    // 直接在元素上监听，不再需要 window
                                    target.addEventListener('pointermove', onPointerMove);
                                    target.addEventListener('pointerup', onPointerUp);
                                }}
                            // 移除之前的 onTouchStart，Pointer Events 已覆盖
                            />

                            {/* 环中心的颜色预览 */}
                            <div
                                class="center-preview"
                                style={{ background: themeColor() }}
                            >
                                <span class="hex-text">{themeColor().toUpperCase()}</span>
                            </div>

                            {/* 指示点 */}
                            <div class="hue-pointer" style={pointerStyle()} />
                        </div>

                        {/* 右侧：预设颜色 */}
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

                    {/* 下方：饱和度控制 */}
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
                                    /* 这里的渐变从黑到当前色再到白 */
                                    background: `linear-gradient(to right, #000, hsl(${h()}, ${s()}%, 50%), #fff)`
                                }}
                                onInput={(e) => handleSliderUpdate('l', parseInt(e.currentTarget.value))}
                            />
                        </div>
                    </div>
                </div>
            </div>

            {/* ==================== 卡片 3: 更新日志 ==================== */}
            <div class="settings-card">
                <div class="card-header">
                    <h3>📝 更新日志</h3>
                </div>
                {/* 静态更新内容：当前版本的功能更新说明 */}
                <div class="update-log-content">
                    <p>• 优化自定义调色算法，彻底解决滑块抽搐问题</p>
                    <p>• 新增饱和度调节轨道，支持更精细的色彩自定义</p>
                    <p>• 修复 CSS 兼容性编译器警告，移除冗余样式代码</p>
                    <p>• 增强视觉反馈，预设颜色选中态实时高亮</p>
                </div>
            </div>
        </div>
    );
};

// 默认导出组件
export default AppSettings;