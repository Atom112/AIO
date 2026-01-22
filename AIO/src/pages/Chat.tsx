import { Component, For, Show, createSignal, onMount, onCleanup, createEffect } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { datas, setDatas, currentAssistantId, setCurrentAssistantId, saveSingleAssistantToBackend, deleteAssistantFile, Assistant, Topic } from '../store/store';
import Markdown from '../components/Markdown';
import './Chat.css';

// ======================
// 1. 常量定义
// ======================
/**
 * 创建新话题的辅助函数
 * @param name - 话题名称（可选）
 * @returns 新话题对象
 */
const createTopic = (name?: string): Topic => ({
  id: Date.now().toString(), // 使用时间戳生成唯一ID
  name: name || `新话题 ${new Date().toLocaleTimeString()}`, // 默认名称
  history: [] // 话题对话历史
});

/**
 * 创建新助手的辅助函数
 * @param name - 助手名称（可选）
 * @param id - 助手ID（可选，用于恢复）
 * @returns 新助手对象
 */
const createAssistant = (name?: string, id?: string): Assistant => ({
  id: id ?? Date.now().toString(), // 生成唯一ID
  name: name || 'New Assistant', // 默认名称
  prompt: 'You are a helpful assistant.', // 系统提示词
  topics: [createTopic('默认话题')] // 初始话题
});

