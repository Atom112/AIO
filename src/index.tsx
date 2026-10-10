/**
 * @file index.tsx
 * @description App entry point: renders the root node, wires up all routes, and enables lazy loading with Suspense.
 */
import { render } from 'solid-js/web';
import { Router, Route, Navigate } from '@solidjs/router';
import { lazy, Suspense } from 'solid-js';
import Layout from './Layout.tsx';
import './index.css';
import {
  initMcpServers,
  initSkills,
  initProjects,
  initProfileModelOverrides,
  initCustomSubagentProfiles,
  initEngineScan,
} from './core/store/store';
import { t } from './core/i18n';

// Initialize the project list on app startup
initProjects();
// Initialize MCP servers on app startup (load configs + auto-connect servers marked autoStart)
initMcpServers();
initSkills();
// Initialize per-profile model overrides on app startup
initProfileModelOverrides();
// Initialize custom subagent profiles on app startup
initCustomSubagentProfiles();
// Scan for installed inference engines on app startup
initEngineScan();

/**
 * Detect the weak Linux/WebKitGTK rendering engine and enable performance-degradation styles
 * only on that platform (`backdrop-filter: none`, opacity-only message animations). Windows(WebView2)/macOS(WKWebView)
 * do not set `data-webkitgtk`, so visuals and effects are unchanged.
 */
function enableWebKitGTKDegrade() {
  try {
    const ua = navigator.userAgent.toLowerCase();
    const isWebKitGTK = ua.includes('linux') && ua.includes('webkit') && !ua.includes('chrome');
    if (isWebKitGTK) {
      document.documentElement.setAttribute('data-webkitgtk', '');
    }
  } catch {
    /* Ignore in non-browser environments */
  }
}
enableWebKitGTKDegrade();

const Settings = lazy(() => import('./features/settings/SettingsPage'));
const ProviderList = lazy(() => import('./features/settings/components/ProviderList'));
const ProviderDetail = lazy(() => import('./features/settings/ProviderDetailPage'));
const AppSettings = lazy(() => import('./features/settings/components/AppSettings'));
const McpServerList = lazy(() => import('./features/settings/components/McpServerList'));
const SkillList = lazy(() => import('./features/settings/components/SkillList'));
const SubagentModelSettings = lazy(
  () => import('./features/settings/components/SubagentModelSettings'),
);
const UsageSettings = lazy(() => import('./features/settings/components/UsageSettings'));
const EmbeddingModelSettings = lazy(
  () => import('./features/settings/components/EmbeddingModelSettings'),
);
const LocalModelMarket = lazy(() => import('./features/settings/components/LocalModelMarket'));

render(
  () => (
    <Suspense
      fallback={
        <div class="flex items-center justify-center h-screen w-screen text-white/40">
          {t('common.loading')}
        </div>
      }
    >
      <Router root={Layout}>
        <Route path="/" component={() => <Navigate href="/chat" />} />
        <Route path="/chat" component={lazy(() => import('./features/chat/ChatPage'))} />
        <Route path="/settings" component={Settings}>
          <Route path="" component={ProviderList} />
          <Route path="/provider/:providerId" component={ProviderDetail} />
          <Route path="/mcp" component={McpServerList} />
          <Route path="/skills" component={SkillList} />
          <Route path="/usage" component={UsageSettings} />
          <Route path="/app" component={AppSettings} />
          <Route path="/subagent-models" component={SubagentModelSettings} />
          <Route path="/embedding-models" component={EmbeddingModelSettings} />
          <Route path="/local-models" component={LocalModelMarket} />
        </Route>
      </Router>
    </Suspense>
  ),
  document.getElementById('root') as HTMLElement,
);
