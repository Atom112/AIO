/**
 * @file index.tsx
 * @description 应用的核心入口文件，负责渲染根节点、配置全站路由及懒加载Suspense。
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
} from './core/store/store';
import { t } from './core/i18n';

// 应用启动时初始化项目列表
initProjects();
// 应用启动时初始化 MCP 服务器（加载配置 + 自动连接标记为 autoStart 的 server）
initMcpServers();
initSkills();
// 应用启动时初始化 per-profile 模型覆盖配置
initProfileModelOverrides();
// 应用启动时初始化自定义子智能体配置文件
initCustomSubagentProfiles();

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
        </Route>
      </Router>
    </Suspense>
  ),
  document.getElementById('root') as HTMLElement,
);
