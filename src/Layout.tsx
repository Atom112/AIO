/**
 * @file Layout.tsx
 * @description 应用的通用布局组件，包含固定的导航栏和动态变化的子页面区域。
 */

import NavBar from "./components/NavBar";
import { Transition } from "solid-transition-group";
import { Component, ParentProps } from "solid-js";

/**
 * Layout 组件
 * @param props - 包含 children (当前匹配路由指向的组件内容)
 * 
 * @returns 返回一个包含通用导航和过渡动画的内容区域
 */
const Layout: Component<ParentProps> = (props) => {
  return (
    <div class="app-container">
      {/* 
          公共导航栏组件 
          它在页面切换时保持挂载状态，不会重新渲染 
      */}
      <NavBar />

      {/* 
          核心内容渲染区域
          使用 Transition 组件为路由切换添加过渡效果 
          需要在 index.css 中定义 .page-fade-enter, .page-fade-exit 等动画样式
      */}
      <main class="content-area">
        <Transition name="page-fade">
          {/* 
              props.children 
              由于 Layout 被作为 <Router root={Layout}> 传递，
              props.children 会自动接收并渲染当前匹配到的 <Route> 组件
          */}
          {props.children}
        </Transition>
      </main>
    </div>
  );
};

export default Layout;