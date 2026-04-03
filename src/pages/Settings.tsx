import { Component, createSignal, Switch, Match, JSX } from 'solid-js';
import { Transition } from 'solid-transition-group';
import ProviderSettings from '../components/ProviderSettings';
import AccountSettings from '../components/AccountSettings';
import AppSettings from '../components/AppSettings';
import Icon from '../components/Icon';

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
        { id: 'provider', label: '供应商设置', icon: <Icon src="/icons/app-logo/provider.svg" class="w-5 h-5" /> },
        { id: 'account', label: '账号信息', icon: <Icon src="/icons/app-logo/account.svg" class="w-5 h-5" /> },
        { id: 'app', label: '应用信息', icon: <Icon src="/icons/app-logo/app-grid.svg" class="w-5 h-5" /> },
    ];

return (
        <div class="flex fixed inset-[65px_1px_1px_0] p-4 glow-border rounded-lg gap-4 bg-dark">
            {/* 侧边栏 */}
            <div class="w-[200px] bg-black/30 glow-border flex flex-col rounded-lg overflow-hidden">
                <div class="px-5 py-6 text-lg text-[#999] uppercase tracking-[2px] font-bold">
                    设置中心
                </div>

                <div class="flex flex-col px-2">
                    {menuItems.map(item => (
                        <div
                            class={`relative px-5 py-4 my-1 cursor-pointer flex items-center gap-3 rounded-lg border border-transparent transition-all duration-300 ease-[cubic-bezier(0.4,0,0.2,1)] group overflow-hidden
                                ${activeTab() === item.id 
                                    ? 'bg-pri-10 text-pri [text-shadow:0_0_8px_var(--primary-5)] border-pri-20 shadow-[inset_0_0_10px_var(--primary-10)]' 
                                    : 'text-[#aaa] hover:bg-pri-10 hover:text-white hover:pl-6'
                                }`}
                            onClick={() => setActiveTab(item.id)}
                        >
                            {/* 激活状态的左侧指示条 (原 ::before) */}
                            <span 
                                class={`absolute left-0 top-1/2 -translate-y-1/2 w-[3px] h-3/5 bg-pri rounded-r-sm transition-transform duration-300 
                                    ${activeTab() === item.id ? 'scale-y-100' : 'scale-y-0 group-hover:scale-y-50'}`}
                            />
                            
                            <span class={`flex items-center justify-center w-5 h-5 transition-colors duration-300 ${activeTab() === item.id ? 'text-pri' : 'text-[#666] group-hover:text-white'}`}>
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
                <Transition name="settings-tab-fade" mode="outin">
                    <Switch>
                        <Match when={activeTab() === 'provider'}>
                            <div class="h-full">
                                <ProviderSettings />
                            </div>
                        </Match>
                        <Match when={activeTab() === 'account'}>
                            <div class="h-full">
                                <AccountSettings />
                            </div>
                        </Match>
                        <Match when={activeTab() === 'app'}>
                            <div class="h-full">
                                <AppSettings />
                            </div>
                        </Match>
                    </Switch>
                </Transition>
            </div>
        </div>
    );
};

export default Settings;