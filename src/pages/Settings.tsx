import { Component, createSignal, Switch, Match, JSX } from 'solid-js';
import ProviderSettings from '../components/ProviderSettings';
import AccountSettings from '../components/AccountSettings';
import AppSettings from '../components/AppSettings';
import './Settings.css';

// 定义图标组件以保持代码整洁
const Icons = {
    Provider: () => (
        <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" />
        </svg>
    ),
    Account: () => (
        <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
            <circle cx="12" cy="7" r="4" />
        </svg>
    ),
    App: () => (
        <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
            <line x1="9" y1="3" x2="9" y2="21" />
            <line x1="3" y1="9" x2="21" y2="9" />
        </svg>
    )
};

const Settings: Component = () => {
    const [activeTab, setActiveTab] = createSignal('provider');

    const menuItems = [
        { id: 'provider', label: '供应商设置', icon: <Icons.Provider /> },
        { id: 'account', label: '账号信息', icon: <Icons.Account /> },
        { id: 'app', label: '应用信息', icon: <Icons.App /> },
    ];

    return (
        <div class="settings-page">
            {/* 左侧侧边栏目录 */}
            <div class="settings-sidebar">
                <div class="sidebar-header">设置中心</div>
                <div class="sidebar-menu">
                    {menuItems.map(item => (
                        <div
                            class={`sidebar-item ${activeTab() === item.id ? 'active' : ''}`}
                            onClick={() => setActiveTab(item.id)}
                        >
                            <span class="sidebar-icon">{item.icon}</span>
                            <span class="sidebar-label">{item.label}</span>
                        </div>
                    ))}
                </div>
            </div>

            {/* 右侧主内容区域 */}
            <div class="settings-main-content">
                <Switch>
                    <Match when={activeTab() === 'provider'}>
                        <div class="tab-content-wrapper">
                            <ProviderSettings />
                        </div>
                    </Match>
                    <Match when={activeTab() === 'account'}>
                        <div class="tab-content-wrapper">
                            <AccountSettings />
                        </div>
                    </Match>
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