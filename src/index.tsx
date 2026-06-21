/**
 * @file index.tsx
 * @description 应用的核心入口文件，负责渲染根节点、配置全站路由及懒加载Suspense。
 */
import { render } from 'solid-js/web';
import { Router, Route, Navigate } from '@solidjs/router';
import { lazy, Suspense } from 'solid-js';
import Layout from './Layout.tsx';
import './index.css';
import { initMcpServers, initSkills } from './store/store';

// 应用启动时初始化 MCP 服务器（加载配置 + 自动连接标记为 autoStart 的 server）
initMcpServers();
initSkills();

const Settings = lazy(() => import('./pages/Settings.tsx'));
const ProviderList = lazy(() => import('./components/ProviderList.tsx'));
const ProviderDetail = lazy(() => import('./pages/ProviderDetail.tsx'));
const AccountSettings = lazy(() => import('./components/AccountSettings.tsx'));
const AppSettings = lazy(() => import('./components/AppSettings.tsx'));
const McpServerList = lazy(() => import('./components/McpServerList.tsx'));
const SkillList = lazy(() => import('./components/SkillList.tsx'));

render(
  () => (
    <Suspense fallback={<div class="loading-container">Loading...</div>}>
      <Router root={Layout}>
        <Route path="/" component={() => <Navigate href="/chat" />} />
        <Route path="/chat" component={lazy(() => import('./pages/Chat.tsx'))} />
        <Route path="/settings" component={Settings}>
          <Route path="" component={ProviderList} />
          <Route path="/provider/:providerId" component={ProviderDetail} />
          <Route path="/mcp" component={McpServerList} />
          <Route path="/skills" component={SkillList} />
          <Route path="/account" component={AccountSettings} />
          <Route path="/app" component={AppSettings} />
        </Route>
      </Router>
    </Suspense>
  ),
  document.getElementById('root') as HTMLElement
);
