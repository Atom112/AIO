import { Component, For, Show, createSignal, onMount, onCleanup, createEffect } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import {
  datas,
  setDatas,
  currentAssistantId,
  setCurrentAssistantId,
  saveSingleAssistantToBackend,
  deleteAssistantFile,
  Assistant,
  Topic
} from '../store';
import './Chat.css';


const createTopic = (name?: string): Topic => ({
  id: Date.now().toString(),
  name: name || `新话题 ${new Date().toLocaleTimeString()}`,
  history: []
});

const createAssistant = (name?: string, id?: string): Assistant => ({
  id: id ?? Date.now().toString(),
  name: name || 'New Assistant',
  prompt: 'You are a helpful assistant.',
  topics: [createTopic('默认话题')]
});

const Chat: Component = () => {
  // --- 1. 面板宽度控制 ---
  const [leftPanelWidth, setLeftPanelWidth] = createSignal<number>(18);
  const [rightPanelWidth, setRightPanelWidth] = createSignal<number>(18);
  const [inputMessage, setInputMessage] = createSignal("");

  // --- 2. 菜单状态控制 ---
  const [showMenuDiv, setShowMenuDiv] = createSignal(false);
  const [isMenuAnimatingOut, setIsMenuAnimatingOut] = createSignal(false);
  const [menuState, setMenuState] = createSignal({ isOpen: false, x: 0, y: 0, targetId: null as string | null });
  const [currentTopicId, setCurrentTopicId] = createSignal<string | null>(null);
  let menuCloseTimeoutId: any;

  // --- 3. 拖拽逻辑变量 ---
  let chatPageRef: HTMLDivElement | undefined;
  let isResizingLeft = false;
  let isResizingRight = false;
  let initialMouseX = 0;
  let initialLeftW = 0;
  let initialRightW = 0;

  // --- 4. 逻辑：初始化 & 全局点击监听 ---
  onMount(async () => {
    // 加载数据
    try {
      const loaded = await invoke<Assistant[]>('load_assistants');
      if (Array.isArray(loaded) && loaded.length > 0) {
        setDatas({ assistants: loaded });
        if (!currentAssistantId()) setCurrentAssistantId(loaded[0].id);
      } else {
        const defaultAsst = createAssistant('默认助手');
        setDatas('assistants', [defaultAsst]);
        setCurrentAssistantId(defaultAsst.id);
        saveSingleAssistantToBackend(defaultAsst.id);
      }
    } catch (err) { console.error(err); }

    // 【修复1：点击空白处收回菜单】
    const handleClickOutside = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      // 如果点击的不是菜单内部，也不是开启菜单的那个按钮，就关闭
      if (!target.closest('.assistant-context-menu') && !target.closest('.assistant-menu-button')) {
        if (showMenuDiv()) closeMenu();
      }
    };
    document.addEventListener('click', handleClickOutside);
    onCleanup(() => document.removeEventListener('click', handleClickOutside));
  });

  // --- 5. 逻辑：面板拖拽调整大小 (完全修复) ---
  const startResize = (e: MouseEvent, type: 'left' | 'right') => {
    e.preventDefault();
    initialMouseX = e.clientX;
    const leftEl = chatPageRef?.querySelector('.assistant-selector') as HTMLElement;
    const rightEl = chatPageRef?.querySelector('.dialog-container') as HTMLElement;

    initialLeftW = leftEl?.clientWidth || 0;
    initialRightW = rightEl?.clientWidth || 0;

    if (type === 'left') isResizingLeft = true;
    else isResizingRight = true;

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', stopResize);
    document.body.style.cursor = 'ew-resize';
    document.body.style.userSelect = 'none';
  };

  const handleMouseMove = (e: MouseEvent) => {
    if (!chatPageRef) return;
    const deltaX = e.clientX - initialMouseX;
    const totalW = chatPageRef.offsetWidth;

    if (isResizingLeft) {
      const newWidth = ((initialLeftW + deltaX) / totalW) * 100;
      // 限制 10% - 40%
      setLeftPanelWidth(Math.min(Math.max(newWidth, 15), 25));
    } else if (isResizingRight) {
      const newWidth = ((initialRightW - deltaX) / totalW) * 100;
      setRightPanelWidth(Math.min(Math.max(newWidth, 15), 25));
    }
  };

  const stopResize = () => {
    isResizingLeft = false;
    isResizingRight = false;
    document.removeEventListener('mousemove', handleMouseMove);
    document.removeEventListener('mouseup', stopResize);
    document.body.style.cursor = '';
    document.body.style.userSelect = '';
  };

  // --- 6. 逻辑：操作菜单 ---
  const openMenu = (e: MouseEvent, assistantId: string) => {
    e.stopPropagation();
    if (menuState().isOpen && menuState().targetId === assistantId) { closeMenu(); return; }

    setShowMenuDiv(true);
    setIsMenuAnimatingOut(false);
    const rect = (e.currentTarget as Element).getBoundingClientRect();
    setMenuState({ isOpen: true, x: rect.left, y: rect.top + rect.height, targetId: assistantId });
  };

  const closeMenu = () => {
    setMenuState(p => ({ ...p, isOpen: false }));
    setIsMenuAnimatingOut(true);
    clearTimeout(menuCloseTimeoutId);
    menuCloseTimeoutId = setTimeout(() => {
      setShowMenuDiv(false);
      setIsMenuAnimatingOut(false);
    }, 200);
  };

  // --- 7. 逻辑：核心功能 ---
  const currentAssistant = () => datas.assistants.find(a => a.id === currentAssistantId());

  const activeTopic = () => {
    const asst = currentAssistant();
    if (!asst) return null;
    return asst.topics.find(t => t.id === currentTopicId()) || asst.topics[0];
  };

  // 监听助手切换，自动切换话题
  createEffect(() => {
    const asst = currentAssistant();
    if (asst && asst.topics.length > 0) {
      if (!currentTopicId() || !asst.topics.find(t => t.id === currentTopicId())) {
        setCurrentTopicId(asst.topics[0].id);
      }
    } else if (asst && asst.topics.length === 0) {
      // 如果选中的助手没话题，自动创建一个
      addTopic();
    }
  });

  const handleSendMessage = async () => {
    const text = inputMessage().trim();
    const asstId = currentAssistantId();
    const topicId = currentTopicId() || activeTopic()?.id;
    if (!text || !asstId || !topicId) return;

    // 定位到具体话题的 history：datas.assistants[asstId].topics[topicId].history
    setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId, 'history', h => [...h, { role: 'user' as const, content: text }]);
    setInputMessage("");

    // 模拟回复
    setTimeout(async () => {
      setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId, 'history', h => [...h, { role: 'assistant' as const, content: "收到。" }]);
      await saveSingleAssistantToBackend(asstId);
    }, 500);
  };

  const addTopic = async () => {
    const asstId = currentAssistantId();
    if (!asstId) return;
    const newT = createTopic();
    setDatas('assistants', a => a.id === asstId, 'topics', prev => [...prev, newT]);
    setCurrentTopicId(newT.id);
    await saveSingleAssistantToBackend(asstId);
  };

  const addAssistant = async () => {
    const newAsst = createAssistant(`新助手 ${datas.assistants.length + 1}`);
    setDatas('assistants', (prev) => [...prev, newAsst]);
    setCurrentAssistantId(newAsst.id);
    await saveSingleAssistantToBackend(newAsst.id);
  };

  const removeAssistant = async (id: string | null) => {
    if (!id) return;
    await deleteAssistantFile(id);
    if (currentAssistantId() === id) {
      const idx = datas.assistants.findIndex(a => a.id === id);
      setCurrentAssistantId(datas.assistants[idx - 1]?.id || datas.assistants[idx + 1]?.id || null);
    }
    setDatas('assistants', prev => prev.filter(a => a.id !== id));
    closeMenu();
  };

  return (
    <div class="chat-page" ref={el => chatPageRef = el}>

      {/* 左侧面板 */}
      <div class="assistant-selector" style={{ width: `${leftPanelWidth()}%` }}>
        <div class="assistant-content">
          <For each={datas.assistants}>
            {(assistant) => (
              <div
                classList={{ 'assistant-item': true, 'active': assistant.id === currentAssistantId() }}
                onClick={() => setCurrentAssistantId(assistant.id)}
              >
                <span>{assistant.name}</span>
                <button class="assistant-menu-button" onClick={(e) => openMenu(e as MouseEvent, assistant.id)}>
                  <svg xmlns="http://www.w3.org/2000/svg" fill="#FFFFFF" viewBox="0 0 24 24" stroke-Width={1.5} class="size-6"><path stroke-Linecap="round" stroke-Linejoin="round" d="M10 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0-6a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0z" /></svg>
                </button>
              </div>
            )}
          </For>
          <button class="add-assistant-button" onClick={addAssistant}>+ 新增助手</button>
        </div>
        {/* 修复：左侧拖拽手柄 */}
        <div class="resize-handle left-handle" onMouseDown={(e) => startResize(e as MouseEvent, 'left')}></div>
      </div>

      {/* 中间聊天区域 */}
      <div class="chat-input-container">
        <div class="chat-messages-area">
          <Show when={activeTopic()} fallback={<div class="empty-state">请选择或创建话题</div>}>
            <For each={activeTopic()?.history}>
              {(msg) => (
                <div class={`message ${msg.role}`}>
                  <div class="message-content">{msg.content}</div>
                </div>
              )}
            </For>
          </Show>
        </div>

        <div class="chat-input-wrapper">
          <input
            type="text"
            class="chat-input"
            placeholder="输入消息..."
            value={inputMessage()}
            onInput={(e) => setInputMessage(e.currentTarget.value)}
            onKeyDown={(e) => e.key === 'Enter' && handleSendMessage()}
          />
          <button class="send-message-button" onClick={() => handleSendMessage()}>
            <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="size-6">
              <path stroke-linecap="round" stroke-linejoin="round" d="M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.27 20.876L5.999 12zm0 0h7.5" />
            </svg>
            <span>发送</span>
          </button>
        </div>
      </div>

      {/* 右侧展示区域 */}
      <div class="dialog-container" style={{ width: `${rightPanelWidth()}%` }}>
        <div class="resize-handle right-handle" onMouseDown={(e) => startResize(e as MouseEvent, 'right')}></div>
        <div class="dialog-content">
          <Show when={currentAssistant()}>
            {(asst) => (
              <>
                <div class="info-header" style="border-bottom: 1px solid #08ddf9; padding-bottom: 10px; margin-bottom: 15px;">
                  <h3>{asst().name} 的话题</h3>
                </div>

                <button class="add-topic-button" onClick={addTopic}>+ 新建话题</button>

                <div class="topics-list">
                  <For each={asst().topics}>
                    {(topic) => (
                      <div
                        classList={{ 'topic-item': true, 'active': topic.id === currentTopicId() }}
                        onClick={() => setCurrentTopicId(topic.id)}
                      >
                        <span class="topic-name">{topic.name}</span>
                        {/* 这里你可以根据需要也加一个删除话题的小图标 */}
                      </div>
                    )}
                  </For>
                </div>
              </>
            )}
          </Show>
        </div>
      </div>

      {/* 下拉菜单 */}
      {showMenuDiv() && (
        <div
          class="assistant-context-menu"
          classList={{ 'menu-exiting': isMenuAnimatingOut() }}
          style={{ top: `${menuState().y}px`, left: `${menuState().x}px` }}
        >
          <button class="context-menu-button" onClick={() => alert('Developing...')}>重命名</button>
          <button class="context-menu-button delete" onClick={() => removeAssistant(menuState().targetId)}>删除助手</button>
        </div>
      )}
    </div>
  );
};

export default Chat;