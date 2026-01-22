import NavBar from "./components/NavBar";
import { Transition } from "solid-transition-group";
import { Component, ParentProps } from "solid-js";

const Layout: Component<ParentProps> = (props) => {
  return (
    <>
      {/* 布局组件，包含导航栏和内容区域，导航栏由多个页面共享 */}
      <NavBar />
      {/* props.children 用于渲染嵌套路由的内容，即App.tsx中的路由 */}
      <Transition name="page-fade">
        {props.children}
      </Transition>
    </>
  );
};

export default Layout;