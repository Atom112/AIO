/**
 * Chat.tsx - AI 聊天应用主页面组件
 * 
 * 【功能概述】
 * 本文件是 Tauri 桌面 AI 聊天应用的核心页面组件，基于 SolidJS 框架构建。
 * 提供三栏式布局：左侧助手列表、中间聊天区域、右侧话题列表。
 * 支持多助手管理、多话题管理、文件上传（文本/图片）、流式 AI 对话、
 * 自动话题命名、历史消息摘要压缩等功能。
 * 
 * 【数据流流向】
 * 
 * 1. 初始化数据流（SQLite → UI）:
 *    Tauri 后端 SQLite → invoke('load_assistants') → store/datas → 组件状态
 * 
 * 2. 用户操作数据流（UI → SQLite）:
 *    用户操作 → setDatas() 更新状态 → saveSingleAssistantToBackend() → Tauri 后端 → SQLite
 * 
 * 3. AI 对话数据流:
 *    用户输入 → handleSendMessage() → invoke('call_llm_stream') → Tauri 后端 → LLM API
 *    LLM 流式响应 → listen('llm-chunk') → 更新 topic.history → UI 渲染
 * 
 * 4. 文件处理数据流:
 *    拖拽/选择文件 → handleFileUpload() → invoke('process_file_content') → 
 *    Tauri 后端读取本地文件 → 返回内容 → pendingFiles 状态 → 随消息发送
 * 
 * 5. 历史摘要数据流（自动触发）:
 *    消息数 > 25 条 → checkAndSummarize() → invoke('summarize_history') → 
 *    LLM 生成摘要 → 更新 topic.summary → SQLite 持久化 → 后续对话携带摘要上下文
 * 
 * 6. 自动命名数据流:
 *    首次对话完成 → generateAutoTitle() → invoke('summarize_history') → 
 *    LLM 生成标题 → 更新 topic.name → SQLite 持久化
 * 
 * 【依赖】
 * - SolidJS: 响应式 UI 框架
 * - @tauri-apps/api: Tauri 桌面应用 API（invoke 调用 Rust 后端，listen 监听事件）
 * - 本地 store: 全局状态管理（助手、话题、当前选中状态）
 */

import { Component, createSignal, onMount, onCleanup, createEffect } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  datas, setDatas, currentAssistantId, setCurrentAssistantId, currentTopicId, setCurrentTopicId,
  saveSingleAssistantToBackend, Assistant, Topic, selectedModel,
} from '../store/store';

import AssistantSidebar from '../components/AssistantSidebar';
import ChatInterface from '../components/ChatInterface';
import TopicSidebar from '../components/TopicSidebar';
import './Chat.css';

/**
 * 辅助函数：创建新话题对象
 * @param name - 可选的话题名称，默认生成带时间戳的名称
 * @returns Topic 对象，包含唯一 ID、名称、空历史记录和空摘要
 */
const createTopic = (name?: string): Topic => ({
  id: Date.now().toString(), // 使用当前时间戳作为唯一标识符
  name: name || `新话题 ${new Date().toLocaleTimeString()}`, // 默认名称包含创建时间
  history: [], // 消息历史记录数组
  summary: "" // SQLite 存储方案新增：长期记忆摘要，用于压缩历史上下文
});

/**
 * 辅助函数：创建新助手对象
 * @param name - 可选的助手名称
 * @param id - 可选的指定 ID（用于从数据库恢复时使用已有 ID）
 * @returns Assistant 对象，包含 ID、名称、系统提示词和默认话题
 */
const createAssistant = (name?: string, id?: string): Assistant => ({
  id: id ?? Date.now().toString(), // 若未提供 ID 则生成新的时间戳 ID
  name: name || '新助手', // 默认助手名称
  prompt: '你是一个乐于助人的 AI 助手。', // 默认系统提示词
  topics: [createTopic('默认话题')] // 每个助手默认创建一个"默认话题"
});

/**
 * 聊天页面主组件
 * 管理三栏布局、所有业务逻辑和状态流转
 */
