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
        <svg class='block' xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" />
        </svg>
    ),
    
    /**
     * 账号图标 - 用户轮廓（头部圆形 + 肩部线条）
     * @returns JSX.Element SVG 图标元素
     */
    Account: () => (
        <svg class='block' xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
            <circle cx="12" cy="7" r="4" />
        </svg>
    ),
    
    /**
     * 应用图标 - 九宫格/网格布局，象征应用程序界面
     * @returns JSX.Element SVG 图标元素
     */
    App: () => (
        <svg class='block' xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
            <line x1="9" y1="3" x2="9" y2="21" />
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
        <div class="flex fixed inset-[65px_1px_1px_0] p-4 border border-[var(--primary-color)] shadow-[inset_0_0_20px_1px_var(--primary-30)] rounded-lg gap-4 bg-[#1e1e1e]">
            {/* 侧边栏 */}
            <div class="w-[200px] bg-black/30 border border-[var(--primary-color)] shadow-[inset_0_0_20px_1px_var(--primary-30)] flex flex-col rounded-lg overflow-hidden">
                <div class="px-5 py-6 text-lg text-[#999] uppercase tracking-[2px] font-bold">
                    设置中心
                </div>

                <div class="flex flex-col px-2">
                    {menuItems.map(item => (
                        <div
                            class={`relative px-5 py-4 my-1 cursor-pointer flex items-center gap-3 rounded-lg border border-transparent transition-all duration-300 ease-[cubic-bezier(0.4,0,0.2,1)] group overflow-hidden
                                ${activeTab() === item.id 
                                    ? 'bg-[var(--primary-10)] text-[var(--primary-color)] [text-shadow:0_0_8px_var(--primary-5)] border-[var(--primary-20)] shadow-[inset_0_0_10px_var(--primary-10)]' 
                                    : 'text-[#aaa] hover:bg-[var(--primary-10)] hover:text-white hover:pl-6'
                                }`}
                            onClick={() => setActiveTab(item.id)}
                        >
                            {/* 激活状态的左侧指示条 (原 ::before) */}
                            <span 
                                class={`absolute left-0 top-1/2 -translate-y-1/2 w-[3px] h-3/5 bg-[var(--primary-color)] rounded-r-sm transition-transform duration-300 
                                    ${activeTab() === item.id ? 'scale-y-100' : 'scale-y-0 group-hover:scale-y-50'}`}
                            />
                            
                            <span class={`flex items-center justify-center w-5 h-5 transition-colors duration-300 ${activeTab() === item.id ? 'text-[var(--primary-color)]' : 'text-[#666] group-hover:text-white'}`}>
                                {item.icon}
                            </span>
                            <span class="text-sm font-medium leading-none">
                                {item.label}
                            </span>
                        </div>
                    ))}
                </div>
            </div>

            {/* 主内容区 */}
            <div class="flex-1 overflow-y-auto pr-1 scroll-smooth bg-[radial-gradient(circle_at_50%_50%,rgb(8_221_249/0.02)_0%,transparent_80%)]">
                <Switch>
                    <Match when={activeTab() === 'provider'}>
                        <div class="tab-content-wrapper h-full animate-in fade-in slide-in-from-bottom-2 duration-400">
                            <ProviderSettings />
                        </div>
                    </Match>
                    
                    <Match when={activeTab() === 'account'}>
                        <div class="tab-content-wrapper h-full animate-in fade-in slide-in-from-bottom-2 duration-400">
                            <AccountSettings />
                        </div>
                    </Match>
                    
                    <Match when={activeTab() === 'app'}>
                        <div class="tab-content-wrapper h-full animate-in fade-in slide-in-from-bottom-2 duration-400">
                            <AppSettings />
                        </div>
                    </Match>
                </Switch>
            </div>
        </div>
    );
};

export default Settings;