import { render } from 'solid-js/web';
import { Router, Route, Navigate } from '@solidjs/router';
import { lazy, Suspense } from 'solid-js';
import Layout from './Layout.tsx';
import './index.css';

// 包裹 Router 组件以启用路由功能
render(
  () => (
      <Suspense fallback={<div>Loading...</div>}>
          <Router root={Layout}>
            <Route path="/" component={() => <Navigate href="/chat" />} />
            <Route path="/chat" component={lazy(() => import("./pages/Chat.tsx"))} />
            <Route path="/settings" component={lazy(() => import("./pages/Settings.tsx"))} />
          </Router>
      </Suspense>
  ),
  document.getElementById('root') as HTMLElement
);