import { Component, JSX, Show, For, createMemo } from 'solid-js';
import { A, useLocation } from '@solidjs/router';
import { Transition } from 'solid-transition-group';
import Icon from '../../shared/components/Icon';
import { t } from '../../core/i18n';

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

  const menuItems = createMemo(
    (): Array<{ id: string; path: string; label: string; icon: JSX.Element }> => [
      {
        id: 'provider',
        path: '/settings',
        label: t('settings.providers'),
        icon: <Icon src="/icons/app-logo/provider.svg" class="w-5 h-5" />,
      },
      {
        id: 'mcp',
        path: '/settings/mcp',
        label: t('settings.mcp'),
        icon: <Icon src="/icons/app-logo/mcp.svg" class="w-5 h-5" />,
      },
      {
        id: 'skills',
        path: '/settings/skills',
        label: t('settings.skills'),
        icon: <Icon src="/icons/app-logo/prompt.svg" class="w-5 h-5" />,
      },
      {
        id: 'usage',
        path: '/settings/usage',
        label: t('settings.usage'),
        icon: <Icon src="/icons/app-logo/chart.svg" class="w-5 h-5" />,
      },
      {
        id: 'subagent-models',
        path: '/settings/subagent-models',
        label: t('settings.subagents'),
        icon: <Icon name="sparkles" class="w-5 h-5" />,
      },
      {
        id: 'app',
        path: '/settings/app',
        label: t('settings.app'),
        icon: <Icon src="/icons/app-logo/app-grid.svg" class="w-5 h-5" />,
      },
    ],
  );

  const isActive = (path: string) => {
    if (path === '/settings')
      return (
        location.pathname === '/settings' || location.pathname.startsWith('/settings/provider')
      );
    return location.pathname.startsWith(path);
  };

  return (
    <div class="h-full flex p-4 gap-4" style={{ background: 'transparent' }}>
      <Show when={!isProviderDetail()}>
        {/* 侧边栏 */}
        <div
          class="w-[200px] flex flex-col rounded-lg overflow-hidden shrink-0"
          style={{
            background: 'rgba(var(--surface-bg), 0.25)',
            'backdrop-filter': 'blur(30px)',
            border: '1px solid var(--border-dim)',
          }}
        >
          <div class="px-5 py-6 text-lg text-[#999] uppercase tracking-[2px] font-bold">
            {t('settings.title')}
          </div>

          <div class="flex flex-col px-2">
            <For each={menuItems()}>
              {(item) => (
                <A
                  href={item.path}
                  class={`relative px-5 py-4 my-1 cursor-pointer flex items-center gap-3 rounded-lg border border-transparent transition-all duration-300 ease-[cubic-bezier(0.4,0,0.2,1)] group overflow-hidden no-underline
                                    ${
                                      isActive(item.path)
                                        ? 'bg-pri-10 text-pri [text-shadow:0_0_8px_var(--primary-5)] border-pri-20 shadow-[inset_0_0_10px_var(--primary-10)]'
                                        : 'text-[#aaa] hover:bg-pri-10 hover:text-white hover:pl-6'
                                    }`}
                >
                  <span
                    class={`absolute left-0 top-1/2 -translate-y-1/2 w-[3px] h-3/5 bg-pri rounded-r-sm transition-transform duration-300
                                        ${isActive(item.path) ? 'scale-y-100' : 'scale-y-0 group-hover:scale-y-50'}`}
                  />
                  <span
                    class={`flex items-center justify-center w-5 h-5 transition-colors duration-300 ${isActive(item.path) ? 'text-pri' : 'text-[#666] group-hover:text-white'}`}
                  >
                    {item.icon}
                  </span>
                  <span class="text-sm font-medium leading-none">{item.label}</span>
                </A>
              )}
            </For>
          </div>
        </div>
      </Show>

      {/* 主内容区 */}
      <Transition name="subpage-fade">
        <div
          class="flex-1 overflow-y-auto overflow-x-hidden min-w-0"
          {...({ key: location.pathname } as any)}
        >
          {props.children}
        </div>
      </Transition>
    </div>
  );
};

export default Settings;
