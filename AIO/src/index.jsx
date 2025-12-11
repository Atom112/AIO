import { render } from 'solid-js/web';
import { Router } from '@solidjs/router';
import App from './App.jsx';
import './index.css';

render(
  () => (
    // 包裹 Router 组件以启用路由功能
    <Router>
      <App />
    </Router>
  ),
  document.getElementById('root')
);