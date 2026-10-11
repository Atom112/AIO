/**
 * @file Layout.tsx
 * @description Root layout component: the whole window is a single acrylic panel (NavBar + pages share one glass surface);
 * the background is a static multi-color gradient (unaffected by the theme color), and the theme color only applies to interactive elements like buttons and switches.
 */
import NavBar from './shared/components/NavBar';
import UpdateNotification from './shared/components/UpdateNotification';
import GlobalKeyboardHandler from './shared/components/GlobalKeyboardHandler';
import CommandPalette from './shared/components/CommandPalette';
import { Transition } from 'solid-transition-group';
import { Component, createSignal, onCleanup, onMount, ParentProps } from 'solid-js';
import { loadProviderConfigs } from './core/api/config';
import { checkAppUpdate } from './core/api/update';
import { Window } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import { getVersion } from '@tauri-apps/api/app';
import { loadModelsCatalog, updateModelsCatalog, getCatalogMeta } from './core/utils/models';
import { EVENTS } from './core/api/events';
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
import { registerChatStreamListeners } from './features/chat/hooks/chatEventListeners';

/**
 * Layout component
 * @param props - contains children (the matched route's component content)
 * @returns a content area with global navigation and transition animations
 */
const Layout: Component<ParentProps> = (props) => {
  // Whether the window is maximized (a transparent window should drop rounded corners when maximized so the desktop doesn't show through the corners)
  const [windowMaximized, setWindowMaximized] = createSignal(false);
  const appWindow = new Window('main');
  let unlistenDiagnostics: (() => void) | undefined;
  let unlistenResized: (() => void) | undefined;

  /**
   * Check for app updates automatically on startup
   * Flow: delay 1.5s (to avoid the first-paint peak) -> call check_app_update
   *      -> if a new version is found and not user-ignored, light up the bottom-left Toast
   */
  onMount(async () => {
    const currentVersion = await getVersion().catch(() => '');

    // Initialize the maximized state and keep it in sync on window resize (drives rounded corners)
    setWindowMaximized(await appWindow.isMaximized().catch(() => false));
    unlistenResized = await appWindow.onResized(async () => {
      setWindowMaximized(await appWindow.isMaximized().catch(() => false));
    });

    // Reset one-shot state on startup (guards against leftovers from the previous session)
    setAppUpdateAvailable(false);
    setAppUpdateDismissed(false);
    setAppUpdateDownloading(false);
    setAppUpdateProgress(0);
    setAppUpdateReady(false);

    // Listen for LSP diagnostics update events
    unlistenDiagnostics = await listen<LspDiagnosticsUpdatePayload>(
      EVENTS.lspDiagnosticsUpdated,
      (event) => {
        updateDiagnostics(event.payload);
      },
    );

    // CHAT-12: register the chat stream listeners once at the root — they must
    // outlive every page component. They used to be registered inside ChatPage, so
    // leaving the chat page mid-generation destroyed them: the backend kept
    // streaming but chunks never reached the store and the final `done` handler
    // (which persists the assistant) never ran, losing the whole turn. The Layout
    // never unmounts, so these listeners stay for the whole app lifetime.
    registerChatStreamListeners();

    // Load the models catalog asynchronously (non-blocking for first paint; consumed on demand by the settings page)
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

    // Load provider configs asynchronously (once after startup)
    loadProviderConfigs()
      .then((file) => setProviderConfigs(file.providers))
      .catch((e) => console.warn('[provider-configs] startup load failed:', e));

    // Silently check for catalog updates in the background (fires only after the initial load completes, to avoid blocking first paint)
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
        console.warn('[catalog] startup background update failed:', e);
      }
    }, 8000);

    setTimeout(async () => {
      try {
        const result = await checkAppUpdate();

        if (result.kind === 'update_available') {
          const info = result.info;
          // Skip re-prompting if the user already snoozed this version
          if (getIgnoredUpdateVersion() === info.version) return;

          setAppUpdateInfo({
            version: info.version,
            currentVersion: info.currentVersion || currentVersion,
            notes: info.notes,
            pubDate: info.pubDate,
          });
          setAppUpdateAvailable(true);
        }
        // Other cases: up_to_date / service_not_ready / network / failed -> stay silent
      } catch (e) {
        // Handle network failures silently; do not disturb the user
        console.warn('Startup update check failed:', e);
      }
    }, 1500);
  });

  onCleanup(() => {
    // Clean up the LSP diagnostics listener and the window resize listener
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
