import { Component, createSignal, onMount, onCleanup, createEffect } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  datas, setDatas, currentAssistantId, setCurrentAssistantId, currentTopicId, setCurrentTopicId,
  saveSingleAssistantToBackend, Assistant, Topic, selectedModel, reasoningLevel,
} from '../store/store';
import AssistantSidebar from '../components/AssistantSidebar';
import ChatInterface from '../components/ChatInterface';
import TopicSidebar from '../components/TopicSidebar';

let isFirstAppLaunch = true;
const DEFAULT_ASST_ID = "default-assistant-id";

/**
 * 辅助函数：创建新话题对象
 * @param name - 可选的话题名称，默认生成带时间戳的名称
 * @returns Topic 对象，包含唯一 ID、名称、空历史记录和空摘要
 */
const createTopic = (name?: string): Topic => ({
  id: Date.now().toString(),                                // 使用当前时间戳作为唯一标识符
  name: name || `新话题 ${new Date().toLocaleTimeString()}`, // 默认名称包含创建时间
  history: [],                                              // 消息历史记录数组
  summary: ""                                               // SQLite 存储方案新增：长期记忆摘要，用于压缩历史上下文
});

/**
 * 辅助函数：创建新助手对象
 * @param name - 可选的助手名称
 * @param id - 可选的指定 ID（用于从数据库恢复时使用已有 ID）
 * @returns Assistant 对象，包含 ID、名称、系统提示词和默认话题
 */
const createAssistant = (name?: string, id?: string): Assistant => ({
  id: id ?? Date.now().toString(),        // 若未提供 ID 则生成新的时间戳 ID
  name: name || '新助手',                  // 默认助手名称
  prompt: '你是一个乐于助人的 AI 助手。',     // 默认系统提示词
  topics: [createTopic('默认话题')]        // 每个助手默认创建一个"默认话题"
});

/**
 * 聊天页面主组件
 * 管理三栏布局、所有业务逻辑和状态流转
 * @component
 */
