/**
 * @file Layout.tsx
 * @description 应用的通用布局组件, 整窗为统一亚克力面板(NavBar + 页面共享同块玻璃),
 * 背景为静态多色渐变(不受主题色影响), 主题色仅作用于按钮/开关等交互元素.
 */
import NavBar from "./components/NavBar";
import { Transition } from "solid-transition-group";
import { Component, ParentProps } from "solid-js";

/**
 * Layout 组件
 * @param props - 包含 children (当前匹配路由指向的组件内容)
 * @returns 返回一个包含通用导航和过渡动画的内容区域
 */
const Layout: Component<ParentProps> = (props) => {
  return (
    <div
      class="app-container h-screen flex flex-col overflow-hidden rounded-xl"
      style={{
        background: [
          "radial-gradient(ellipse 100% 70% at 25% 15%, rgba(70, 120, 200, 0.45), transparent 60%)",
          "radial-gradient(ellipse 80% 90% at 70% 85%, rgba(50, 90, 140, 0.35), transparent 55%)",
          "linear-gradient(135deg, #1e3a5f 0%, #2d2d5e 30%, #3d2d5a 50%, #2a2050 70%, #1a2540 100%)",
        ].join(", "),
      }}
    >
      <NavBar />
      <main class="flex-1 relative overflow-hidden">
        <Transition name="page-fade">
          {props.children}
        </Transition>
      </main>
    </div>
  );
};

export default Layout;