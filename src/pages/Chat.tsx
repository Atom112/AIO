import { Component, For, Show, createSignal, onMount, onCleanup, createEffect } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { loadAvatarFromPath, setGlobalUserAvatar, globalUserAvatar, Message, config, datas, setDatas, currentAssistantId, setCurrentAssistantId, saveSingleAssistantToBackend, deleteAssistantFile, Assistant, Topic, selectedModel } from '../store/store';
import Markdown from '../components/Markdown';
import { listen } from '@tauri-apps/api/event';
import './Chat.css';

// =============================================================================
// I. 辅助函数与常量定义
// =============================================================================

/**
 * 创建新话题的结构对象
 * @param name - 话题名称，默认带时间戳
 * @returns 初始化的 Topic 对象
 */
const createTopic = (name?: string): Topic => ({
  id: Date.now().toString(),
  name: name || `新话题 ${new Date().toLocaleTimeString()}`,
  history: []
});

/**
 * 创建新助手的结构对象
 * @param name - 助手展示名称
 * @param id - 指定 ID，不指定则生成时间戳 ID
 * @returns 初始化的 Assistant 对象
 */
const createAssistant = (name?: string, id?: string): Assistant => ({
  id: id ?? Date.now().toString(),
  name: name || 'New Assistant',
  prompt: 'You are a helpful assistant.',
  topics: [createTopic('默认话题')]
});

/**
 * 聊天主组件：负责处理 AI 对话、文件拖拽、面板调整及话题管理
 */
