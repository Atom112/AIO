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
    // 当 lazy 加载组件时，显示 fallback 内容
    <Suspense fallback={<div class="loading-container">Loading...</div>}>
      
      {/* 
          Router 容器 
          root={Layout} 表示所有子路由都会被包裹在 Layout 组件内渲染
      */}
      <Router root={Layout}>
        
        {/* 根路径重定向：访问 "/" 时自动跳转到 "/chat" */}
        <Route path="/" component={() => <Navigate href="/chat" />} />
        
        {/* 
            聊天页面路由
            使用 lazy 进行代码分割，只有进入该路径时才会加载对应的 JS 文件
        */}
        <Route path="/chat" component={lazy(() => import("./pages/Chat.tsx"))} />
        
        {/* 设置页面路由 */}
        <Route path="/settings" component={lazy(() => import("./pages/Settings.tsx"))} />
        
      </Router>
    </Suspense>
  ),
  document.getElementById('root') as HTMLElement
);