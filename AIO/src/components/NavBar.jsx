// src/components/NavBar.jsx

import { createSignal, For } from 'solid-js';
import { A } from '@solidjs/router';
import './NavBar.css';
import PromptModal from '../pages/PromptModal'; 

function NavBar() {
  const [isModalOpen, setIsModalOpen] = createSignal(false);
  const [currentPrompt, setCurrentPrompt] = createSignal(
    'You are a helpful AI assistant. Please answer questions accurately and concisely.'
  );

  const allModels = ['GPT-4', 'Claude 3', 'Gemini Pro', 'Llama 3'];
  const [selectedModel, setSelectedModel] = createSignal(allModels[0]);
  const [isDropdownVisible, setDropdownVisible] = createSignal(false);
  
  // --- 新增：用于存储 setTimeout 的 ID ---
  let hideTimeoutId;

  // --- 新增：处理鼠标进入的逻辑 ---
  const handleMouseEnter = () => {
    // 如果有正在计划中的“隐藏”任务，则取消它
    clearTimeout(hideTimeoutId);
    // 立即显示下拉菜单
    setDropdownVisible(true);
  };

  // --- 新增：处理鼠标离开的逻辑 ---
  const handleMouseLeave = () => {
    // 计划在 200 毫秒后隐藏下拉菜单
    hideTimeoutId = setTimeout(() => {
      setDropdownVisible(false);
    }, 200); // 这里的 200 就是 0.2 秒
  };

  const handleSavePrompt = (newPrompt) => {
    setCurrentPrompt(newPrompt);
    console.log("新的提示词已保存:", newPrompt); 
  };
  
  const handleModelSelect = (model) => {
    setSelectedModel(model);
    setDropdownVisible(false); // 点击后立即隐藏
    // 同样，取消任何可能正在等待的隐藏任务
    clearTimeout(hideTimeoutId);
    console.log("模型已切换为:", model);
  };
  
  return (
    <>
      <nav class="navbar">
        {/* ... 左侧和中心部分不变 ... */}
        <A href="/" class="nav-item" activeClass="active">对话</A>
        <A href="/settings" class="nav-item" activeClass="active">设置</A>
        <img src="/path-to-your-avatar.png" alt="User Avatar" class="avatar"/>

        {/* --- 修改这里的事件绑定 --- */}
        <div 
          class="model-selector-wrapper"
          onMouseEnter={handleMouseEnter}  /* <-- 使用新的处理函数 */
          onMouseLeave={handleMouseLeave}  /* <-- 使用新的处理函数 */
        >
          <div class="nav-item model-selector">
            <span>模型：{selectedModel()}</span>
          </div>

          <div classList={{ 'dropdown-menu': true, 'active': isDropdownVisible() }}>
            <For each={allModels}>
              {(model) => (
                <div 
                  class="dropdown-item"
                  onClick={() => handleModelSelect(model)}
                >
                  {model}
                </div>
              )}
            </For>
          </div>
        </div>

        <a
          href="#" 
          class="nav-item" 
          onClick={(e) => {
            e.preventDefault();
            setIsModalOpen(true);
          }}
        >
          设置提示词
        </a>
      </nav>

      <PromptModal 
        show={isModalOpen()} 
        onClose={() => setIsModalOpen(false)}
        onSave={handleSavePrompt}
        initialPrompt={currentPrompt()}
      />
    </>
  );
}

export default NavBar;