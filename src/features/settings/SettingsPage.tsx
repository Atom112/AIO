import { Component, JSX, Show, For, createMemo } from 'solid-js';
import { A, useLocation } from '@solidjs/router';
import { Transition } from 'solid-transition-group';
import Icon from '../../shared/components/Icon';
import { t } from '../../core/i18n';

/**
 * Settings page layout (LobeHub v2 nested routes)
 * - Sidebar: switches top-level tabs (provider / account / app)
 * - Main area: renders the currently matched child route (props.children)
 * - The provider tab has sub-routes: /settings/provider (list) + /settings/provider/:id (detail)
 *   The sidebar hides in detail view; the detail page provides its own back button
 */
const Settings: Component<{ children?: JSX.Element }> = (props) => {
  const location = useLocation();

  /** Hide the sidebar on detail pages (matching /settings/provider/<id>) */
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
        id: 'embedding-models',
        path: '/settings/embedding-models',
        label: t('settings.embeddingModels'),
        icon: <Icon name="layers" class="w-5 h-5" />,
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
        {/* Sidebar */}
        <div
          class="w-[200px] flex flex-col rounded-lg overflow-hidden shrink-0"
          style={{
            background: 'rgba(var(--surface-bg), 0.25)',
            'backdrop-filter': 'blur(16px)',
            border: '1px solid var(--border-dim)',
          }}
        >
          <div class="px-5 py-6 text-lg text-[rgba(var(--text-base-rgb),0.6)] uppercase tracking-[2px] font-bold">
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
                                        ? 'bg-pri-10 text-white [text-shadow:0_0_8px_var(--primary-5)] border-pri-20 shadow-[inset_0_0_10px_var(--primary-10)]'
                                        : 'text-[rgba(var(--text-base-rgb),0.65)] hover:bg-pri-10 hover:text-white hover:pl-6'
                                    }`}
                >
                  <span
                    class={`absolute left-0 top-1/2 -translate-y-1/2 w-[3px] h-3/5 bg-pri rounded-r-sm transition-transform duration-300
                                        ${isActive(item.path) ? 'scale-y-100' : 'scale-y-0 group-hover:scale-y-50'}`}
                  />
                  <span
                    class={`flex items-center justify-center w-5 h-5 transition-colors duration-300 ${isActive(item.path) ? 'text-pri' : 'text-[rgba(var(--text-base-rgb),0.4)] group-hover:text-white'}`}
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

      {/* Main content area */}
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
