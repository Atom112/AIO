/**
 * @file Layout.tsx
 * @description Root layout component: the whole window is a single acrylic panel (NavBar + pages share one glass surface);
 * the background is the `--app-bg` multi-color gradient (unaffected by the theme color) plus an optional user
 * background layer (solid / gradient / image, light mode only), and the theme color only applies to interactive
 * elements like buttons and switches.
 */
import NavBar from './shared/components/NavBar';
import UpdateNotification from './shared/components/UpdateNotification';
import GlobalKeyboardHandler from './shared/components/GlobalKeyboardHandler';
import CommandPalette from './shared/components/CommandPalette';
import { Transition } from 'solid-transition-group';
import {
  Component,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  ParentProps,
  Show,
} from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
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
  appBackground,
  isDarkMode,
  buildBackgroundLayerStyle,
  appFont,
  appFontStack,
  appFontSize,
  appFontScale,
} from './core/store/store';
import { updateDiagnostics, type LspDiagnosticsUpdatePayload } from './core/store/diagnostics';
import { registerChatStreamListeners } from './features/chat/hooks/chatEventListeners';
import { registerBackgroundTaskListeners } from './features/chat/hooks/backgroundTaskListeners';
import { registerHarnessEventListeners } from './features/harness/hooks/harnessEventListeners';

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

    // Background-task listeners (memory jobs / index rebuilds / model downloads)
    // also live for the whole app: the jobs keep running no matter which page is
    // open, and the status-bar pull-up menu reads the registry from anywhere.
    registerBackgroundTaskListeners();

    // Harness workbench listeners (CLI agent event stream / approvals / states):
    // harness agents keep running across page navigation — the drivers feed the
    // store from here no matter which page is open.
    registerHarnessEventListeners();

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

  // Custom background layer (solid / gradient / image) — null in dark mode or on defaults.
  const backgroundStyle = createMemo(() =>
    buildBackgroundLayerStyle(appBackground(), isDarkMode(), convertFileSrc),
  );

  // App font (settings → App settings): applied on <body> so every page inherits it
  // (index.css gives `html, body` an explicit font stack, so the root alone would not
  // cascade); mono/code surfaces keep their own explicit font-family stacks.
  createEffect(() => {
    const stack = appFontStack(appFont());
    if (stack) document.body.style.setProperty('font-family', stack);
    else document.body.style.removeProperty('font-family');
  });

  // App font size: the CSS text tokens multiply by this scale (Layout's effect
  // keeps live slider changes in sync; index.tsx sets the initial value).
  createEffect(() => {
    document.documentElement.style.setProperty(
      '--app-font-scale',
      String(appFontScale(appFontSize())),
    );
  });

  return (
    <div
      class="app-container relative h-screen flex flex-col overflow-hidden rounded-xl"
      classList={{ 'rounded-none': windowMaximized() }}
      style={{ background: 'var(--app-bg)' }}
    >
      {/* Sits between the default --app-bg and the UI; opacity < 100 lets --app-bg show through */}
      <Show when={backgroundStyle()}>
        <div class="app-bg-layer" style={backgroundStyle()!} aria-hidden="true" />
      </Show>
      <div id="app-content" class="relative z-[1] flex flex-col flex-1 min-h-0">
        <GlobalKeyboardHandler />
        <CommandPalette />
        <NavBar />
        <main class="flex-1 relative overflow-hidden">
          <Transition name="page-fade">{props.children}</Transition>
        </main>
        <UpdateNotification />
      </div>
    </div>
  );
};

export default Layout;
