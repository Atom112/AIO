import { Component, For, Show, createSignal, onMount, onCleanup, createEffect } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { datas, setDatas, currentAssistantId, setCurrentAssistantId, saveSingleAssistantToBackend, deleteAssistantFile, Assistant, Topic } from '../store/store';
import Markdown from '../components/Markdown';
import { listen } from '@tauri-apps/api/event';
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
  const [isThinking, setIsThinking] = createSignal(false);
  const [typingIndex, setTypingIndex] = createSignal<number | null>(null);
  const [isChangingTopic, setIsChangingTopic] = createSignal(false);  // 主菜单状态（用于显示助手上下文菜单）
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
    // --- 1. 原有的：加载助手初始数据 ---
    try {
      const loaded = await invoke<Assistant[]>('load_assistants');
      if (Array.isArray(loaded) && loaded.length > 0) {
        setDatas({ assistants: loaded });
        if (!currentAssistantId()) setCurrentAssistantId(loaded[0].id);
      } else {
        const defaultAsst = createAssistant('默认助手');
        setDatas('assistants', [defaultAsst]);
        setCurrentAssistantId(defaultAsst.id);
        await saveSingleAssistantToBackend(defaultAsst.id);
      }
    } catch (err) {
      console.error("加载助手失败:", err);
    }

    // --- 2. 新增的：监听 Rust 侧发来的流式数据 ---
    const unlisten = await listen<any>('llm-chunk', (event) => {
      const { assistant_id, topic_id, content, done } = event.payload;

      if (done) {
        setIsThinking(false);
        setTypingIndex(null);
        saveSingleAssistantToBackend(assistant_id);
        return;
      }

      // --- 关键修改：细粒度更新 ---
      // 1. 先找到当前对话历史
      const asst = datas.assistants.find(a => a.id === assistant_id);
      const topic = asst?.topics.find(t => t.id === topic_id);
      if (!topic) return;

      const history = topic.history;
      const lastIdx = history.length - 1;

      // 2. 只有当最后一条是助手消息时才更新内容
      if (lastIdx >= 0 && history[lastIdx].role === 'assistant') {
        // 使用路径式更新：('assistants', 筛选助手, 'topics', 筛选话题, 'history', 索引, '属性', 更新函数)
        setDatas(
          'assistants', (a) => a.id === assistant_id,
          'topics', (t) => t.id === topic_id,
          'history', lastIdx,
          'content', (oldContent: string) => oldContent + content // 仅增加文字，不替换对象
        );
      }

      // 处理自动滚动
      const area = document.querySelector('.chat-messages-area');
      if (area) {
        requestAnimationFrame(() => {
          area.scrollTop = area.scrollHeight;
        });
      }

      // 如果流结束了
      if (done) {
        setIsThinking(false);
        setTypingIndex(null);
        saveSingleAssistantToBackend(assistant_id); // 持久化保存
      }
    });

    // --- 3. 原有的：全局点击监听（用于关闭菜单） ---
    const handleClickOutside = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      if (!target.closest('.assistant-context-menu') && !target.closest('.assistant-menu-button')) {
        if (showMenuDiv()) closeMenu();
        if (showTopicMenuDiv()) closeTopicMenu();
      }
    };

    document.addEventListener('click', handleClickOutside);

    // 清理函数：组件卸载时取消事件监听
    onCleanup(() => {
      unlisten();
      document.removeEventListener('click', handleClickOutside);
    });
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
    const tId = currentTopicId();

    if (asst && tId) {
      // 逻辑：开启遮罩 -> 延迟一丁点时间(或者不处理) -> 关闭遮罩
      // 虽然 Solid 更新飞快，但给浏览器一帧时间渲染 opacity: 0 能消除大部分刷屏感
      setIsChangingTopic(true);

      // 这里的 setTimeout 时间不需要太长，只需确保气泡开始加载即可
      const timer = setTimeout(() => {
        setIsChangingTopic(false);
      }, 50);

      onCleanup(() => clearTimeout(timer));
    }
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
    const topicId = currentTopicId();
    const asst = currentAssistant();
    const topic = activeTopic();

    if (!text || !asstId || !asst || !topic || isThinking()) return;

    // 1. 立即更新 UI：添加用户消息
    const newUserMsg = { role: 'user' as const, content: text };

    // 2. 核心：一次性添加用户消息和空的助理消息
    setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId, 'history', h => [
      ...h,
      newUserMsg,
      { role: 'assistant' as const, content: "" } // 预留给流式输出的容器
    ]);

    // 3. 设置状态
    setInputMessage("");
    setIsThinking(true);
    // 指向刚刚添加的那条空消息 (即最后一条，下标为 length - 1)
    const newHistory = activeTopic()?.history || [];
    setTypingIndex(newHistory.length - 1);

    if (textareaRef) textareaRef.style.height = '40px';

    try {
      const messagesForAI = [
        { role: 'system', content: asst.prompt },
        ...topic.history.map(m => ({ role: m.role, content: m.content })),
        newUserMsg
      ];

      // 4. 调用 Rust
      await invoke('call_llm_stream', {
        apiKey: "sk-KwmAR4Az6SHLAgEr19FbC79531124d449cF18b2aF35f34Ea",
        model: "gpt-4o",
        assistantId: asstId,
        topicId: topicId,
        messages: messagesForAI
      });
    } catch (err: any) {
      alert(err.toString());
      setIsThinking(false);
      setTypingIndex(null);
    }
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
        <div
          class="chat-messages-area"
          /* 使用简化的透明度切换类 */
          classList={{ 'topic-switching': isChangingTopic() }}
        >
          <Show when={activeTopic()}>
            <For each={activeTopic()?.history}>
              {(msg, index) => (
                <div
                  class={`message ${msg.role}`}
                  /* 关键：如果不是当前正在打字的那条（即历史消息），我们给个错开的延迟 */
                  /* 只有在初次渲染时会有这个阶梯效果，流式输出时 index 会很大，不影响 */
                  style={{
                    "animation-delay": `${Math.min(index() * 0.03, 0.4)}s`,
                    /* 如果是正在打字的那条，立即显示，不设置延迟 */
                    "animation-duration": typingIndex() === index() ? "0.1s" : "0.35s"
                  }}
                >
                  <div class="message-content" classList={{ 'typing': typingIndex() === index() }}>
                    <Markdown content={msg.content} />
                  </div>
                </div>
              )}
            </For>

            {/* 思考状态气泡 */}
            <Show when={isThinking()}>
              <div class="message assistant" style={{ "animation-delay": "0s" }}>
                <div class="message-content" style="opacity: 0.6">
                  AI 正在思考中...
                </div>
              </div>
            </Show>
          </Show>
        </div>
        {/* 输入区域 */}
        <div class="chat-input-wrapper">
          <textarea
            ref={textareaRef}
            class="chat-input"
            placeholder="输入消息... (Ctrl + Enter 换行)"
            value={inputMessage()}
            onInput={(e) => {
              const target = e.currentTarget;
              setInputMessage(target.value);

              // --- 自动高度逻辑 ---
              target.style.height = 'auto'; // 先重置，以便收缩

              // 计算新高度
              const newHeight = target.scrollHeight;
              target.style.height = `${newHeight}px`;
              const computedStyle = window.getComputedStyle(target);
              const maxHeight = parseFloat(computedStyle.maxHeight);
              // 逻辑判定：如果内容高度超过了 clientHeight (即被 max-height 锁定了)
              // 则显示滚动条，否则隐藏滚动条（视觉更干净）
              if (newHeight > maxHeight) {
                target.style.overflowY = 'auto';
                target.style.height = `${maxHeight}px`; // 强制锁定高度为最大值
              } else {
                target.style.overflowY = 'hidden';
              }
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                const target = e.currentTarget;

                if (e.ctrlKey) {
                  // --- 核心修改：按下 Ctrl + Enter 强制换行 ---
                  e.preventDefault(); // 阻止默认行为（防止浏览器不响应或乱跳）

                  const start = target.selectionStart;
                  const end = target.selectionEnd;
                  const value = target.value;

                  // 在光标位置手动插入换行符
                  const newValue = value.substring(0, start) + "\n" + value.substring(end);
                  setInputMessage(newValue);

                  // 使用 setTimeout 确保在 SolidJS 更新输入框内容后，把光标移到新行
                  setTimeout(() => {
                    target.selectionStart = target.selectionEnd = start + 1;
                    // 手动触发一次高度调整，确保增加行后输入框变高
                    target.dispatchEvent(new Event('input', { bubbles: true }));
                  }, 0);

                } else if (!e.shiftKey) {
                  // --- 只有 Enter (不含 Shift) 则发送消息 ---
                  e.preventDefault();
                  handleSendMessage();
                }

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