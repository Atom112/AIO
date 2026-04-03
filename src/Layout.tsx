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
    <div class="app-container min-h-screen flex flex-col bg-transparent">
      <NavBar />
      <main class="content-area flex-1 relative overflow-hidden">
        <Transition name="page-fade">
          {props.children}
        </Transition>
      </main>
    </div>
  );
};

export default Layout;