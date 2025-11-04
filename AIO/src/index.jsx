/* @refresh reload */
// src/index.jsx
import { render } from 'solid-js/web';
// [!code focus]
import { Router } from '@solidjs/router'; // <-- 已修改
import App from './App.jsx';
import './index.css';
render(
  () => (
    
    <Router>
      <App />
    </Router>
  ),
  document.getElementById('root')
);