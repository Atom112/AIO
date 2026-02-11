import { Component, createEffect, createMemo, createSignal, For, onMount } from 'solid-js';
import { open } from '@tauri-apps/plugin-shell';
import './AppSettings.css';
import { setThemeColor, themeColor } from '../store/store';
import { getVersion } from '@tauri-apps/api/app';
import { untrack } from 'solid-js';

const AppSettings: Component = () => {
    const [h, setH] = createSignal(0);
    const [s, setS] = createSignal(0);
    const [l, setL] = createSignal(0);
    const [autoStart, setAutoStart] = createSignal(true);
    const [version, setVersion] = createSignal('1.0.0');

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
    // --- 颜色转换核心逻辑（解决抽搐与同步问题） ---


    createEffect(() => {
        const currentHex = themeColor(); // 只要 themeColor 变了，就运行此 Effect

        // 使用 untrack 包裹，防止拖动滑块时触发此 Effect 自身的死循环
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

    // 3. 修改更新函数：直接操作本地 Signal
    const handleSliderUpdate = (type: 'h' | 's', val: number) => {
        // 先获取当前值，确保计算 hex 时用的是最新的
        let nextH = h();
        let nextS = s();

        if (type === 'h') {
            setH(val);
            nextH = val;
        } else {
            setS(val);
            nextS = val;
        }

        // 将最新状态同步到全局 Store
        const nextHex = hslToHex(nextH, nextS, l());
        setThemeColor(nextHex);
    };

    // Hex -> RGB (用于显示数据)
    const hexToRgb = (hex: string) => {
        const r = parseInt(hex.slice(1, 3), 16) || 0;
        const g = parseInt(hex.slice(3, 5), 16) || 0;
        const b = parseInt(hex.slice(5, 7), 16) || 0;
        return { r, g, b };
    };

    // Hex -> HSL (用于滑动条初始值)
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
        return { h: Math.round(h * 360), s: Math.round(s * 100), l: Math.round(l * 100) };
    };

    // HSL -> Hex (用于写入状态)
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

    const rgb = createMemo(() => hexToRgb(themeColor()));
    const hsl = createMemo(() => hexToHsl(themeColor()));

    const handleColorUpdate = (type: 'h' | 's', val: number) => {
        const { h, s, l } = hsl();
        const nextHex = type === 'h' ? hslToHex(val, s, l) : hslToHex(h, val, l);
        setThemeColor(nextHex);
    };

    const presetThemes = [
        { name: '极光青', color: '#08ddf9' },
        { name: '樱花粉', color: '#ff85c0' },
        { name: '深海蓝', color: '#1890ff' },
        { name: '翡翠绿', color: '#52c41a' },
    ];

    return (
        <div class="app-settings-container">
            {/* 卡片 1: 应用状态 */}
            <div class="settings-card">
                <div class="card-header">
                    <h3>📱 应用状态</h3>
                    <div class="version-wrapper">
                        <span class="version-label">版本号:</span>
                        <div class="version-badge">v{version()}-Beta</div>
                    </div>
                </div>

                <div class="setting-item-row">
                    <div class="item-label">
                        <span>系统自启</span>
                        <p class="item-desc">随系统启动自动运行应用</p>
                    </div>
                    <label class="switch">
                        <input type="checkbox" checked={autoStart()} onChange={(e) => setAutoStart(e.currentTarget.checked)} />
                        <span class="slider"></span>
                    </label>
                </div>

                <div class="setting-item-row">
                    <div class="item-label">
                        <span>开源主页</span>
                        <p class="item-desc">访问 GitHub 仓库获取最新动态</p>
                    </div>
                    <div class="github-link" onClick={() => open('https://github.com/Atom112/AIO')} title="访问 GitHub">
                        <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor">
                            <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.43.372.823 1.102.823 2.222 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
                        </svg>
                        <span>GitHub</span>
                    </div>
                </div>
            </div>

            {/* 卡片 2: 视觉主题 */}
            <div class="settings-card cool-theme-card">
                <div class="card-header">
                    <h3>🎨 视觉主题</h3>
                </div>

                <div class="cool-picker-layout">
                    {/* 左侧预览区 */}
                    <div class="picker-main">
                        <div class="main-preview" style={{ background: themeColor(), "box-shadow": `0 0 30px ${themeColor()}66` }}>
                            <span class="hex-display">{themeColor().toUpperCase()}</span>
                        </div>
                        <div class="rgb-inputs">
                            <div class="rgb-field"><span>R</span>{rgb().r}</div>
                            <div class="rgb-field"><span>G</span>{rgb().g}</div>
                            <div class="rgb-field"><span>B</span>{rgb().b}</div>
                        </div>
                    </div>

                    {/* 右侧控制区 */}
                    <div class="picker-controls">
                        <label>预设方案</label>
                        <div class="theme-strip">
                            <For each={presetThemes}>
                                {(theme) => (
                                    <div class="strip-item" onClick={() => setThemeColor(theme.color)}
                                        style={{ background: theme.color, border: themeColor().toLowerCase() === theme.color.toLowerCase() ? '2px solid #fff' : 'none' }}
                                    />
                                )}
                            </For>
                        </div>

                        <label>色相 (Hue)</label>
                        <div class="slider-container">
                            {/* 绑定本地状态 h() 而非 hex 计算出的状态 */}
                            <input type="range" min="0" max="360" value={h()} class="hue-slider"
                                onInput={(e) => handleSliderUpdate('h', parseInt(e.currentTarget.value))}
                            />
                        </div>

                        <label>饱和度 (Saturation)</label>
                        <div class="slider-container">
                            {/* 绑定本地状态 s() */}
                            <input type="range" min="0" max="100" value={s()} class="sat-slider"
                                style={{ "--h": h() }}
                                onInput={(e) => handleSliderUpdate('s', parseInt(e.currentTarget.value))}
                            />
                        </div>
                    </div>
                </div>
            </div>

            {/* 卡片 3: 更新日志 */}
            <div class="settings-card">
                <div class="card-header">
                    <h3>📝 更新日志</h3>
                </div>
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

export default AppSettings;