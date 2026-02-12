/**
 * Settings.tsx - 应用设置中心页面组件
 * 
 * 【功能概述】
 * 本文件实现了一个三标签页的设置中心界面，基于 SolidJS 框架构建。
 * 采用经典的左右布局：左侧为导航菜单，右侧为对应设置内容。
 * 包含三个设置模块：
 *   1. 供应商设置 - 配置 AI 模型提供商（API 地址、密钥、模型列表等）
 *   2. 账号信息   - 用户账户相关设置
 *   3. 应用信息   - 应用程序通用配置
 * 
 * 【数据流流向】
 * 
 * 1. 路由/导航数据流:
 *    应用路由 → 渲染 Settings 组件 → 默认显示 'provider' 标签页
 * 
 * 2. 标签切换数据流（内部状态）:
 *    用户点击菜单项 → setActiveTab(tabId) → activeTab 信号更新 → 
 *    Switch/Match 条件渲染 → 对应设置组件挂载显示
 * 
 * 3. 各标签页独立数据流:
 *    - ProviderSettings: 可能涉及本地存储读写（API 配置持久化）
 *    - AccountSettings: 可能涉及用户信息加载/保存
 *    - AppSettings: 可能涉及应用配置读写
 *    （具体数据流见各子组件实现）
 * 
 * 【组件架构】
 * Settings (容器组件)
 * ├── 左侧 Sidebar (导航菜单)
 * └── 右侧 Content (动态渲染)
 *     ├── ProviderSettings (供应商配置)
 *     ├── AccountSettings (账号管理)
 *     └── AppSettings (应用配置)
 * 
 * 【依赖】
 * - SolidJS: 响应式 UI 框架（createSignal, Switch, Match, Component）
 * - 本地组件: ProviderSettings, AccountSettings, AppSettings
 * - 样式: Settings.css
 */

import { Component, createSignal, Switch, Match, JSX } from 'solid-js';
import ProviderSettings from '../components/ProviderSettings';
import AccountSettings from '../components/AccountSettings';
import AppSettings from '../components/AppSettings';
import './Settings.css';

/**
 * 图标组件集合
 * 使用内联 SVG 定义三个菜单项对应的图标，保持代码整洁且无需外部图标库依赖
 */
const Icons = {
    /**
     * 供应商图标 - 三层菱形堆叠，象征服务提供商/云服务商的分层架构
     * @returns JSX.Element SVG 图标元素
     */
    Provider: () => (
        <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            {/* 顶层菱形 */}
            <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" />
        </svg>
    ),
    
    /**
     * 账号图标 - 用户轮廓（头部圆形 + 肩部线条）
     * @returns JSX.Element SVG 图标元素
     */
    Account: () => (
        <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            {/* 肩部线条 */}
            <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
            {/* 头部圆形 */}
            <circle cx="12" cy="7" r="4" />
        </svg>
    ),
    
    /**
     * 应用图标 - 九宫格/网格布局，象征应用程序界面
     * @returns JSX.Element SVG 图标元素
     */
    App: () => (
        <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            {/* 外框圆角矩形 */}
            <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
            {/* 垂直分隔线 */}
            <line x1="9" y1="3" x2="9" y2="21" />
            {/* 水平分隔线 */}
            <line x1="3" y1="9" x2="21" y2="9" />
        </svg>
    )
};

/**
 * 设置页面主组件
 * 管理标签页状态并渲染对应的设置内容
 */
const Settings: Component = () => {
    /**
     * 当前激活的标签页 ID
     * 可选值: 'provider' | 'account' | 'app'
     * 默认显示供应商设置页
     */
    const [activeTab, setActiveTab] = createSignal('provider');

    /**
     * 侧边栏菜单配置数组
     * 定义三个设置分类的 ID、显示标签和对应图标
     * 结构: { id: 唯一标识, label: 显示文本, icon: JSX 图标元素 }
     */
    const menuItems = [
        { id: 'provider', label: '供应商设置', icon: <Icons.Provider /> },
        { id: 'account', label: '账号信息', icon: <Icons.Account /> },
        { id: 'app', label: '应用信息', icon: <Icons.App /> },
    ];

    return (
        <div class="settings-page">
            {/* 左侧侧边栏导航 */}
            <div class="settings-sidebar">
                {/* 侧边栏标题 */}
                <div class="sidebar-header">设置中心</div>
                
                {/* 导航菜单列表 */}
                <div class="sidebar-menu">
                    {menuItems.map(item => (
                        <div
                            // 动态类名：当前激活项添加 'active' 类用于高亮样式
                            class={`sidebar-item ${activeTab() === item.id ? 'active' : ''}`}
                            // 点击切换激活标签页
                            onClick={() => setActiveTab(item.id)}
                        >
                            {/* 菜单项图标 */}
                            <span class="sidebar-icon">{item.icon}</span>
                            {/* 菜单项文本标签 */}
                            <span class="sidebar-label">{item.label}</span>
                        </div>
                    ))}
                </div>
            </div>

            {/* 右侧主内容区域 */}
            <div class="settings-main-content">
                {/* 
                  Switch/Match: SolidJS 的条件渲染组件
                  根据 activeTab() 的值匹配对应的设置组件
                  相比多个 Show 组件或三元表达式，Switch 在互斥条件时性能更好且更清晰
                */}
                <Switch>
                    {/* 供应商设置标签页 */}
                    <Match when={activeTab() === 'provider'}>
                        <div class="tab-content-wrapper">
                            <ProviderSettings />
                        </div>
                    </Match>
                    
                    {/* 账号信息标签页 */}
                    <Match when={activeTab() === 'account'}>
                        <div class="tab-content-wrapper">
                            <AccountSettings />
                        </div>
                    </Match>
                    
                    {/* 应用信息标签页 */}
                    <Match when={activeTab() === 'app'}>
                        <div class="tab-content-wrapper">
                            <AppSettings />
                        </div>
                    </Match>
                </Switch>
            </div>
        </div>
    );
};

export default Settings;