const ChatPage: Component = () => {

  const [leftPanelWidth, setLeftPanelWidth] = createSignal(
    Number(localStorage.getItem('chat-left-panel-width')) || 18
  ); // 左侧面板宽度百分比（助手列表），默认 18%，范围 15%-30%
  const [rightPanelWidth, setRightPanelWidth] = createSignal(
    Number(localStorage.getItem('chat-right-panel-width')) || 18
  ); // 右侧面板宽度百分比（话题列表），默认 18%，范围 15%-30%
  const [isResizing, setIsResizing] = createSignal(false);
  const [isLeftCollapsed, setIsLeftCollapsed] = createSignal(localStorage.getItem('left-collapsed') === 'true'); // 左右两侧面板宽度调整逻辑
  const [isRightCollapsed, setIsRightCollapsed] = createSignal(localStorage.getItem('right-collapsed') === 'true');
  const [inputMessage, setInputMessage] = createSignal("");                       // 当前输入框中的消息文本
  const [pendingFiles, setPendingFiles] = createSignal<{ name: string, content: string, type: 'text' | 'image' }[]>([]); // 待发送的文件列表（用户上传但尚未发送的文件）
  const [isThinking, setIsThinking] = createSignal(false);                        // AI 是否正在思考/生成回复（控制加载动画和停止按钮）
  const [isProcessing, setIsProcessing] = createSignal(false);                    // 是否正在处理文件（控制文件解析加载状态）
  const [isDragging, setIsDragging] = createSignal(false);                        // 是否正在拖拽文件到窗口（控制拖拽状态样式）
  const [isChangingTopic, setIsChangingTopic] = createSignal(false);              // 是否正在切换话题（控制切换动画）
  const [typingIndex, setTypingIndex] = createSignal<number | null>(null);        // 当前正在打字机效果显示的消息索引，null 表示无打字效果
  const [editingAsstId, setEditingAsstId] = createSignal<string | null>(null);    // 当前正在编辑名称的助手 ID，null 表示无编辑中
  const [editingTopicId, setEditingTopicId] = createSignal<string | null>(null);  // 当前正在编辑名称的话题 ID，null 表示无编辑中

  /** 页面根元素引用，用于计算拖拽调整面板宽度时的相对位置 */
  let chatPageRef: HTMLDivElement | undefined;
  /**
   * 计算左侧面板显示宽度
   * @returns {number} 左侧面板宽度，如果折叠则返回0
   */
  const displayLeftWidth = () => isLeftCollapsed() ? 0 : leftPanelWidth();
  /**
   * 计算右侧面板显示宽度
   * @returns {number} 右侧面板宽度，如果折叠则返回0
   */
  const displayRightWidth = () => isRightCollapsed() ? 0 : rightPanelWidth();

  /**
   * 切换左侧面板的折叠状态
   * @param {MouseEvent} e - 鼠标事件
   */
  const toggleLeft = (e: MouseEvent) => {
    e.stopPropagation(); // 防止触发拖拽
    const newState = !isLeftCollapsed();
    setIsLeftCollapsed(newState);
    localStorage.setItem('left-collapsed', String(newState));
    if (!newState && leftPanelWidth() < 5) {
      setLeftPanelWidth(18);
    }
  };

  /**
   * 切换右侧面板的折叠状态
   * @param {MouseEvent} e - 鼠标事件
   */
  const toggleRight = (e: MouseEvent) => {
    e.stopPropagation();
    const newState = !isRightCollapsed();
    setIsRightCollapsed(newState);
    localStorage.setItem('right-collapsed', String(newState));
    if (!newState && rightPanelWidth() < 5) {
      setRightPanelWidth(18);
    }
  };

  /**
   * 当前选中的助手对象
   * @returns Assistant | undefined
   */
  const currentAssistant = () => datas.assistants.find(a => a.id === currentAssistantId());

  /**
   * 当前激活的话题对象
   * @returns Topic | null
   */
  const activeTopic = () => {
    const asst = currentAssistant();
    if (!asst) return null;
    return asst.topics.find((t: Topic) => t.id === currentTopicId()) || asst.topics[0] || null;
  };

  /**
   * 处理文件上传和解析
   * 调用 Tauri 后端读取本地文件内容，支持文本文件和图片
   * M7 加固：前端做扩展名白名单预校验，避免无效调用
   * @param filePath - 文件的绝对路径
   * @param fileType - 文件类型提示（'file' 或 'image'）
   */
  const handleFileUpload = async (filePath: string, fileType: 'file' | 'image') => {
    const fileName = filePath.split(/[\\/]/).pop() || '未知文件';
    const ext = (fileName.split('.').pop() || '').toLowerCase();
    const ALLOWED_IMG = ['png', 'jpg', 'jpeg', 'webp'];
    const ALLOWED_DOC = ['pdf', 'docx', 'pptx', 'txt', 'md', 'json', 'csv', 'log', 'xml', 'yaml', 'yml', 'ini', 'tsv'];
    const isImg = fileType === 'image' || ALLOWED_IMG.includes(ext);
    const isDoc = ALLOWED_DOC.includes(ext);
    if (!isImg && !isDoc) {
      alert(`不支持的文件类型: .${ext}\n仅支持: ${[...ALLOWED_IMG, ...ALLOWED_DOC].join(', ')}`);
      return;
    }
    setIsProcessing(true);
    try {
      const content = await invoke<string>('process_file_content', { path: filePath });
      setPendingFiles(prev => [...prev, { name: fileName, content, type: isImg ? 'image' : 'text' }]);
    } catch (err) {
      alert(err);
    } finally {
      setIsProcessing(false);
    }
  };

  /**
   * 检查并总结对话历史
   * 当历史记录超过25条时，触发总结机制以压缩上下文
   */
  const checkAndSummarize = async () => {
    const topic = activeTopic();
    const currentMdl = selectedModel();
    if (!topic || !currentMdl || isThinking()) return;
    // 当历史记录多于 25 条时触发总结机制
    if (topic.history.length > 25) {
      console.log("正在通过 SQLite 触发历史总结...");
      // 取前 15 条消息进行总结（保留后 10 条保持上下文连贯性）
      const messagesToSummarize = topic.history.slice(0, 15);
      try {
        // 调用后端 LLM 接口生成摘要
        const newSummarySnippet = await invoke<string>('summarize_history', {
          apiUrl: currentMdl.api_url,
          apiKey: currentMdl.api_key,
          model: currentMdl.model_id,
          messages: messagesToSummarize
        });

        // 重新获取最新话题状态（防止期间已切换）
        const latestTopic = activeTopic();
        if (!latestTopic) return;

        // 保留后半部分对话（索引 15 之后），前半部分进入 summary
        const updatedHistory = latestTopic.history.slice(15);
        // 合并历史摘要：若已有摘要则追加新摘要，否则直接使用新摘要
        const combinedSummary = latestTopic.summary
          ? `[历史背景]: ${latestTopic.summary}\n[近期增补]: ${newSummarySnippet}`
          : newSummarySnippet;

        // 同步更新全局状态（SolidJS Store 的嵌套更新语法）
        setDatas('assistants', a => a.id === currentAssistantId(), 'topics', t => t.id === latestTopic.id, {
          history: updatedHistory,
          summary: combinedSummary
        });

        // 持久化到 SQLite 数据库
        await saveSingleAssistantToBackend(currentAssistantId()!);
      } catch (e) {
        console.error("生成总结失败:", e);
      }
    }
  };

  /**
   * 处理发送消息的逻辑
   * 包括文件处理、API调用和状态更新
   */
  const handleSendMessage = async () => {
    const currentMdl = selectedModel();
    const topicObj = activeTopic();
    const asstObj = currentAssistant();

    // 前置条件检查：必须有模型、话题、助手，且 AI 不在生成中
    if (!currentMdl || !topicObj || !asstObj || isThinking()) return;

    const userInput = inputMessage().trim();
    const files = pendingFiles();
    // 必须满足：有文本输入或有文件附件
    if (!userInput && files.length === 0) return;

    // 分离文本文件和图片文件，分别处理
    const documents = files.filter(f => f.type === 'text');
    const images = files.filter(f => f.type === 'image');
    // 构造文件上下文文本：列出所有文本文件的内容
    let textContext = documents.length > 0
      ? "参考文件内容：\n" + documents.map(d => `[${d.name}]\n${d.content}`).join('\n') + "\n---\n"
      : "";

    // 最终发送给 AI 的文本内容（文件上下文 + 用户输入）
    const finalPrompt = `${textContext}${userInput}`;

    // 适配多模态 API 格式：若有图片则构造数组格式，否则保持纯文本
    const apiContent = images.length > 0 ? [
      { type: "text", text: finalPrompt },
      ...images.map(img => ({ type: "image_url", image_url: { url: img.content } }))
    ] : finalPrompt;

    const asstId = currentAssistantId();
    const topicId = currentTopicId();
    if (!asstId || !topicId) return;

    const newUserMsg = {
      id: crypto.randomUUID(),
      role: 'user' as const,
      content: apiContent,
      displayFiles: files.map(f => ({ name: f.name })),
      displayText: userInput
    };

    const currentAsst = currentAssistant();
    const currentTopic = activeTopic();
    if (!currentAsst || !currentTopic) return;

    /** 根据推理强度注入对应的 system 提示, 让模型使用 <think>...</think> 输出思考过程 */
    const reasoningPrompt = (() => {
        switch (reasoningLevel()) {
            case 'low':    return '在回答前先进行简单思考. 用 <think> 标签包裹你的推理过程, 再给出最终回答. 控制思考长度, 简单问题不要过度展开.';
            case 'medium': return '在回答前先进行中等深度的思考. 用 <think> 标签包裹你的推理过程 (分析问题、拆解步骤、对比方案), 再给出最终回答.';
            case 'high':   return '在回答前进行深入的多步推理. 必须在 <think> 标签中详细分析问题、列出前提、考虑边界情况、对比多种方案, 再给出严谨的最终回答. 思考越充分越好.';
            default:       return null;
        }
    })();

    const messagesForAI = [
      { role: 'system', content: currentAsst.prompt },
      ...(reasoningPrompt ? [{ role: 'system', content: reasoningPrompt }] : []),
      ...(currentTopic.summary ? [{
        role: 'system',
        content: `这是之前对话的摘要记忆，请结合这些上下文回答：\n${currentTopic.summary}`
      }] : []),
      ...currentTopic.history.map((m: any) => ({ role: m.role, content: m.content })),
      { role: 'user', content: newUserMsg.content }
    ];

    const lastMsg = messagesForAI[messagesForAI.length - 1];
    if (lastMsg.role !== 'user') {
      console.error("错误：发送给 API 的最后一条消息不是 User!", lastMsg);
      return;
    }

    // 更新本地 Store：添加用户消息和空的 AI 占位消息
    setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId, 'history', h => [
      ...h,
      newUserMsg,
      { id: crypto.randomUUID(), role: 'assistant' as const, content: "", modelId: selectedModel()?.model_id }
    ]);

    // 清空输入状态和文件列表，设置生成中状态
    setInputMessage("");
    setPendingFiles([]);
    setIsThinking(true);
    // 设置打字机效果索引为刚添加的 AI 消息位置
    setTypingIndex(activeTopic()?.history.length! - 1);

    try {
      // 调用 Tauri 后端流式接口（非阻塞，通过事件监听接收数据）
      await invoke('call_llm_stream', {
        apiUrl: currentMdl.api_url,
        apiKey: currentMdl.api_key,
        model: currentMdl.model_id,
        assistantId: asstId,
        topicId: topicId,
        messages: messagesForAI
      });

    } catch (err) {
      alert(err); // 调用失败时提示错误
      setIsThinking(false);
    }
  };

  /**
   * 停止当前的AI生成过程
   */
  const handleStopGeneration = async () => {
    await invoke('stop_llm_stream', {
      assistantId: currentAssistantId(),
      topicId: currentTopicId()
    });
    setIsThinking(false);
    setTypingIndex(null);
  };

  /**
   * 添加新的助手
   * 创建新助手并设置为当前选中助手
   */
  const addAssistant = async () => {
    const newAsst = createAssistant(`新助手 ${datas.assistants.length + 1}`);
    setDatas('assistants', prev => [...prev, newAsst]);
    setCurrentAssistantId(newAsst.id);
    setCurrentTopicId(newAsst.topics[0].id);
    await saveSingleAssistantToBackend(newAsst.id);
  };

  /**
   * 添加新的话题到当前助手
   * 创建新话题并设置为当前选中话题
   */
  const addTopic = async () => {
    const asstId = currentAssistantId();
    if (!asstId) return;
    const newT = createTopic();
    setDatas('assistants', a => a.id === asstId, 'topics', prev => [...prev, newT]);
    setCurrentTopicId(newT.id);
    await saveSingleAssistantToBackend(asstId);
  };

  /**
   * 拖拽调整面板宽度
   * @param e - MouseEvent 鼠标事件
   * @param type - 'left' 调整左面板，'right' 调整右面板
   */
  const startResize = (e: MouseEvent, type: 'left' | 'right') => {
    e.preventDefault();
    // 2. 开始拖拽时，设为 true
    setIsResizing(true);

    const handleMove = (moveEvent: MouseEvent) => {
      const totalW = chatPageRef!.offsetWidth;
      if (type === 'left') {
        setLeftPanelWidth(Math.min(Math.max((moveEvent.clientX / totalW) * 100, 15), 30));
      } else {
        setRightPanelWidth(Math.min(Math.max(((totalW - moveEvent.clientX) / totalW) * 100, 15), 30));
      }
    };

    const stopResize = () => {
      // 3. 停止拖拽时，设为 false
      setIsResizing(false);
      document.removeEventListener('mousemove', handleMove);
      document.removeEventListener('mouseup', stopResize);
    };

    document.addEventListener('mousemove', handleMove);
    document.addEventListener('mouseup', stopResize);
  };

  onMount(() => {
    // 首次进入：从 SQLite 加载所有助手数据
    invoke<Assistant[]>('load_assistants').then(async (loaded) => {
      let finalAssistants = [...loaded];

      // 1. 确保默认助手存在
      let defaultAsst = finalAssistants.find(a => a.id === DEFAULT_ASST_ID);
      if (!defaultAsst) {
        // createAssistant 内部已经带了一个“默认话题”
        defaultAsst = createAssistant('默认助手', DEFAULT_ASST_ID);
        finalAssistants = [defaultAsst, ...finalAssistants];
        setDatas('assistants', finalAssistants);
        await saveSingleAssistantToBackend(DEFAULT_ASST_ID);
      } else {
        setDatas('assistants', finalAssistants);
      }

      // 2. 处理应用启动时的默认选中（仅冷启动触发）
      if (isFirstAppLaunch) {
        // 默认选中
        setCurrentAssistantId(DEFAULT_ASST_ID);

        const asst = datas.assistants.find(a => a.id === DEFAULT_ASST_ID);
        if (asst && asst.topics.length > 0) {
          // 如果已有话题，选中第一个，不再新建
          setCurrentTopicId(asst.topics[0].id);
        } else if (asst) {
          // 如果万一没话题（极端情况），补充一个
          const newDefaultTopic = createTopic('默认话题');
          setDatas('assistants', a => a.id === DEFAULT_ASST_ID, 'topics', [newDefaultTopic]);
          setCurrentTopicId(newDefaultTopic.id);
          await saveSingleAssistantToBackend(DEFAULT_ASST_ID);
        }

        isFirstAppLaunch = false;
      }
      console.log("成功加载数据");
    })
      .catch((err) => {
        // 如果后端报错，这里会打印出来
        console.error("加载助手列表失败:", err);
        alert("数据库加载失败: " + err);
      });

    // 设置多个事件监听器，存储 unlisten 函数用于清理
    const unlistens = [
      listen('tauri://drag-enter', () => setIsDragging(true)),
      listen('tauri://drag-leave', () => setIsDragging(false)),
      listen<{ paths: string[] }>('tauri://drag-drop', async (e) => {
        setIsDragging(false);
        for (const p of e.payload.paths) await handleFileUpload(p, 'file');
      }),
      listen<any>('llm-chunk', (e) => {
        const { assistant_id, topic_id, content, done } = e.payload;
        if (done) {
          setIsThinking(false);
          setTypingIndex(null);
          saveSingleAssistantToBackend(assistant_id);
          setTimeout(() => checkAndSummarize(), 500);
          return;
        }

        // 流式数据追加：查找对应消息并追加内容
        const asst = datas.assistants.find(a => a.id === assistant_id);
        const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
        if (topic) {
          const lastIdx = topic.history.length - 1; // 最后一条消息（AI 回复）
          setDatas('assistants', a => a.id === assistant_id,
            'topics', t => t.id === topic_id,
            'history', lastIdx, 'content', (old: string) => old + content);
        }
      })
    ];

    // 组件卸载时清理所有事件监听
    onCleanup(() => unlistens.forEach(u => u.then(fn => fn())));
  });

  createEffect(() => {
    const tId = currentTopicId();
    if (tId) {
      setIsChangingTopic(true);
      setTimeout(() => setIsChangingTopic(false), 50); // 50ms 后恢复，触发过渡
    }
  });

  createEffect(() => {
    const tId = currentTopicId();
    // 只有在非初次静默加载且 tId 真正存在时触发
    if (tId && !isFirstAppLaunch) {
      setIsChangingTopic(true);
      setTimeout(() => setIsChangingTopic(false), 50);
    }
  });
  createEffect(() => {
    localStorage.setItem('chat-left-panel-width', leftPanelWidth().toString());
  });
  createEffect(() => {
    localStorage.setItem('chat-right-panel-width', rightPanelWidth().toString());
  });

  return (
    <div class="h-full flex gap-[3px] p-[1px]" style="background: transparent;"
      classList={{ 'is-resizing': isResizing() }} ref={chatPageRef}>
      <AssistantSidebar
        width={displayLeftWidth()}
        isCollapsed={isLeftCollapsed()}
        onToggle={toggleLeft}
        onResize={(e) => !isLeftCollapsed() && startResize(e, 'left')}
        editingAsstId={editingAsstId()}
        setEditingAsstId={setEditingAsstId}
        addAssistant={addAssistant}
        isResizing={isResizing()}
      />

      <ChatInterface
        activeTopic={activeTopic()}
        isChangingTopic={isChangingTopic()}
        isThinking={isThinking()}
        isProcessing={isProcessing()}
        isDragging={isDragging()}
        typingIndex={typingIndex()}
        inputMessage={inputMessage()}
        setInputMessage={setInputMessage}
        pendingFiles={pendingFiles()}
        setPendingFiles={setPendingFiles}
        handleSendMessage={handleSendMessage}
        handleStopGeneration={handleStopGeneration}
        handleFileUpload={handleFileUpload}
      />

      <TopicSidebar
        width={displayRightWidth()}
        isCollapsed={isRightCollapsed()}
        onToggle={toggleRight}
        onResize={(e) => !isRightCollapsed() && startResize(e, 'right')}
        currentAssistant={currentAssistant()}
        editingTopicId={editingTopicId()}
        setEditingTopicId={setEditingTopicId}
        addTopic={addTopic}
        isResizing={isResizing()}
      />
    </div>
  );
};

export default ChatPage;