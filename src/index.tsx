/**
 * @file index.tsx
 * @description 应用的核心入口文件，负责渲染根节点、配置全站路由及懒加载Suspense。
 */
import { render } from 'solid-js/web';
import { Router, Route, Navigate } from '@solidjs/router';
import { lazy, Suspense } from 'solid-js';
import Layout from './Layout.tsx';
import './index.css';

render(
  () => (
    <Suspense fallback={<div class="loading-container">Loading...</div>}>
      <Router root={Layout}>
        <Route path="/" component={() => <Navigate href="/chat" />} />
        <Route path="/chat" component={lazy(() => import("./pages/Chat.tsx"))} />
        <Route path="/settings" component={lazy(() => import("./pages/Settings.tsx"))} />
      </Router>
    </Suspense>
  ),
  document.getElementById('root') as HTMLElement
);