const ChatPage: Component = () => {
  // ==================== 布局与 UI 状态 ====================
  
  /** 左侧面板宽度百分比（助手列表），默认 18%，范围 15%-30% */
  const [leftPanelWidth, setLeftPanelWidth] = createSignal(18);
  /** 右侧面板宽度百分比（话题列表），默认 18%，范围 15%-30% */
  const [rightPanelWidth, setRightPanelWidth] = createSignal(18);
  /** 当前输入框中的消息文本 */
  const [inputMessage, setInputMessage] = createSignal("");
  /** 待发送的文件列表（用户上传但尚未发送的文件） */
  const [pendingFiles, setPendingFiles] = createSignal<{ name: string, content: string, type: 'text' | 'image' }[]>([]);
  /** AI 是否正在思考/生成回复（控制加载动画和停止按钮） */
  const [isThinking, setIsThinking] = createSignal(false);
  /** 是否正在处理文件（控制文件解析加载状态） */
  const [isProcessing, setIsProcessing] = createSignal(false);
  /** 是否正在拖拽文件到窗口（控制拖拽状态样式） */
  const [isDragging, setIsDragging] = createSignal(false);
  /** 是否正在切换话题（控制切换动画） */
  const [isChangingTopic, setIsChangingTopic] = createSignal(false);
  /** 当前正在打字机效果显示的消息索引，null 表示无打字效果 */
  const [typingIndex, setTypingIndex] = createSignal<number | null>(null);
  /** 当前正在编辑名称的助手 ID，null 表示无编辑中 */
  const [editingAsstId, setEditingAsstId] = createSignal<string | null>(null);
  /** 当前正在编辑名称的话题 ID，null 表示无编辑中 */
  const [editingTopicId, setEditingTopicId] = createSignal<string | null>(null);

  /** 页面根元素引用，用于计算拖拽调整面板宽度时的相对位置 */
  let chatPageRef: HTMLDivElement | undefined;

  // ==================== 派生状态 (Computed) ====================
  
  /**
   * 当前选中的助手对象
   * 根据 currentAssistantId 从 datas.assistants 数组中查找
   * @returns Assistant | undefined
   */
  const currentAssistant = () => datas.assistants.find(a => a.id === currentAssistantId());

  /**
   * 当前激活的话题对象
   * 先获取当前助手，再从中查找 currentTopicId 对应的话题
   * 若找不到则返回该助手的第一个话题，若助手无话题则返回 null
   * @returns Topic | null
   */
  const activeTopic = () => {
    const asst = currentAssistant();
    if (!asst) return null;
    return asst.topics.find((t: Topic) => t.id === currentTopicId()) || asst.topics[0] || null;
  };

  // ==================== 业务逻辑函数 ====================
  
  /**
   * 处理文件上传和解析
   * 调用 Tauri 后端读取本地文件内容，支持文本文件和图片
   * @param filePath - 文件的绝对路径
   * @param fileType - 文件类型提示（'file' 或 'image'）
   */
  const handleFileUpload = async (filePath: string, fileType: 'file' | 'image') => {
    setIsProcessing(true); // 开始处理，显示加载状态
    try {
      // 从路径中提取文件名（兼容 Windows 和 Unix 路径分隔符）
      const fileName = filePath.split(/[\\/]/).pop() || '未知文件';
      // 调用 Rust 后端读取文件内容（文本直接读取，图片转为 base64）
      const content = await invoke<string>('process_file_content', { path: filePath });
      // 根据文件扩展名或传入的类型判断是否为图片
      const isImg = fileType === 'image' || ['png', 'jpg', 'jpeg'].includes(fileName.split('.').pop()?.toLowerCase() || '');
      // 添加到待发送文件列表
      setPendingFiles(prev => [...prev, { name: fileName, content, type: isImg ? 'image' : 'text' }]);
    } catch (err) { 
      alert(err); // 解析失败时提示错误
    } finally { 
      setIsProcessing(false); // 无论成功与否，结束加载状态
    }
  };

  /**
   * 核心优化：检测上下文长度并生成历史摘要
   * 当历史消息超过 25 条时，将前 15 条发送给 LLM 生成摘要
   * 摘要存储在 topic.summary 中，后续对话携带摘要以节省 Token
   * 同时从 history 中移除已摘要的消息，保持列表精简
   */
  const checkAndSummarize = async () => {
    const topic = activeTopic();
    const currentMdl = selectedModel();
    // 前置条件检查：必须有话题、模型，且 AI 不在生成中
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
   * 发送消息核心逻辑
   * 处理用户输入、文件附件、构造 API 请求格式、调用流式接口
   * 同时处理多模态内容（文本 + 图片）的格式转换
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

    // 构造 UI 消息对象（用于本地显示，与 API 格式可能不同）
    const newUserMsg = {
      role: 'user' as const,
      content: apiContent, // API 格式的内容（可能是字符串或数组）
      displayFiles: files.map(f => ({ name: f.name })), // UI 显示用的文件列表
      displayText: userInput // UI 显示用的纯文本（不含文件上下文）
    };

    // 更新本地 Store：添加用户消息和空的 AI 占位消息
    setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId, 'history', h => [
      ...h,
      newUserMsg,
      { role: 'assistant' as const, content: "", modelId: currentMdl.model_id } // 空的 AI 回复占位
    ]);

    // 清空输入状态和文件列表，设置生成中状态
    setInputMessage(""); 
    setPendingFiles([]); 
    setIsThinking(true);
    // 设置打字机效果索引为刚添加的 AI 消息位置
    setTypingIndex(activeTopic()?.history.length! - 1);

    try {
      // 构造发送给 AI 的完整消息数组
      const messagesForAI = [
        { role: 'system', content: asstObj.prompt }, // 系统提示词
        ...(topicObj.summary ? [{ // 若有历史摘要则作为系统消息插入
          role: 'system',
          content: `这是之前对话的摘要记忆，请结合这些上下文回答：\n${topicObj.summary}`
        }] : []),
        ...topicObj.history.map((m: any) => ({ role: m.role, content: m.content })) // 完整历史记录
      ];

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
   * 停止 AI 生成
   * 调用后端中断当前流式请求
   */
  const handleStopGeneration = async () => {
    await invoke('stop_llm_stream', { 
      assistantId: currentAssistantId(), 
      topicId: currentTopicId() 
    });
    setIsThinking(false);
    setTypingIndex(null);
  };

  // ==================== 助手与话题管理 ====================
  
  /**
   * 添加新助手
   * 创建助手对象 → 更新状态 → 选中新助手 → 持久化到 SQLite
   */
  const addAssistant = async () => {
    const newAsst = createAssistant(`新助手 ${datas.assistants.length + 1}`);
    setDatas('assistants', prev => [...prev, newAsst]);
    setCurrentAssistantId(newAsst.id);
    setCurrentTopicId(newAsst.topics[0].id);
    await saveSingleAssistantToBackend(newAsst.id);
  };

  /**
   * 添加新话题到当前助手
   * 创建话题对象 → 追加到当前助手的 topics 数组 → 选中新话题 → 持久化
   */
  const addTopic = async () => {
    const asstId = currentAssistantId();
    if (!asstId) return;
    const newT = createTopic();
    setDatas('assistants', a => a.id === asstId, 'topics', prev => [...prev, newT]);
    setCurrentTopicId(newT.id);
    await saveSingleAssistantToBackend(asstId);
  };

  // ==================== UI/UX 交互逻辑 ====================
  
  /**
   * 拖拽调整面板宽度
   * @param e - MouseEvent 鼠标事件
   * @param type - 'left' 调整左面板，'right' 调整右面板
   */
  const startResize = (e: MouseEvent, type: 'left' | 'right') => {
    e.preventDefault(); // 阻止默认拖拽行为
    // 鼠标移动处理：根据鼠标位置计算新的宽度百分比
    const handleMove = (moveEvent: MouseEvent) => {
      const totalW = chatPageRef!.offsetWidth; // 获取容器总宽度
      if (type === 'left') {
        // 左面板：根据鼠标 X 坐标计算百分比，限制在 15%-30%
        setLeftPanelWidth(Math.min(Math.max((moveEvent.clientX / totalW) * 100, 15), 30));
      } else {
        // 右面板：根据右侧剩余空间计算，限制在 15%-30%
        setRightPanelWidth(Math.min(Math.max(((totalW - moveEvent.clientX) / totalW) * 100, 15), 30));
      }
    };
    // 鼠标释放处理：移除事件监听
    const stopResize = () => {
      document.removeEventListener('mousemove', handleMove);
      document.removeEventListener('mouseup', stopResize);
    };
    // 绑定全局鼠标事件
    document.addEventListener('mousemove', handleMove);
    document.addEventListener('mouseup', stopResize);
  };

  /**
   * 自动生成话题标题
   * 在首次对话完成后，根据对话内容让 LLM 生成简短标题
   * @param asstId - 助手 ID
   * @param topicId - 话题 ID
   * @param history - 当前对话历史
   */
  const generateAutoTitle = async (asstId: string, topicId: string, history: any[]) => {
    const currentMdl = selectedModel();
    if (!currentMdl) return;

    console.log("正在自动生成话题名称...");

    try {
      // 构造专门用于生成标题的消息：完整历史 + 系统指令
      const messagesForNaming = [
        ...history.map((m: any) => ({ role: m.role, content: m.content })),
        {
          role: 'system',
          content: '请根据上述对话内容，总结一个非常简短的话题标题。要求：不超过10个字，不要包含标点符号，直接输出标题文字。'
        }
      ];

      // 复用 summarize_history 接口生成标题
      const newTitle = await invoke<string>('summarize_history', {
        apiUrl: currentMdl.api_url,
        apiKey: currentMdl.api_key,
        model: currentMdl.model_id,
        messages: messagesForNaming
      });

      if (newTitle) {
        // 清理可能的引号和空格
        const cleanedTitle = newTitle.replace(/["'""]/g, '').trim();

        // 更新本地 Store 中的话题名称
        setDatas('assistants', a => a.id === asstId,
          'topics', t => t.id === topicId,
          'name', cleanedTitle);

        // 同步到 SQLite 数据库
        await saveSingleAssistantToBackend(asstId);
      }
    } catch (e) {
      console.error("生成话题名称失败:", e);
    }
  };

  // ==================== 生命周期与事件监听 ====================
  
  /**
   * 组件挂载时初始化
   * 1. 从 SQLite 加载助手数据
   * 2. 设置 Tauri 拖拽事件监听（文件拖入）
   * 3. 设置 LLM 流式响应监听
   */
  onMount(() => {
    // 首次进入：从 SQLite 加载所有助手数据
    invoke<Assistant[]>('load_assistants').then(loaded => {
      if (loaded.length > 0) {
        setDatas('assistants', loaded);
        const firstAsst = loaded[0];
        setCurrentAssistantId(firstAsst.id);
        if (firstAsst.topics && firstAsst.topics.length > 0) {
          setCurrentTopicId(firstAsst.topics[0].id);
        }
      } else {
        addAssistant(); // 数据库为空时自动创建首个助手
      }
    });

    // 设置多个事件监听器，存储 unlisten 函数用于清理
    const unlistens = [
      // 拖拽进入窗口：显示拖拽状态样式
      listen('tauri://drag-enter', () => setIsDragging(true)),
      // 拖拽离开窗口：取消拖拽状态
      listen('tauri://drag-leave', () => setIsDragging(false)),
      // 拖拽释放文件：逐个处理上传的文件
      listen<{ paths: string[] }>('tauri://drag-drop', async (e) => {
        setIsDragging(false);
        for (const p of e.payload.paths) await handleFileUpload(p, 'file');
      }),
      // 流式输出监听：接收 LLM 生成的文本块
      listen<any>('llm-chunk', (e) => {
        const { assistant_id, topic_id, content, done } = e.payload;

        // 生成完成处理
        if (done) {
          setIsThinking(false);
          setTypingIndex(null);

          // 获取当前话题信息
          const currentAsst = datas.assistants.find(a => a.id === assistant_id);
          const currentTopic = currentAsst?.topics.find((t: any) => t.id === topic_id);

          if (currentTopic) {
            // 检测是否为首次对话（2 条消息：1 用户 + 1 AI）
            const isFirstInteraction = currentTopic.history.length === 2;
            // 检测标题是否为默认生成的（避免覆盖用户手动修改的标题）
            const isDefaultName = currentTopic.name.startsWith("新话题") || currentTopic.name === "默认话题";

            // 首次对话且标题为默认时，触发自动生成标题
            if (isFirstInteraction && isDefaultName) {
              generateAutoTitle(assistant_id, topic_id, currentTopic.history);
            }
          }

          // 保存当前状态到 SQLite
          saveSingleAssistantToBackend(assistant_id);
          // 延迟检查是否需要历史摘要（避免立即触发影响性能）
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

  /**
   * 话题切换动画效果
   * 当 currentTopicId 变化时，短暂设置 isChangingTopic 为 true
   * 用于触发 CSS 过渡动画
   */
  createEffect(() => {
    const tId = currentTopicId();
    if (tId) {
      setIsChangingTopic(true);
      setTimeout(() => setIsChangingTopic(false), 50); // 50ms 后恢复，触发过渡
    }
  });

  // ==================== 渲染 ====================
  
  return (
    <div class="chat-page" ref={chatPageRef}>
      {/* 左侧助手侧边栏 */}
      <AssistantSidebar
        width={leftPanelWidth()}
        onResize={(e) => startResize(e, 'left')}
        editingAsstId={editingAsstId()}
        setEditingAsstId={setEditingAsstId}
        addAssistant={addAssistant}
      />

      {/* 中间聊天主区域 */}
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

      {/* 右侧话题侧边栏 */}
      <TopicSidebar
        width={rightPanelWidth()}
        onResize={(e) => startResize(e, 'right')}
        currentAssistant={currentAssistant()}
        editingTopicId={editingTopicId()}
        setEditingTopicId={setEditingTopicId}
        addTopic={addTopic}
      />
    </div>
  );
};

export default ChatPage;