// ======================
// 2. 状态定义
// ======================
const Chat: Component = () => {
  // 左右面板宽度（百分比）
  const [leftPanelWidth, setLeftPanelWidth] = createSignal<number>(18);
  const [rightPanelWidth, setRightPanelWidth] = createSignal<number>(18);

  // 输入消息内容
  const [inputMessage, setInputMessage] = createSignal("");

  // 正在重命名的助理ID（用于输入框聚焦）
  const [editingAsstId, setEditingAsstId] = createSignal<string | null>(null);

  // 话题菜单状态（用于显示上下文菜单）
  const [topicMenuState, setTopicMenuState] = createSignal({ isOpen: false, x: 0, y: 0, targetTopicId: null as string | null });
  const [showTopicMenuDiv, setShowTopicMenuDiv] = createSignal(false);
  const [isTopicMenuAnimatingOut, setIsTopicMenuAnimatingOut] = createSignal(false);

  // 主菜单状态（用于显示助手上下文菜单）
  const [showMenuDiv, setShowMenuDiv] = createSignal(false);
  const [isMenuAnimatingOut, setIsMenuAnimatingOut] = createSignal(false);
  const [menuState, setMenuState] = createSignal({ isOpen: false, x: 0, y: 0, targetId: null as string | null });

  // 当前激活的话题ID
  const [currentTopicId, setCurrentTopicId] = createSignal<string | null>(null);

  // 拖拽相关变量（用于面板调整）
  let menuCloseTimeoutId: any;
  let chatPageRef: HTMLDivElement | undefined;
  let isResizingLeft = false;
  let isResizingRight = false;
  let initialMouseX = 0;
  let initialLeftW = 0;
  let initialRightW = 0;
  let textareaRef: HTMLTextAreaElement | undefined;
  // ======================
  // 3. 函数定义
  // ======================

  /**
   * 保存重命名的助手名称
   * @param id - 助手ID
   * @param newName - 新名称
   */
  const saveRename = async (id: string, newName: string) => {
    // 验证名称不为空
    if (!newName.trim()) return setEditingAsstId(null);

    // 更新本地状态
    setDatas('assistants', a => a.id === id, 'name', newName);

    // 保存到后端
    await saveSingleAssistantToBackend(id);

    // 退出重命名模式
    setEditingAsstId(null);
  };

  /**
   * 打开话题上下文菜单
   * @param e - 鼠标事件
   * @param topicId - 话题ID
   */
  const openTopicMenu = (e: MouseEvent, topicId: string) => {
    e.stopPropagation(); // 阻止事件冒泡
    setShowTopicMenuDiv(true);
    setIsTopicMenuAnimatingOut(false);

    // 获取点击元素位置
    const rect = (e.currentTarget as Element).getBoundingClientRect();
    const menuWidth = 150;
    let x = rect.left;

    // 确保菜单不超出屏幕
    if (x + menuWidth > window.innerWidth) {
      x = rect.right - menuWidth;
    }

    // 设置菜单位置
    setTopicMenuState({ isOpen: true, x: x, y: rect.top + rect.height, targetTopicId: topicId });
  };

  /**
   * 关闭话题上下文菜单
   */
  const closeTopicMenu = () => {
    setTopicMenuState(p => ({ ...p, isOpen: false }));
    setIsTopicMenuAnimatingOut(true);

    // 动画结束后隐藏菜单
    setTimeout(() => {
      setShowTopicMenuDiv(false);
      setIsTopicMenuAnimatingOut(false);
    }, 200);
  };

  /**
   * 删除话题
   * @param asstId - 助手ID
   * @param topicId - 话题ID
   */
  const deleteTopic = async (asstId: string | null, topicId: string | null) => {
    if (!asstId || !topicId) return;

    const asst = datas.assistants.find(a => a.id === asstId);
    if (!asst) return;

    // 确保每个助手至少保留一个话题
    if (asst.topics.length <= 1) {
      alert("每个助手至少保留一个话题");
      closeTopicMenu();
      return;
    }

    // 更新本地状态
    setDatas('assistants', a => a.id === asstId, 'topics', topics => topics.filter(t => t.id !== topicId));

    // 如果删除的是当前话题，切换到第一个话题
    if (currentTopicId() === topicId) {
      setCurrentTopicId(asst.topics[0].id);
    }

    // 保存到后端
    await saveSingleAssistantToBackend(asstId);
    closeTopicMenu();
  };

  // 初始化应用状态
  onMount(async () => {
    try {
      // 从后端加载助手数据
      const loaded = await invoke<Assistant[]>('load_assistants');

      if (Array.isArray(loaded) && loaded.length > 0) {
        // 设置本地状态
        setDatas({ assistants: loaded });

        // 如果没有当前助手，设置第一个助手
        if (!currentAssistantId()) setCurrentAssistantId(loaded[0].id);
      } else {
        // 创建默认助手
        const defaultAsst = createAssistant('默认助手');
        setDatas('assistants', [defaultAsst]);
        setCurrentAssistantId(defaultAsst.id);

        // 保存到后端
        await saveSingleAssistantToBackend(defaultAsst.id);
      }
    } catch (err) {
      console.error("加载助手失败:", err);
    }

    // 处理点击外部关闭菜单
    const handleClickOutside = (e: MouseEvent) => {
      const target = e.target as HTMLElement;

      // 如果点击不在菜单或按钮上，关闭菜单
      if (!target.closest('.assistant-context-menu') && !target.closest('.assistant-menu-button')) {
        if (showMenuDiv()) closeMenu();
        if (showTopicMenuDiv()) closeTopicMenu();
      }
    };

    document.addEventListener('click', handleClickOutside);
    onCleanup(() => document.removeEventListener('click', handleClickOutside));
  });

  /**
   * 开始调整面板大小
   * @param e - 鼠标事件
   * @param type - 调整的面板（left/right）
   */
  const startResize = (e: MouseEvent, type: 'left' | 'right') => {
    e.preventDefault(); // 阻止默认行为

    // 获取初始位置和宽度
    initialMouseX = e.clientX;
    const leftEl = chatPageRef?.querySelector('.assistant-selector') as HTMLElement;
    const rightEl = chatPageRef?.querySelector('.dialog-container') as HTMLElement;
    initialLeftW = leftEl?.clientWidth || 0;
    initialRightW = rightEl?.clientWidth || 0;

    // 设置调整状态
    if (type === 'left') isResizingLeft = true;
    else isResizingRight = true;

    // 添加拖拽事件监听
    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', stopResize);

    // 设置鼠标样式
    document.body.style.cursor = 'ew-resize';
    document.body.style.userSelect = 'none';
  };

  /**
   * 处理拖拽移动
   * @param e - 鼠标事件
   */
  const handleMouseMove = (e: MouseEvent) => {
    if (!chatPageRef) return;

    const deltaX = e.clientX - initialMouseX;
    const totalW = chatPageRef.offsetWidth;

    if (isResizingLeft) {
      // 计算新的左侧面板宽度
      const newWidth = ((initialLeftW + deltaX) / totalW) * 100;
      setLeftPanelWidth(Math.min(Math.max(newWidth, 15), 25)); // 限制在15%-25%
    } else if (isResizingRight) {
      // 计算新的右侧面板宽度
      const newWidth = ((initialRightW - deltaX) / totalW) * 100;
      setRightPanelWidth(Math.min(Math.max(newWidth, 15), 25)); // 限制在15%-25%
    }
  };

  /**
   * 停止调整面板大小
   */
  const stopResize = () => {
    isResizingLeft = false;
    isResizingRight = false;

    // 移除拖拽事件监听
    document.removeEventListener('mousemove', handleMouseMove);
    document.removeEventListener('mouseup', stopResize);

    // 恢复鼠标样式
    document.body.style.cursor = '';
    document.body.style.userSelect = '';
  };

  /**
   * 打开助手上下文菜单
   * @param e - 鼠标事件
   * @param assistantId - 助手ID
   */
  const openMenu = (e: MouseEvent, assistantId: string) => {
    e.stopPropagation(); // 阻止事件冒泡

    // 如果菜单已打开且是同一个助手，关闭菜单
    if (menuState().isOpen && menuState().targetId === assistantId) {
      closeMenu();
      return;
    }

    setShowMenuDiv(true);
    setIsMenuAnimatingOut(false);

    // 获取点击位置
    const rect = (e.currentTarget as Element).getBoundingClientRect();
    setMenuState({ isOpen: true, x: rect.left, y: rect.top + rect.height, targetId: assistantId });
  };

  /**
   * 关闭助手上下文菜单
   */
  const closeMenu = () => {
    setMenuState(p => ({ ...p, isOpen: false }));
    setIsMenuAnimatingOut(true);

    // 清除之前的关闭定时器
    clearTimeout(menuCloseTimeoutId);

    // 动画结束后隐藏菜单
    menuCloseTimeoutId = setTimeout(() => {
      setShowMenuDiv(false);
      setIsMenuAnimatingOut(false);
    }, 200);
  };

  /**
   * 获取当前激活的助手
   * @returns 当前助手对象
   */
  const currentAssistant = () => datas.assistants.find(a => a.id === currentAssistantId());

  /**
   * 获取当前激活的话题
   * @returns 当前话题对象
   */
  const activeTopic = () => {
    const asst = currentAssistant();
    if (!asst) return null;

    // 如果当前话题ID有效，返回对应话题
    // 否则返回第一个话题
    return asst.topics.find(t => t.id === currentTopicId()) || asst.topics[0];
  };

  // 监听助手切换，自动切换话题
  createEffect(() => {
    const asst = currentAssistant();
    if (asst && asst.topics.length > 0) {
      // 如果当前话题不存在，切换到第一个话题
      if (!currentTopicId() || !asst.topics.find(t => t.id === currentTopicId())) {
        setCurrentTopicId(asst.topics[0].id);
      }
    } else if (asst && asst.topics.length === 0) {
      // 如果没有话题，创建新话题
      addTopic();
    }
  });

  /**
   * 发送消息到聊天
   */
  const handleSendMessage = async () => {
    const text = inputMessage().trim();
    const asstId = currentAssistantId();
    const topicId = currentTopicId() || activeTopic()?.id;

    // 验证输入
    if (!text || !asstId || !topicId) return;

    // 添加用户消息到本地状态
    setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId, 'history', h => [...h, { role: 'user' as const, content: text }]);

    // 清空输入框
    setInputMessage("");
    if (textareaRef) {
      textareaRef.style.height = '40px';      // 恢复初始高度
      textareaRef.style.overflowY = 'hidden'; // 发送后隐藏滚动条
    }

    // 模拟AI响应（实际应用中应调用API）
    setTimeout(async () => {
      setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId, 'history', h => [...h, { role: 'assistant' as const, content: "收到。" }]);
      await saveSingleAssistantToBackend(asstId);
    }, 500);
  };

  /**
   * 添加新话题
   */
  const addTopic = async () => {
    const asstId = currentAssistantId();
    if (!asstId) return;

    // 创建新话题
    const newT = createTopic();

    // 添加到本地状态
    setDatas('assistants', a => a.id === asstId, 'topics', prev => [...prev, newT]);

    // 设置为当前话题
    setCurrentTopicId(newT.id);

    // 保存到后端
    await saveSingleAssistantToBackend(asstId);
  };

  /**
   * 添加新助手
   */
  const addAssistant = async () => {
    // 创建新助手
    const newAsst = createAssistant(`新助手 ${datas.assistants.length + 1}`);

    // 添加到本地状态
    setDatas('assistants', (prev) => [...prev, newAsst]);

    // 设置为当前助手
    setCurrentAssistantId(newAsst.id);

    // 保存到后端
    await saveSingleAssistantToBackend(newAsst.id);
  };

  /**
   * 删除助手
   * @param id - 助手ID
   */
  const removeAssistant = async (id: string | null) => {
    if (!id) return;

    // 从后端删除
    await deleteAssistantFile(id);

    // 如果删除的是当前助手，切换到其他助手
    if (currentAssistantId() === id) {
      const idx = datas.assistants.findIndex(a => a.id === id);
      setCurrentAssistantId(datas.assistants[idx - 1]?.id || datas.assistants[idx + 1]?.id || null);
    }

    // 从本地状态移除
    setDatas('assistants', prev => prev.filter(a => a.id !== id));

    // 关闭菜单
    closeMenu();
  };

  // ======================
  // 4. 组件主体
  // ======================
  return (
    <div class="chat-page" ref={el => chatPageRef = el}>
      {/* 左侧面板 - 助手列表 */}
      <div class="assistant-selector" style={{ width: `${leftPanelWidth()}%` }}>
        <div class="assistant-content">
          <For each={datas.assistants}>
            {(assistant) => (
              <div classList={{ 'assistant-item': true, 'active': assistant.id === currentAssistantId() }} onClick={() => setCurrentAssistantId(assistant.id)}>
                {/* 显示助手名称，或输入框用于重命名 */}
                <Show when={editingAsstId() === assistant.id} fallback={<span class="assistant-name">{assistant.name}</span>}>
                  <input class="rename-input" value={assistant.name} ref={(el) => {
                    // 确保输入框聚焦并选中文本
                    setTimeout(() => {
                      el.focus();
                      el.select();
                    }, 0);
                  }} onBlur={(e) => saveRename(assistant.id, e.currentTarget.value)} onKeyDown={(e) => e.key === 'Enter' && saveRename(assistant.id, e.currentTarget.value)} onClick={(e) => e.stopPropagation()} />
                </Show>
                {/* 助手菜单按钮 */}
                <button class="assistant-menu-button" onClick={(e) => openMenu(e as MouseEvent, assistant.id)}>
                  <svg xmlns="http://www.w3.org/2000/svg" fill="#FFFFFF" viewBox="0 0 24 24" stroke-width={1.5} class="size-6">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M10 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0-6a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0z" />
                  </svg>
                </button>
              </div>
            )}
          </For>
          {/* 添加新助手按钮 */}
          <button class="add-assistant-button" onClick={addAssistant}>+ 新增助手</button>
        </div>
        {/* 左侧调整手柄 */}
        <div class="resize-handle left-handle" onMouseDown={(e) => startResize(e as MouseEvent, 'left')}></div>
      </div>

      {/* 中间聊天区域 */}
      <div class="chat-input-container">
        <div class="chat-messages-area">
          {/* 显示对话历史或空状态 */}
          <Show when={activeTopic()} fallback={<div class="empty-state">请选择或创建话题</div>}>
            <For each={activeTopic()?.history}>
              {(msg) => (
                <div class={`message ${msg.role}`}>
                  <div class="message-content">
                    <Markdown content={msg.content} />
                  </div>
                </div>
              )}
            </For>
          </Show>
        </div>
        {/* 输入区域 */}
        <div class="chat-input-wrapper">
          <textarea
            ref={textareaRef}
            class="chat-input"
            placeholder="输入消息... (Shift + Enter 换行)"
            value={inputMessage()}
            onInput={(e) => {
              const target = e.currentTarget;
              setInputMessage(target.value);

              // --- 自动高度逻辑 ---
              target.style.height = 'auto'; // 先重置，以便收缩

              // 计算新高度
              const newHeight = target.scrollHeight;
              target.style.height = `${newHeight}px`;

              // 逻辑判定：如果内容高度超过了 clientHeight (即被 max-height 锁定了)
              // 则显示滚动条，否则隐藏滚动条（视觉更干净）
              if (newHeight > target.clientHeight) {
                target.style.overflowY = 'auto';
              } else {
                target.style.overflowY = 'hidden';
              }
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                handleSendMessage();
              }
            }}
            rows={1}
          />
          <button class="send-message-button" onClick={() => handleSendMessage()}>
            {/* SVG 图标保持不变 */}
            <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="size-6">
              <path stroke-linecap="round" stroke-linejoin="round" d="M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.27 20.876L5.999 12zm0 0h7.5" />
            </svg>
            <span>发送</span>
          </button>
        </div>
      </div>



      {/* 右侧展示区域 - 话题列表 */}
      <div class="dialog-container" style={{ width: `${rightPanelWidth()}%` }}>
        {/* 右侧调整手柄 */}
        <div class="resize-handle right-handle" onMouseDown={(e) => startResize(e as MouseEvent, 'right')}></div>
        <div class="dialog-content">
          {/* 显示当前助手的话题 */}
          <Show when={currentAssistant()}>
            {(asst) => (
              <>
                <div class="info-header" style="border-bottom: 1px solid #08ddf9; padding-bottom: 10px; margin-bottom: 15px;">
                  <h3>{asst().name} 的话题</h3>
                </div>
                {/* 添加新话题按钮 */}
                <button class="add-topic-button" onClick={addTopic}>+ 新建话题</button>
                <div class="topics-list">
                  <For each={asst().topics}>
                    {(topic) => (
                      <div classList={{ 'topic-item': true, 'active': topic.id === currentTopicId() }} onClick={() => setCurrentTopicId(topic.id)}>
                        <span class="topic-name">{topic.name}</span>
                        {/* 话题菜单按钮 */}
                        <button class="assistant-menu-button" style="width: 24px; height: 24px;" onClick={(e) => openTopicMenu(e as MouseEvent, topic.id)}>
                          <svg xmlns="http://www.w3.org/2000/svg" fill="#FFFFFF" viewBox="0 0 24 24" stroke-width={1.5} style="width: 18px; height: 18px;">
                            <path stroke-linecap="round" stroke-linejoin="round" d="M10 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0-6a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0z" />
                          </svg>
                        </button>
                      </div>
                    )}
                  </For>
                </div>
              </>
            )}
          </Show>
        </div>
      </div>

      {/* 助手上下文菜单 */}
      {showMenuDiv() && (
        <div class="assistant-context-menu" classList={{ 'menu-exiting': isMenuAnimatingOut() }} style={{ top: `${menuState().y}px`, left: `${menuState().x}px` }}>
          <button class="context-menu-button" onClick={() => {
            setEditingAsstId(menuState().targetId);
            closeMenu();
          }}>重命名</button>
          <button class="context-menu-button delete" onClick={() => removeAssistant(menuState().targetId)}>删除助手</button>
        </div>
      )}

      {/* 话题上下文菜单 */}
      {showTopicMenuDiv() && (
        <div class="assistant-context-menu" classList={{ 'menu-exiting': isTopicMenuAnimatingOut() }} style={{ top: `${topicMenuState().y}px`, left: `${topicMenuState().x}px` }}>
          <button class="context-menu-button delete" onClick={() => deleteTopic(currentAssistantId(), topicMenuState().targetTopicId)}>删除话题</button>
        </div>
      )}
    </div>
  );
};

export default Chat;