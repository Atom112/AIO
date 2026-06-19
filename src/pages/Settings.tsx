import { Component, JSX, Show } from 'solid-js';
import { A, useLocation } from '@solidjs/router';
import Icon from '../components/Icon';

/**
 * 设置页面布局 (lobehub v2 嵌套路由)
 * - 侧栏: 切换顶级 tab (provider / account / app)
 * - 主区域: 渲染当前匹配的子路由 (props.children)
 * - provider tab 下还有子路由: /settings/provider (列表) + /settings/provider/:id (详情)
 *   进入详情时侧栏隐藏, 详情页自带返回按钮
 */
const Settings: Component<{ children?: JSX.Element }> = (props) => {
    const location = useLocation();

    /** 详情页 (匹配 /settings/provider/<id>) 时隐藏侧栏 */
    const isProviderDetail = () => /^\/settings\/provider\/[^/]+/.test(location.pathname);

    const menuItems: Array<{ id: string; path: string; label: string; icon: JSX.Element }> = [
        { id: 'provider', path: '/settings', label: '供应商设置', icon: <Icon src="/icons/app-logo/provider.svg" class="w-5 h-5" /> },
        { id: 'account', path: '/settings/account', label: '账号信息', icon: <Icon src="/icons/app-logo/account.svg" class="w-5 h-5" /> },
        { id: 'app', path: '/settings/app', label: '应用信息', icon: <Icon src="/icons/app-logo/app-grid.svg" class="w-5 h-5" /> },
    ];

    const isActive = (path: string) => {
        if (path === '/settings') return location.pathname === '/settings' || location.pathname.startsWith('/settings/provider');
        return location.pathname.startsWith(path);
    };

    return (
        <div
            class="h-full flex p-4 gap-4"
            style="background: transparent;"
        >
            <Show when={!isProviderDetail()}>
                {/* 侧边栏 */}
                <div class="w-[200px] flex flex-col rounded-lg overflow-hidden shrink-0" style="background: rgba(18, 22, 35, 0.25); backdrop-filter: blur(30px); border: 1px solid rgba(255, 255, 255, 0.06);">
                    <div class="px-5 py-6 text-lg text-[#999] uppercase tracking-[2px] font-bold">
                        设置中心
                    </div>

                    <div class="flex flex-col px-2">
                        {menuItems.map(item => (
                            <A
                                href={item.path}
                                class={`relative px-5 py-4 my-1 cursor-pointer flex items-center gap-3 rounded-lg border border-transparent transition-all duration-300 ease-[cubic-bezier(0.4,0,0.2,1)] group overflow-hidden no-underline
                                    ${isActive(item.path)
                                        ? 'bg-pri-10 text-pri [text-shadow:0_0_8px_var(--primary-5)] border-pri-20 shadow-[inset_0_0_10px_var(--primary-10)]'
                                        : 'text-[#aaa] hover:bg-pri-10 hover:text-white hover:pl-6'
                                    }`}
                            >
                                <span
                                    class={`absolute left-0 top-1/2 -translate-y-1/2 w-[3px] h-3/5 bg-pri rounded-r-sm transition-transform duration-300
                                        ${isActive(item.path) ? 'scale-y-100' : 'scale-y-0 group-hover:scale-y-50'}`}
                                />
                                <span class={`flex items-center justify-center w-5 h-5 transition-colors duration-300 ${isActive(item.path) ? 'text-pri' : 'text-[#666] group-hover:text-white'}`}>
                                    {item.icon}
                                </span>
                                <span class="text-sm font-medium leading-none">
                                    {item.label}
                                </span>
                            </A>
                        ))}
                    </div>
                </div>
            </Show>

            {/* 主内容区 */}
            <div class="flex-1 overflow-hidden min-w-0">
                {props.children}
            </div>
        </div>
    );
};

export default Settings;

