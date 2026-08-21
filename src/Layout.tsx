/**
 * @file Layout.tsx
 * @description 应用的通用布局组件, 整窗为统一亚克力面板(NavBar + 页面共享同块玻璃),
 * 背景为静态多色渐变(不受主题色影响), 主题色仅作用于按钮/开关等交互元素.
 */
import NavBar from './shared/components/NavBar';
import UpdateNotification from './shared/components/UpdateNotification';
import GlobalKeyboardHandler from './shared/components/GlobalKeyboardHandler';
import CommandPalette from './shared/components/CommandPalette';
import { Transition } from 'solid-transition-group';
import { Component, createSignal, onCleanup, onMount, ParentProps } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { Window } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import { getVersion } from '@tauri-apps/api/app';
import { loadModelsCatalog, updateModelsCatalog, getCatalogMeta } from './core/utils/models';
import {
  setAppUpdateAvailable,
  setAppUpdateInfo,
  setAppUpdateDismissed,
  setAppUpdateDownloading,
  setAppUpdateProgress,
  setAppUpdateReady,
  getIgnoredUpdateVersion,
  setModelsCatalog,
  setModelsCatalogStatus,
  setModelsCatalogSource,
  setModelsCatalogPath,
  setModelsCatalogVersion,
  setModelsCatalogGeneratedAt,
  setProviderConfigs,
} from './core/store/store';
import { updateDiagnostics, type LspDiagnosticsUpdatePayload } from './core/store/diagnostics';
import type { ProviderConfigFile } from './core/utils/models';

/**
 * Layout 组件
 * @param props - 包含 children (当前匹配路由指向的组件内容)
 * @returns 返回一个包含通用导航和过渡动画的内容区域
 */
const Layout: Component<ParentProps> = (props) => {
  // 窗口是否处于最大化（透明窗口下最大化时应取消圆角，避免四角透出桌面）
  const [windowMaximized, setWindowMaximized] = createSignal(false);
  const appWindow = new Window('main');
  let unlistenDiagnostics: (() => void) | undefined;
  let unlistenResized: (() => void) | undefined;

  /**
   * 应用启动时自动检查更新
   * 流程：延迟 1.5s（避开首屏渲染高峰）→ 调用 check_app_update
   *      → 若发现新版本且未被用户忽略则点亮左下角 Toast
   */
  onMount(async () => {
    const currentVersion = await getVersion().catch(() => '');

    // 初始化最大化状态，并在窗口尺寸变化时同步（控制圆角）
    setWindowMaximized(await appWindow.isMaximized().catch(() => false));
    unlistenResized = await appWindow.onResized(async () => {
      setWindowMaximized(await appWindow.isMaximized().catch(() => false));
    });

    // 启动时重置一次性状态（防止上次会话残留）
    setAppUpdateAvailable(false);
    setAppUpdateDismissed(false);
    setAppUpdateDownloading(false);
    setAppUpdateProgress(0);
    setAppUpdateReady(false);

    // 监听 LSP 诊断更新事件
    unlistenDiagnostics = await listen<LspDiagnosticsUpdatePayload>(
      'lsp-diagnostics-updated',
      (event) => {
        updateDiagnostics(event.payload);
      },
    );

    // 异步加载模型目录（不阻塞首屏，由 settings 页面按需使用）
    setModelsCatalogStatus('loading');
    loadModelsCatalog()
      .then((c) => {
        const meta = getCatalogMeta();
        setModelsCatalog(c);
        setModelsCatalogSource(meta.source);
        setModelsCatalogPath(meta.path);
        setModelsCatalogVersion(meta.version);
        setModelsCatalogGeneratedAt(meta.generatedAt);
        setModelsCatalogStatus('ready');
      })
      .catch(() => setModelsCatalogStatus('failed'));

    // 异步加载 provider 配置（启动后做一次）
    invoke<ProviderConfigFile>('load_provider_configs')
      .then((file) => setProviderConfigs(file.providers))
      .catch((e) => console.warn('[provider-configs] 启动加载失败:', e));

    // 静默后台检查 catalog 更新（仅在已加载完成后再触发，避免阻塞首屏）
    setTimeout(async () => {
      try {
        const result = await updateModelsCatalog();
        if (result.success) {
          const cat = await loadModelsCatalog();
          const meta = getCatalogMeta();
          setModelsCatalog(cat);
          setModelsCatalogSource(meta.source);
          setModelsCatalogPath(meta.path);
          setModelsCatalogVersion(meta.version);
          setModelsCatalogGeneratedAt(meta.generatedAt);
        }
      } catch (e) {
        console.warn('[catalog] 启动静默更新失败:', e);
      }
    }, 8000);

    setTimeout(async () => {
      try {
        type CheckUpdateResult =
          | { kind: 'up_to_date'; current_version: string }
          | {
              kind: 'update_available';
              info: { version: string; current_version: string; notes?: string; pub_date?: string };
            }
          | { kind: 'service_not_ready'; current_version: string; endpoint: string; reason: string }
          | { kind: 'network'; current_version: string; endpoint: string; reason: string }
          | { kind: 'failed'; current_version: string; endpoint: string; reason: string };

        const result = await invoke<CheckUpdateResult>('check_app_update');

        if (result.kind === 'update_available') {
          const info = result.info;
          // 用户已经点过"稍后"过该版本就不再提示
          if (getIgnoredUpdateVersion() === info.version) return;

          setAppUpdateInfo({
            version: info.version,
            currentVersion: info.current_version || currentVersion,
            notes: info.notes,
            pubDate: info.pub_date,
          });
          setAppUpdateAvailable(true);
        }
        // 其他情况：up_to_date / service_not_ready / network / failed → 静默不打扰
      } catch (e) {
        // 网络失败静默处理，不打扰用户
        console.warn('启动时检查更新失败:', e);
      }
    }, 1500);
  });

  onCleanup(() => {
    // 清理 LSP 事件监听与窗口尺寸监听
    unlistenDiagnostics?.();
    unlistenResized?.();
  });

  return (
    <div
      class="app-container h-screen flex flex-col overflow-hidden rounded-xl"
      classList={{ 'rounded-none': windowMaximized() }}
      style={{ background: 'var(--app-bg)' }}
    >
      <GlobalKeyboardHandler />
      <CommandPalette />
      <NavBar />
      <main class="flex-1 relative overflow-hidden">
        <Transition name="page-fade">{props.children}</Transition>
      </main>
      <UpdateNotification />
    </div>
  );
};

export default Layout;
