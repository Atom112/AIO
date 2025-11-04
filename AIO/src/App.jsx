// src/App.jsx
import { Route } from '@solidjs/router'; // <-- 移除了 Router 的导入
import Layout from './Layout.jsx';
import Chat from './pages/Chat.jsx';
import Settings from './pages/Settings.jsx';

function App() {
  return (
    // 直接返回路由定义
    <Route path="/" component={Layout}>
      <Route path="/" component={Chat} />
      <Route path="/settings" component={Settings} />
    </Route>
  );
}

export default App;