const Chat: Component = () => {
  // =============================================================================
  // II. 状态定义 (Signals)
  // =============================================================================

  // 1. 界面布局状态
  const [leftPanelWidth, setLeftPanelWidth] = createSignal<number>(18);   // 左侧助手列表宽度 %
  const [rightPanelWidth, setRightPanelWidth] = createSignal<number>(18); // 右侧话题列表宽度 %
  const [isDragging, setIsDragging] = createSignal(false);               // 全局拖拽文件状态

  // 2. 交互与输入状态
  const [inputMessage, setInputMessage] = createSignal("");               // 当前输入框文本
  const [pendingFiles, setPendingFiles] = createSignal<{ name: string, content: string }[]>([]); // 待发送的文件附件
  const [isProcessing, setIsProcessing] = createSignal(false);            // 正在处理解析文件状态
  const [isThinking, setIsThinking] = createSignal(false);                // AI 是否正在生成回答
  const [typingIndex, setTypingIndex] = createSignal<number | null>(null); // 当前正在“打字”的消息索引

  // 3. 话题与助手编辑状态
  const [editingAsstId, setEditingAsstId] = createSignal<string | null>(null);    // 正在重命名的助手 ID
  const [editingTopicId, setEditingTopicId] = createSignal<string | null>(null);  // 正在重命名的话题 ID
  const [currentTopicId, setCurrentTopicId] = createSignal<string | null>(null);  // 当前选中的话题 ID
  const [isChangingTopic, setIsChangingTopic] = createSignal(false);              // 切换话题时的视觉过渡状态

  // 4. 菜单状态 (Context Menu)
  const [showMenuDiv, setShowMenuDiv] = createSignal(false);              // 是否在 DOM 中创建助手菜单
  const [isMenuAnimatingOut, setIsMenuAnimatingOut] = createSignal(false); // 助手菜单退出动画控制
  const [menuState, setMenuState] = createSignal({ isOpen: false, x: 0, y: 0, targetId: null as string | null });

  const [showTopicMenuDiv, setShowTopicMenuDiv] = createSignal(false);              // 是否创建话题菜单
  const [isTopicMenuAnimatingOut, setIsTopicMenuAnimatingOut] = createSignal(false); // 话题菜单退出动画
  const [topicMenuState, setTopicMenuState] = createSignal({ isOpen: false, x: 0, y: 0, targetTopicId: null as string | null });

  // 5. 引用与临时变量
  let menuCloseTimeoutId: any;
  let chatPageRef: HTMLDivElement | undefined;
  let textareaRef: HTMLTextAreaElement | undefined;
  let isResizingLeft = false;
  let isResizingRight = false;
  let initialMouseX = 0;
  let initialLeftW = 0;
  let initialRightW = 0;


  const getModelLogo = (modelName: string) => {
    const name = modelName.toLowerCase();
    if (name.includes('gpt')) return '/icons/openai.svg';
    if (name.includes('claude')) return '/icons/claude-color.svg';
    if (name.includes('grok')) return '/icons/grok.svg';
    if (name.includes('gemini')) return '/icons/gemini-color.svg';
    if (name.includes('deepseek')) return '/icons/deepseek-color.svg';
    if (name.includes('qwen') || name.includes('qwq')) return '/icons/qwen-color.svg';

    // 默认或本地模型的图标
    return '/icons/ollama.svg';
  };
  // =============================================================================
  // III. 业务逻辑函数
  // =============================================================================

  /**
   * 保存话题重命名结果
   * @param asstId 助手ID
   * @param topicId 话题ID
   * @param newName 新名称
   */
  const saveTopicRename = async (asstId: string, topicId: string, newName: string) => {
    if (!newName.trim()) return setEditingTopicId(null);
    setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId, 'name', newName);
    await saveSingleAssistantToBackend(asstId);
    setEditingTopicId(null);
  };

  /**
   * 自动话题总结：根据首轮对话生成标题
   */
  const summarizeTopic = async (asstId: string, topicId: string, userMsg: string, aiMsg: string) => {
    const currentMdl = selectedModel();
    if (!currentMdl) return;

    const prompt = `请简要总结以下对话的主题，作为一个简短的标题（不超过10个字）。直接返回标题，不要包含任何标点或多余文字。\n用户：${userMsg}\n助手：${aiMsg}`;

    try {
      const response = await fetch(currentMdl.api_url.replace(/\/+$/, "") + (currentMdl.api_url.endsWith("/chat/completions") ? "" : "/chat/completions"), {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "Authorization": `Bearer ${currentMdl.api_key}`
        },
        body: JSON.stringify({
          model: currentMdl.model_id,
          messages: [{ role: "user", content: prompt }],
          stream: false
        })
      });

      const data = await response.json();
      const summary = data.choices?.[0]?.message?.content?.trim();
      if (summary) {
        setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId, 'name', summary);
        saveSingleAssistantToBackend(asstId);
      }
    } catch (err) {
      console.error("话题总结请求失败:", err);
    }
  };

  /**
   * 保存助手重命名结果
   */
  const saveRename = async (id: string, newName: string) => {
    if (!newName.trim()) return setEditingAsstId(null);
    setDatas('assistants', a => a.id === id, 'name', newName);
    await saveSingleAssistantToBackend(id);
    setEditingAsstId(null);
  };

  /**
   * 话题右键/更多菜单控制
   */
  const openTopicMenu = (e: MouseEvent, topicId: string) => {
    e.stopPropagation();
    setShowTopicMenuDiv(true);
    setIsTopicMenuAnimatingOut(false);
    const rect = (e.currentTarget as Element).getBoundingClientRect();
    const menuWidth = 150;
    let x = rect.left;
    if (x + menuWidth > window.innerWidth) x = rect.right - menuWidth;
    setTopicMenuState({ isOpen: true, x: x, y: rect.top + rect.height, targetTopicId: topicId });
  };

  const closeTopicMenu = () => {
    setTopicMenuState(p => ({ ...p, isOpen: false }));
    setIsTopicMenuAnimatingOut(true);
    setTimeout(() => {
      setShowTopicMenuDiv(false);
      setIsTopicMenuAnimatingOut(false);
    }, 200);
  };

  /**
   * 删除话题及其处理
   */
  const deleteTopic = async (asstId: string | null, topicId: string | null) => {
    if (!asstId || !topicId) return;
    const asst = datas.assistants.find(a => a.id === asstId);
    if (!asst) return;
    if (asst.topics.length <= 1) {
      alert("每个助手至少保留一个话题");
      closeTopicMenu();
      return;
    }
    setDatas('assistants', (a: Assistant) => a.id === asstId, 'topics', (topics: Topic[]) => topics.filter((t: Topic) => t.id !== topicId));
    if (currentTopicId() === topicId) setCurrentTopicId(asst.topics[0].id);
    await saveSingleAssistantToBackend(asstId);
    closeTopicMenu();
  };

  // =============================================================================
  // IV. 生命周期与事件监听 (Tauri Event Listeners)
  // =============================================================================

  onMount(() => {
    let unlistenLLM: (() => void) | undefined;
    let unlistenDrop: (() => void) | undefined;
    let unlistenDragEnter: (() => void) | undefined;
    let unlistenDragLeave: (() => void) | undefined;

    // 1. 初始化数据加载
    if (datas.assistants.length === 0) {
      invoke<Assistant[]>('load_assistants')
        .then((loaded) => {
          if (Array.isArray(loaded) && loaded.length > 0) {
            setDatas({ assistants: loaded });
            if (!currentAssistantId()) setCurrentAssistantId(loaded[0].id);
          } else {
            const defaultAsst = createAssistant('默认助手');
            setDatas('assistants', [defaultAsst]);
            setCurrentAssistantId(defaultAsst.id);
            saveSingleAssistantToBackend(defaultAsst.id);
          }
        })
        .catch((err) => console.error("加载助手失败:", err));
    } else {
      const asst = datas.assistants.find(a => a.id === currentAssistantId());
      if (asst && !currentTopicId()) setCurrentTopicId(asst.topics[0]?.id || null);
    }
    const savedPath = localStorage.getItem('user-avatar-path');
    if (savedPath && globalUserAvatar() === '/icons/user.svg') {
      loadAvatarFromPath(savedPath).then(url => setGlobalUserAvatar(url));
    }
    // 2. 拖拽状态监听
    listen('tauri://drag-enter', () => setIsDragging(true)).then(un => unlistenDragEnter = un);
    listen('tauri://drag-leave', () => setIsDragging(false)).then(un => unlistenDragLeave = un);

    // 3. 文件解析监听
    listen<{ paths: string[] }>('tauri://drag-drop', async (event) => {
      setIsDragging(false);
      setIsProcessing(true);
      const paths = event.payload.paths;
      for (const path of paths) {
        try {
          const content = await invoke<string>('process_file_content', { path });
          const fileName = path.split(/[\\/]/).pop() || '未知文件';
          setPendingFiles(prev => [...prev, { name: fileName, content }]);
        } catch (err) { alert(`处理文件失败: ${err}`); }
      }
      setIsProcessing(false);
    }).then(un => unlistenDrop = un);

    // 4. 流式 LLM 消息监听
    listen<any>('llm-chunk', (event) => {
      const { assistant_id, topic_id, content, done } = event.payload;

      if (done) {
        setIsThinking(false);
        setTypingIndex(null);
        saveSingleAssistantToBackend(assistant_id);
        const asst = datas.assistants.find(a => a.id === assistant_id);
        const topic = asst?.topics.find((t: Topic) => t.id === topic_id);

        if (topic && topic.history.length === 2 && (topic.name.startsWith("新话题") || topic.name.startsWith("默认话题"))) {
          const userText = topic.history[0].displayText || topic.history[0].content;
          const aiText = topic.history[1].content;
          summarizeTopic(assistant_id, topic_id, userText, aiText);
        }
        return;
      }

      const asst = datas.assistants.find(a => a.id === assistant_id);
      const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
      if (!topic) return;

      const lastIdx = topic.history.length - 1;
      if (lastIdx >= 0 && topic.history[lastIdx].role === 'assistant') {
        setDatas('assistants', a => a.id === assistant_id, 'topics', t => t.id === topic_id, 'history', lastIdx, 'content', (old: string) => old + content);
      }

      const area = document.querySelector('.chat-messages-area');
      if (area) requestAnimationFrame(() => area.scrollTop = area.scrollHeight);
    }).then(un => unlistenLLM = un);

    // 5. 点击外部关闭菜单
    const handleClickOutside = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      if (!target.closest('.assistant-context-menu') && !target.closest('.assistant-menu-button')) {
        if (showMenuDiv()) closeMenu();
        if (showTopicMenuDiv()) closeTopicMenu();
      }
    };
    document.addEventListener('click', handleClickOutside);

    onCleanup(() => {
      if (unlistenLLM) unlistenLLM();
      if (unlistenDrop) unlistenDrop();
      if (unlistenDragEnter) unlistenDragEnter();
      if (unlistenDragLeave) unlistenDragLeave();
      document.removeEventListener('click', handleClickOutside);
    });
  });

  // =============================================================================
  // V. 界面交互与实用工具 (Handles & UI Tools)
  // =============================================================================

  /**
   * 左右侧面板拖拽缩放
   */
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
      setLeftPanelWidth(Math.min(Math.max(newWidth, 15), 25));
    } else if (isResizingRight) {
      const newWidth = ((initialRightW - deltaX) / totalW) * 100;
      setRightPanelWidth(Math.min(Math.max(newWidth, 15), 25));
    }
  };

  const stopResize = () => {
    isResizingLeft = isResizingRight = false;
    document.removeEventListener('mousemove', handleMouseMove);
    document.removeEventListener('mouseup', stopResize);
    document.body.style.cursor = document.body.style.userSelect = '';
  };

  /**
   * 助手上下文菜单
   */
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

  /**
   * 获取当前助手及话题对象
   */
  const currentAssistant = () => datas.assistants.find(a => a.id === currentAssistantId());
  const activeTopic = () => {
    const asst = currentAssistant();
    if (!asst) return null;
    return asst.topics.find((t: Topic) => t.id === currentTopicId()) || asst.topics[0];
  };

  /**
   * 自动管理话题切换及兜底创建
   */
  createEffect(() => {
    const asst = currentAssistant();
    const tId = currentTopicId();
    if (asst && tId) {
      setIsChangingTopic(true);
      const timer = setTimeout(() => setIsChangingTopic(false), 50);
      onCleanup(() => clearTimeout(timer));
    }
    if (asst && asst.topics.length > 0) {
      if (!currentTopicId() || !asst.topics.find((t: Topic) => t.id === currentTopicId())) {
        setCurrentTopicId(asst.topics[0].id);
      }
    } else if (asst && asst.topics.length === 0) {
      addTopic();
    }
  });

  /**
   * 核心：发送消息处理函数
   */
  const handleSendMessage = async () => {
    if (isThinking()) return;
    const currentMdl = selectedModel();
    if (!currentMdl) { alert("请先选择一个模型！"); return; }

    let userInputText = inputMessage().trim();
    const files = pendingFiles();
    if (!userInputText && files.length === 0) return;

    // 构建上下文
    let fullContext = userInputText;
    if (files.length > 0) {
      let fileContext = "以下是参考文件内容：\n";
      for (const file of files) {
        const safeContent = file.content.length > 10000 ? file.content.substring(0, 10000) + "...(已截断)" : file.content;
        fileContext += `\n[文件名: ${file.name}]\n${safeContent}\n`;
      }
      fullContext = `${fileContext}\n---\n用户问题：${userInputText}`;
    }

    const asstId = currentAssistantId();
    const topicId = currentTopicId();
    const asst = currentAssistant();
    const topic = activeTopic();
    if (!asstId || !topicId || !asst || !topic) return;

    // UI 展示消息对象
    const newUserMsg = {
      role: 'user' as const,
      content: fullContext,
      displayFiles: files.map(f => ({ name: f.name })),
      displayText: userInputText
    };

    setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId, 'history', h => [
      ...h,
      newUserMsg,
      { role: 'assistant' as const, content: "", modelId: currentMdl.model_id }
    ]);

    setPendingFiles([]); setInputMessage(""); setIsThinking(true);
    const newHistory = activeTopic()?.history || [];
    setTypingIndex(newHistory.length - 1);
    if (textareaRef) textareaRef.style.height = '40px';

    if (!config().apiKey) { alert("请先在设置页面配置 API Key"); return; }

    try {
      const messagesForAI = [
        { role: 'system', content: asst.prompt },
        ...topic.history.map((m: Message) => ({ role: m.role, content: m.content })),
        newUserMsg
      ];
      await invoke('call_llm_stream', {
        apiUrl: currentMdl.api_url, apiKey: currentMdl.api_key, model: currentMdl.model_id,
        assistantId: asstId, topicId: topicId, messages: messagesForAI
      });
    } catch (err: any) {
      alert(err.toString()); setIsThinking(false); setTypingIndex(null);
    }
  };

  /**
   * 停止流式生成
   */
  const handleStopGeneration = async () => {
    const asstId = currentAssistantId();
    const topicId = currentTopicId();
    if (!asstId || !topicId) return;
    try {
      await invoke('stop_llm_stream', { assistantId: asstId, topicId: topicId });
    } catch (err) {
      console.error("停止失败:", err);
    } finally {
      setIsThinking(false); setTypingIndex(null);
    }
  };

  /**
   * 实体新增与移除操作
   */
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

  // =============================================================================
  // VI. 组件渲染 (JSX)
  // =============================================================================

  return (
    <div class="chat-page" ref={el => chatPageRef = el}>

      {/* 1. 左侧面板：助手列表选择器 */}
      <div class="assistant-selector" style={{ width: `${leftPanelWidth()}%` }}>
        <div class="assistant-content">
          <For each={datas.assistants}>
            {(assistant) => (
              <div classList={{ 'assistant-item': true, 'active': assistant.id === currentAssistantId() }} onClick={() => setCurrentAssistantId(assistant.id)}>
                <Show when={editingAsstId() === assistant.id} fallback={<span class="assistant-name">{assistant.name}</span>}>
                  <input class="rename-input" value={assistant.name} ref={(el) => {
                    setTimeout(() => { el.focus(); el.select(); }, 0);
                  }} onBlur={(e) => saveRename(assistant.id, e.currentTarget.value)} onKeyDown={(e) => e.key === 'Enter' && saveRename(assistant.id, e.currentTarget.value)} onClick={(e) => e.stopPropagation()} />
                </Show>
                <button class="assistant-menu-button" onClick={(e) => openMenu(e as MouseEvent, assistant.id)}>
                  <svg xmlns="http://www.w3.org/2000/svg" fill="#FFFFFF" viewBox="0 0 24 24" stroke-width={1.5} class="size-6">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M10 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0-6a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0z" />
                  </svg>
                </button>
              </div>
            )}
          </For>
          <button class="add-assistant-button" onClick={addAssistant}>+ 新增助手</button>
        </div>
        <div class="resize-handle left-handle" onMouseDown={(e) => startResize(e as MouseEvent, 'left')}></div>
      </div>

      {/* 2. 中间区域：核心聊天对话展示与输入 */}
      <div class="chat-input-container">

        {/* 对话消息滚动流 */}
        <div class="chat-messages-area" classList={{ 'topic-switching': isChangingTopic() }}>
          <Show when={activeTopic()}>
            <For each={activeTopic()?.history}>
              {(msg: any, index) => (
                <div class={`message ${msg.role}`} style={{ "animation-delay": `${Math.min(index() * 0.03, 0.4)}s`, "animation-duration": typingIndex() === index() ? "0.1s" : "0.35s" }}>

                  <div class="message-wrapper">

                    {/* 如果是助手，头像在左侧 */}
                    <Show when={msg.role === 'assistant'}>
                      <div class="chat-avatar-container ai">
                        <img src={getModelLogo(msg.modelId || selectedModel()?.model_id || "")} alt="AI" class="chat-avatar-img" />
                      </div>
                    </Show>

                    <div class="message-body">

                      <div class="message-content" classList={{ 'typing': typingIndex() === index() }}>
                        {/* 文件附件卡片 */}
                        <Show when={msg.role === 'user' && msg.displayFiles && msg.displayFiles.length > 0}>
                          <For each={msg.displayFiles}>
                            {(file: any) => (
                              <div class="file-attachment-card">
                                <div class="file-icon-wrapper"><svg fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" /></svg></div>
                                <div class="file-info"><div class="file-name">{file.name}</div><div class="file-meta">已读取文本内容</div></div>
                              </div>
                            )}
                          </For>
                        </Show>
                        {/* 消息文本渲染 */}
                        <div class="message-text-part">
                          <Markdown content={msg.role === 'user' && msg.displayText !== undefined ? msg.displayText : msg.content} />
                        </div>
                      </div>


                      <Show when={msg.role === 'assistant' && (msg.modelId || selectedModel()?.model_id)}>
                        <div class="message-model-info">
                          {msg.modelId || selectedModel()?.model_id}
                        </div>
                      </Show>
                      {/* 消息气泡快捷操作 */}
                      <div class="message-actions">
                        <button class="copy-bubble-button" onClick={(e) => {
                          e.stopPropagation();
                          const text = msg.role === 'user' && msg.displayText !== undefined ? msg.displayText : msg.content;
                          if (!text) return;
                          navigator.clipboard.writeText(text).then(() => {
                            const btn = e.currentTarget; const label = btn.querySelector('span');
                            if (label) {
                              const originalText = label.innerText; btn.classList.add('copied'); label.innerText = '已复制';
                              setTimeout(() => { btn.classList.remove('copied'); label.innerText = originalText; }, 2000);
                            }
                          });
                        }}>
                          <svg fill="none" viewBox="0 0 24 24" stroke="currentColor" style="width: 14px; height: 14px;"><path d="M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3" /></svg>
                          <span>复制</span>
                        </button>
                      </div>


                    </div>
                    <Show when={msg.role === 'user'}>
                      <div class="chat-avatar-container user">
                        <img src={globalUserAvatar()} alt="User" class="chat-avatar-img" />
                      </div>
                    </Show>
                  </div>
                </div>
              )}
            </For>
            <Show when={isThinking()}>
              <div class="message assistant">
                <div class="message-wrapper">
                  <div class="chat-avatar-container ai">
                    <img src={getModelLogo(selectedModel()?.model_id || "")} alt="AI" class="chat-avatar-img" />
                  </div>
                  <div class="message-body">
                    <div class="message-content" style="opacity: 0.6">AI 正在思考中...</div>
                  </div>
                </div>
              </div>
            </Show>
          </Show>
        </div>

        {/* 状态指示：解析中 & 文件预览 */}
        <Show when={isProcessing()}><div class="loading-overlay">正在解析文件内容...</div></Show>
        <div class="file-tags-container">
          <For each={pendingFiles()}>
            {(file, i) => <div class="file-tag"><span class="file-icon">📄</span>{file.name}<button onClick={() => setPendingFiles(p => p.filter((_, idx) => idx !== i()))}>×</button></div>}
          </For>
        </div>

        {/* 底部输入框区域 */}
        <div class="chat-input-wrapper">
          <textarea
            ref={textareaRef}
            class="chat-input"
            placeholder="输入消息... (Ctrl + Enter 换行)"
            value={inputMessage()}
            onInput={(e) => {
              const target = e.currentTarget; setInputMessage(target.value);
              target.style.height = 'auto'; const newHeight = target.scrollHeight; target.style.height = `${newHeight}px`;
              const maxHeight = parseFloat(window.getComputedStyle(target).maxHeight);
              if (newHeight > maxHeight) { target.style.overflowY = 'auto'; target.style.height = `${maxHeight}px`; }
              else { target.style.overflowY = 'hidden'; }
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                const target = e.currentTarget;
                if (e.ctrlKey) {
                  e.preventDefault();
                  const start = target.selectionStart; const end = target.selectionEnd; const value = target.value;
                  const newValue = value.substring(0, start) + "\n" + value.substring(end);
                  setInputMessage(newValue);
                  setTimeout(() => { target.selectionStart = target.selectionEnd = start + 1; target.dispatchEvent(new Event('input', { bubbles: true })); }, 0);
                } else if (!e.shiftKey) {
                  e.preventDefault(); handleSendMessage();
                }
              }
            }}
            rows={1}
          />
          <button classList={{ 'send-message-button': true, 'stop-button': isThinking() }} onClick={() => isThinking() ? handleStopGeneration() : handleSendMessage()}>
            <Show when={isThinking()} fallback={<svg fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="size-6"><path stroke-linecap="round" stroke-linejoin="round" d="M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.27 20.876L5.999 12zm0 0h7.5" /></svg>}>
              <svg fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="size-6"><rect x="6" y="6" width="12" height="12" rx="2" fill="currentColor" /><path d="M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0z" /></svg>
            </Show>
            <span>{isThinking() ? "停止" : "发送"}</span>
          </button>
        </div>
      </div>

      {/* 3. 右侧面板：话题列表 */}
      <div class="dialog-container" style={{ width: `${rightPanelWidth()}%` }}>
        <div class="resize-handle right-handle" onMouseDown={(e) => startResize(e as MouseEvent, 'right')}></div>
        <div class="dialog-content">
          <Show when={currentAssistant()}>
            {(asst) => (
              <>
                <div class="info-header" style="border-bottom: 1px solid #08ddf9; padding-bottom: 10px; margin-bottom: 15px;"><h3>{asst().name} 的话题</h3></div>
                <button class="add-topic-button" onClick={addTopic}>+ 新建话题</button>
                <div class="topics-list">
                  <For each={asst().topics}>
                    {(topic) => (
                      <div classList={{ 'topic-item': true, 'active': topic.id === currentTopicId() }} onClick={() => setCurrentTopicId(topic.id)}>
                        <Show when={editingTopicId() === topic.id} fallback={<span class="topic-name">{topic.name}</span>}>
                          <input class="rename-input" style="width: 70%;" value={topic.name} ref={(el) => { setTimeout(() => { el.focus(); el.select(); }, 0); }} onBlur={(e) => saveTopicRename(asst().id, topic.id, e.currentTarget.value)} onKeyDown={(e) => e.key === 'Enter' && saveTopicRename(asst().id, topic.id, e.currentTarget.value)} onClick={(e) => e.stopPropagation()} />
                        </Show>
                        <button class="assistant-menu-button" style="width: 24px; height: 24px;" onClick={(e) => openTopicMenu(e as MouseEvent, topic.id)}>
                          <svg fill="#FFFFFF" viewBox="0 0 24 24" stroke-width={1.5} style="width: 18px; height: 18px;"><path d="M10 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0-6a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0z" /></svg>
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

      {/* 4. 上下文菜单与浮层 */}
      {showMenuDiv() && (
        <div class="assistant-context-menu" classList={{ 'menu-exiting': isMenuAnimatingOut() }} style={{ top: `${menuState().y}px`, left: `${menuState().x}px` }}>
          <button class="context-menu-button" onClick={() => { setEditingAsstId(menuState().targetId); closeMenu(); }}>重命名</button>
          <button class="context-menu-button delete" onClick={() => removeAssistant(menuState().targetId)}>删除助手</button>
        </div>
      )}

      {showTopicMenuDiv() && (
        <div class="assistant-context-menu" classList={{ 'menu-exiting': isTopicMenuAnimatingOut() }} style={{ top: `${topicMenuState().y}px`, left: `${topicMenuState().x}px` }}>
          <button class="context-menu-button" onClick={() => { setEditingTopicId(topicMenuState().targetTopicId); closeTopicMenu(); }}>重命名</button>
          <button class="context-menu-button delete" onClick={() => deleteTopic(currentAssistantId(), topicMenuState().targetTopicId)}>删除话题</button>
        </div>
      )}

      {/* 5. 拖拽文件拖放区 (Overlay) */}
      <Show when={isDragging()}>
        <div class="drag-drop-overlay">
          <div class="drag-drop-content">
            <div class="drag-icons">
              <div class="drag-icon-card side"><svg width="24" height="24" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"></path></svg></div>
              <div class="drag-icon-card center"><svg width="32" height="32" fill="none" stroke="currentColor" stroke-width="2.5" viewBox="0 0 24 24"><path d="M12 16V8m0 0l-3 3m3-3l3 3m-9 8h12"></path></svg></div>
              <div class="drag-icon-card side"><svg width="24" height="24" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z"></path></svg></div>
            </div>
            <h2>上传文件</h2>
            <p>拖拽文件到这里，支持解析 PDF、Docx、pptx 和文本代码文件</p>
            <div class="dashed-border"></div>
          </div>
        </div>
      </Show>

    </div>
  );
};

export default Chat;