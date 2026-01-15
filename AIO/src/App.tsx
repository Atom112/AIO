import { Route,Navigate } from '@solidjs/router'; 
import Layout from './Layout.tsx';
import Chat from './pages/Chat.tsx';
import Settings from './pages/Settings.tsx';

function App() {
  return (
    // 直接返回路由定义
    <Route path="/" component={Layout}>
      {/*主页自动重定向到 /chat，解决导航栏按钮激活状态问题*/}
      <Route path="/" component={() => <Navigate href="/chat" />} /> 
      <Route path="/chat" component={Chat} />
      <Route path="/settings" component={Settings} />
    </Route>
  );
}

export default App;