/**
 * @file index.tsx
 * @description 应用的核心入口文件，负责渲染根节点、配置全站路由及懒加载Suspense。
 */
import { render } from 'solid-js/web';
import { Router, Route, Navigate } from '@solidjs/router';
import { lazy, Suspense } from 'solid-js';
import Layout from './Layout.tsx';
import './index.css';

/**
 * 使用 render 函数将 Solid 应用挂载到 DOM。
 * 
 * 结构说明：
 * 1. <Suspense>: 处理 lazy(() => import(...)) 时的过渡状态。
 * 2. <Router>: 定义路由容器，root 属性指定了全局布局组件。
 * 3. <Route>: 具体路径配置。
 */
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