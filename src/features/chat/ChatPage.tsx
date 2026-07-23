import { Component, createSignal, onMount, onCleanup, createEffect, Show } from 'solid-js';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useNavigate } from '@solidjs/router';
import {
  datas, setDatas, currentAssistantId, setCurrentAssistantId, currentTopicId, setCurrentTopicId,
  saveSingleAssistantToBackend, Assistant, Topic, Message, PendingAttachment, StoredAttachment, selectedModel, setSelectedModel,
  resolveAssistantModel, modelKey, reasoningLevel, setReasoningLevel, webSearchEnabled, setWebSearchEnabled,
  pendingRenameRequest, setPendingRenameRequest,
  mcpServers, mcpServerStatus, resolveAssistantSkills,
  currentProjectId, currentProject,
  profileModelOverrides, allAvailableModels, resolveProfileModel, customSubagentProfiles,
  workflowState, setWorkflowState, type WorkflowState, type WorkflowStepState,
  isChatMode, projects, ensureProjectAssistant, showProjectCreateModal, setShowProjectCreateModal,
} from '../../core/store/store';
import { buildAgentSystemPrompt } from '../../core/agent-prompts';
import {
  registerCommand,
  unregisterCommand,
  resolveSlashCommand,
  getSlashCommands,
} from '../../core/shortcuts';
import ProjectSidebar from './components/ProjectSidebar';
import ProjectSettingsModal from './components/ProjectSettingsModal';
import ChatInterface from './components/ChatInterface';
import TopicSidebar from './components/TopicSidebar';
import { Portal } from 'solid-js/web';
import ShareModal from './components/ShareModal';
import ProblemsPanel from './components/ProblemsPanel';
import WorkflowVisualization from './components/WorkflowVisualization';
import type { PendingApproval } from './components/ToolApprovalBubble';
import { problemsPanelVisible, setProblemsPanelVisible, clearAllDiagnostics } from '../../core/store/diagnostics';

let isFirstAppLaunch = true;
const DEFAULT_ASST_ID = "default-assistant-id";

// 跟踪当前正在进行的"标题生成"任务，用于在用户切换话题时取消未完成的回调
// （Tauri invoke 暂不支持 AbortSignal，这里仅在 JS 侧跳过结果处理；
//  服务端调用仍会完成但结果被忽略，避免不必要的 state 更新）
let activeTitleGen: { topicId: string; cancelled: boolean } | null = null;

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
  modelId: selectedModel() ? modelKey(selectedModel()!) : undefined,  // 继承当前生效模型（复合键）作为新助手默认模型
  // 有项目上下文时自动启用内置文件系统 MCP，让 agent 开箱即用
  mcpServerIds: currentProjectId() ? ['__aio-filesystem__'] : [],
  skillIds: [],
  projectId: currentProjectId() ?? undefined, // 关联当前项目
  assistantType: id === DEFAULT_ASST_ID ? 'chat' : 'project',
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
  const [pendingFiles, setPendingFiles] = createSignal<PendingAttachment[]>([]); // 待发送的文件列表（已复制到应用附件目录但尚未关联消息）
  const [isThinking, setIsThinking] = createSignal(false);                        // AI 是否正在思考/生成回复（控制加载动画和停止按钮）
  const [isProcessing, setIsProcessing] = createSignal(false);                    // 是否正在处理文件（控制文件解析加载状态）
  const [isDragging, setIsDragging] = createSignal(false);                        // 是否正在拖拽文件到窗口（控制拖拽状态样式）
  const [isChangingTopic, setIsChangingTopic] = createSignal(false);              // 是否正在切换话题（控制切换动画）
  const [typingIndex, setTypingIndex] = createSignal<number | null>(null);        // 当前正在打字机效果显示的消息索引，null 表示无打字效果
  // setTimeout 批量流式内容合并（50ms 窗口），避免逐 token UI 更新（消除打字机效果）
  let streamBatch: { assistant_id: string; topic_id: string; content: string; agentStepsContent: string; lastIdx: number } | null = null;
  let streamBatchRAF: number | undefined;
  const [editingTopicId, setEditingTopicId] = createSignal<string | null>(null);  // 当前正在编辑名称的话题 ID，null 表示无编辑中
  const [settingsAsstId, setSettingsAsstId] = createSignal<string | null>(null);   // 当前打开设置弹窗的助手 ID，null 表示弹窗关闭
  // 待用户审批的工具调用列表
  /** 分享弹窗状态 */
  const [showShareModal, setShowShareModal] = createSignal(false);
  const [activeShareTopicId, setActiveShareTopicId] = createSignal<string | null>(null);
  /** 消息级选择模式 */
  const [isSelectingMessages, setIsSelectingMessages] = createSignal(false);
  const [selectedMessageIds, setSelectedMessageIds] = createSignal<Set<string>>(new Set());
  const [pendingApprovals, setPendingApprovals] = createSignal<PendingApproval[]>([]);
  /** 页面根元素引用，用于计算拖拽调整面板宽度时的相对位置 */
  let chatPageRef: HTMLDivElement | undefined;
  /**
   * 计算左侧面板显示宽度
   * @returns {number} 左侧面板宽度（存储值，折叠模式下由子组件自行处理）
   */
  const displayLeftWidth = () => leftPanelWidth();
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

  /** 分享弹窗展示的话题：优先使用来自 TopicSidebar 右键菜单指定的 ID，否则用当前活跃话题 */
  const shareTopic = (): Topic | null => {
    const asst = currentAssistant();
    if (!asst) return null;
    const sid = activeShareTopicId();
    if (sid) return asst.topics.find((t: Topic) => t.id === sid) || null;
    return activeTopic();
  };

  // -------- 消息级选择 --------

  /** 进入选择模式 */
  const enterSelectionMode = (topicId?: string | null) => {
    if (topicId) {
      setActiveShareTopicId(topicId);
      setCurrentTopicId(topicId);
    }
    setSelectedMessageIds(new Set<string>());
    setIsSelectingMessages(true);
  };

  /** 切换单条消息的选中状态 */
  const handleToggleMessage = (msgId: string) => {
    setSelectedMessageIds((prev) => {
      const next = new Set<string>(prev);
      if (next.has(msgId)) next.delete(msgId);
      else next.add(msgId);
      return next;
    });
  };

  /** 全选当前话题的可导出消息（排除 tool 消息） */
  const handleSelectAll = () => {
    const topic = activeTopic();
    if (!topic) return;
    const ids = new Set<string>();
    for (const msg of topic.history) {
      if (msg.role !== 'tool' && msg.id) ids.add(msg.id);
    }
    setSelectedMessageIds(ids);
  };

  /** 取消选择，退出选择模式 */
  const handleCancelSelection = () => {
    setIsSelectingMessages(false);
    setSelectedMessageIds(new Set<string>());
    setActiveShareTopicId(null);
  };

  /** 确认选择，打开分享弹窗 */
  const handleConfirmSelection = () => {
    setIsSelectingMessages(false);
    setShowShareModal(true);
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
      const stored = await invoke<StoredAttachment>('store_chat_attachment', { path: filePath });
      setPendingFiles(prev => prev.some(file => file.id === stored.id)
        ? prev
        : [...prev, {
            ...stored,
            name: fileName,
            type: isImg ? 'image' : 'text',
            previewUrl: isImg ? convertFileSrc(stored.storagePath) : undefined,
          }]
      );
    } catch (err) {
      alert(err);
    } finally {
      setIsProcessing(false);
    }
  };

  /**
   * 检查并总结对话历史（Token-based）
   * 当会话累计 token 数超过模型上下文窗口的 75% 时，触发压缩。
   */
  const checkAndSummarize = async () => {
    const topic = activeTopic();
    const currentMdl = selectedModel();
    if (!topic || !currentMdl) return;

    // 获取当前模型的上下文窗口大小
    let maxContext = 128_000; // 默认 128K
    try {
      const { getCachedCatalog } = await import('../../core/utils/models');
      const cat = getCachedCatalog();
      if (cat) {
        const modelMeta = cat.models?.find((m: any) => m.id === currentMdl.model_id);
        if (modelMeta?.contextWindow && modelMeta.contextWindow > 0) {
          maxContext = modelMeta.contextWindow;
        }
      }
    } catch {}

    // 统计实际上下文占用（使用最后一条 assistant 消息的 contextTokens，不跨消息累加）
    let totalUsed = 0;
    // 找到最后一条 assistant 消息，取其上下文峰值
    for (let i = topic.history.length - 1; i >= 0; i--) {
      const msg = topic.history[i];
      if (msg.role === 'assistant') {
        totalUsed = (msg.contextTokens || msg.inputTokens || 0) + (msg.outputTokens || 0);
        break;
      }
    }
    // 补充 user 消息的粗略估算（最后一条 assistant 未覆盖的部分）
    for (const msg of topic.history) {
      if (msg.role === 'user') {
        const text = typeof msg.content === 'string' ? msg.content : (msg.displayText || '');
        totalUsed += Math.ceil(text.length / 4);
      }
    }

    const usageRatio = maxContext > 0 ? totalUsed / maxContext : 0;
    if (usageRatio < 0.75 || topic.history.length <= 10) return;

    console.log(`[Context] 触发压缩: ${(usageRatio * 100).toFixed(0)}% (${totalUsed}/${maxContext} tokens, ${topic.history.length} 条消息)`);

    const keepCount = Math.max(4, Math.floor(topic.history.length * 0.3));
    const summarizeCount = topic.history.length - keepCount;
    const messagesToSummarize = topic.history.slice(0, summarizeCount);

    try {
      const newSummarySnippet = await invoke<string>('summarize_history', {
        apiUrl: currentMdl.api_url,
        apiKey: currentMdl.api_key,
        model: currentMdl.model_id,
        messages: messagesToSummarize
      });
      const latestTopic = activeTopic();
      if (!latestTopic) return;
      const updatedHistory = latestTopic.history.slice(summarizeCount);
      const combinedSummary = latestTopic.summary
        ? `[历史背景]: ${latestTopic.summary}\n[近期增补]: ${newSummarySnippet}`
        : newSummarySnippet;
      setDatas('assistants', a => a.id === currentAssistantId(), 'topics', t => t.id === latestTopic.id, {
        history: updatedHistory,
        summary: combinedSummary
      });
      await saveSingleAssistantToBackend(currentAssistantId()!);
      console.log('[Context] 压缩完成');
    } catch (e) {
      console.error("生成总结失败:", e);
    }
  };

  /**
   * 启发式后备方案：从历史中提取一段文本作为标题。
   * 用于 LLM 标题生成失败时保证至少能给出有意义的命名。
   * 取第一条 user 消息的前 12 个字符。
   */
  const fallbackTitle = (history: Message[]): string => {
    const firstUser = history.find(m => m.role === 'user');
    if (!firstUser) return '';
    let text: string;
    if (typeof firstUser.content === 'string') {
      text = firstUser.content;
    } else if (firstUser.displayText) {
      text = firstUser.displayText;
    } else {
      // 多模态：尝试提取 text 片段
      try {
        text = (firstUser.content as any[])
          .filter((p: any) => p?.type === 'text' && p?.text)
          .map((p: any) => p.text)
          .join(' ');
      } catch {
        text = '';
      }
    }
    // 去掉换行避免标题跨行
    const cleaned = text.trim().replace(/\s+/g, ' ');
    return cleaned.length > 12 ? cleaned.slice(0, 12) : cleaned;
  };

  /**
   * 为"非默认话题"在首次对话后自动重命名。
   * 规则：
   *   1. 跳过默认话题（每个助手的话题列表第一项）
   *   2. 跳过已重命名过的话题（topic.renamed === true）
   *   3. 跳过 AI 回复为空/为错误的情况
   *   4. 优先调用 LLM 生成标题；LLM 失败时回退到启发式（取首条 user 消息前 12 字）
   *   5. 成功后更新 topic.name 并置 renamed=true，持久化到后端
   *   6. 用户切换到其他话题时通过 activeTitleGen 取消未完成的回调处理
   */
  const checkAndRename = async (eventAsstId: string, eventTopicId: string) => {
    // 取消之前未完成的标题生成任务（用户切换话题或触发了新的重命名）
    if (activeTitleGen) {
      activeTitleGen.cancelled = true;
      activeTitleGen = null;
    }
    const task = { topicId: eventTopicId, cancelled: false };
    activeTitleGen = task;

    const asst = datas.assistants.find((a: any) => a.id === eventAsstId);
    if (!asst) { if (activeTitleGen === task) activeTitleGen = null; return; }
    // 必须用事件中的 topicId 重新定位（用户可能已切换话题）
    const topic = (asst.topics as Topic[]).find((t: Topic) => t.id === eventTopicId);
    if (!topic) { if (activeTitleGen === task) activeTitleGen = null; return; }

    // 1. 默认话题不重命名
    if (asst.topics[0]?.id === topic.id) { if (activeTitleGen === task) activeTitleGen = null; return; }
    // 2. 已经重命名过的话题不再重命名
    if (topic.renamed) { if (activeTitleGen === task) activeTitleGen = null; return; }
    // 3. 没有可参考的对话内容
    if (topic.history.length < 2) { if (activeTitleGen === task) activeTitleGen = null; return; }

    // 4. 检查最后一条 AI 消息是否有效（避免错误响应触发重命名）
    const lastMsg = topic.history[topic.history.length - 1];
    if (lastMsg?.role !== 'assistant') { if (activeTitleGen === task) activeTitleGen = null; return; }
    const lastContent = typeof lastMsg.content === 'string' ? lastMsg.content.trim() : '';
    if (lastContent.length < 2 || lastContent.startsWith('[Error:')) {
      if (activeTitleGen === task) activeTitleGen = null; return;
    }

    const currentMdl = selectedModel();
    if (!currentMdl) { if (activeTitleGen === task) activeTitleGen = null; return; }

    // 截断到前 2 条消息：业界标准做法（user + assistant 即可概括主题，省 token）
    const sample = topic.history.slice(0, 2);

    // 第一阶段：尝试 LLM 生成
    let newName: string | null = null;
    let llmError: unknown = null;
    try {
      const result = await invoke<string>('generate_topic_title', {
        apiUrl: currentMdl.api_url,
        apiKey: currentMdl.api_key,
        model: currentMdl.model_id,
        messages: sample
      });
      // 用户中途切换了话题或触发了新任务：放弃本次结果
      if (task.cancelled) return;
      if (result && result.trim()) {
        newName = result.trim();
      }
    } catch (e) {
      if (task.cancelled) return;
      llmError = e;
    }

    // 用户在 LLM 调用期间切换了话题：放弃处理
    if (task.cancelled) return;

    // 第二阶段：LLM 失败时用启发式后备
    if (!newName) {
      newName = fallbackTitle(topic.history);
      if (newName) {
        console.warn('LLM 标题生成失败，使用启发式后备:', llmError);
      }
    }

    // 二次校验：状态可能在异步期间被改变
    const latestAsst = datas.assistants.find((a: any) => a.id === eventAsstId);
    const latestTopic = latestAsst?.topics.find((t: Topic) => t.id === eventTopicId);
    if (!latestAsst || !latestTopic) { if (activeTitleGen === task) activeTitleGen = null; return; }
    if (latestAsst.topics[0]?.id === latestTopic.id) { if (activeTitleGen === task) activeTitleGen = null; return; }
    if (latestTopic.renamed) { if (activeTitleGen === task) activeTitleGen = null; return; }

    // 实在拿不到任何名字：仅标记为已重命名，避免下次再尝试
    if (!newName) {
      setDatas('assistants', a => a.id === eventAsstId, 'topics', t => t.id === eventTopicId, 'renamed', true);
      await saveSingleAssistantToBackend(eventAsstId);
      if (activeTitleGen === task) activeTitleGen = null;
      console.warn('自动重命名失败，无可用后备:', llmError);
      return;
    }

    // 与旧名相同则视为无效，但仍标记为已重命名
    if (newName === latestTopic.name) {
      setDatas('assistants', a => a.id === eventAsstId, 'topics', t => t.id === eventTopicId, 'renamed', true);
      await saveSingleAssistantToBackend(eventAsstId);
      if (activeTitleGen === task) activeTitleGen = null;
      return;
    }

    setDatas('assistants', a => a.id === eventAsstId, 'topics', t => t.id === eventTopicId, {
      name: newName,
      renamed: true
    });
    await saveSingleAssistantToBackend(eventAsstId);
    if (activeTitleGen === task) activeTitleGen = null;
    console.log(`话题已自动重命名: ${latestTopic.name} → ${newName}`);
  };

  /** 中断/取消导致的工具调用占位结果文本（标记给模型看，保证上下文可理解） */
  const TOOL_INTERRUPTED = '[Interrupted by user]';

  /** 截断过长的工具返回内容，防止 LLM 上下文膨胀。完整内容保留在 agentSteps 中供用户查看。 */
  const truncateToolResult = (content: string): string => {
    const MAX_LEN = 10000;
    if (content.length <= MAX_LEN) return content;
    return content.slice(0, MAX_LEN) + `\n\n... [已截断: 共${content.length}字符]`;
  };

  /**
   * 把一条历史消息展开为 API 载荷消息（可能 1 条或多条：assistant + 其 tool 结果）。
   *
   * 关键防御：state 停留在 'calling' 的孤儿 toolCall（用户中断 / 应用被杀 / 旧版本遗留），
   * 就地合成为中断错误的 tool 结果消息，保证每个 tool_calls 都有配对的 role:tool 响应。
   * 否则严格校验的 API 会返回 400（missing field tool_call_id / insufficient tool messages）。
   */
  function buildApiMessages(m: any): any[] {
    const obj: any = { role: m.role, content: m.content };
    if (m.toolCallId) obj.toolCallId = m.toolCallId;
    if (m.name) obj.name = m.name;
    if (!m.toolCalls || m.toolCalls.length === 0) return [obj];

    // 缺 id 的条目无法配对，直接剔除（避免 JSON.stringify 丢弃 undefined 字段导致 400）
    const validCalls = m.toolCalls.filter((tc: any) => tc.id != null);
    if (validCalls.length === 0) return [obj];

    obj.toolCalls = validCalls.map((tc: any) => ({
      id: tc.id,
      type: tc.type || 'function',
      function: { name: tc.function?.name, arguments: tc.function?.arguments },
    }));
    const toolMsgs = validCalls.map((tc: any) => {
      // 孤儿（calling / 无状态）：视为中断，合成错误结果
      const interrupted = tc.state !== 'success' && tc.state !== 'error';
      const raw = interrupted
        ? TOOL_INTERRUPTED
        : tc.state === 'error'
          ? (tc.error ?? tc.content ?? '[Tool Error]')
          : (tc.content ?? '');
      return {
        role: 'tool' as const,
        toolCallId: tc.id,
        name: tc.function?.name,
        content: truncateToolResult(raw),
      };
    });
    return [obj, ...toolMsgs];
  }

  /**
   * 确保 messagesForAI 中的所有 assistant(tool_calls) 都有对应的 role:tool 消息。
   * 如果缺少某个 tool_call_id 的 tool 响应，自动补全占位消息，
   * 防止 API 400 "insufficient tool messages" 错误。
   */
  function ensureToolMessagesComplete(msgs: any[]): any[] {
    const result: any[] = [];
    let pendingToolCallIds: string[] = [];

    // 补全 pending 中所有未配对 toolCallId 的占位 tool 消息
    const flushPending = (target: any[]) => {
      for (const id of pendingToolCallIds) {
        console.warn(`[ensureToolMessages] 补全缺失的 tool 响应: ${id}`);
        target.push({
          role: 'tool' as const,
          toolCallId: id,
          name: '__pending__',
          content: '[System: tool result pending]',
        });
      }
      pendingToolCallIds = [];
    };

    for (const msg of msgs) {
      // 遇到新的 assistant(toolCalls) 时，先补全前一组未配对的 toolCallId，
      // 避免连续多个 tool 调用回合时前一组 pending 被直接覆盖丢失
      if (msg.role === 'assistant' && msg.toolCalls?.length && pendingToolCallIds.length > 0) {
        flushPending(result);
      }

      result.push(msg);

      if (msg.role === 'assistant' && msg.toolCalls?.length) {
        // 收集这条 assistant 消息声明的所有 toolCallId
        pendingToolCallIds = msg.toolCalls.map((tc: any) => tc.id);
      } else if (msg.role === 'tool' && msg.toolCallId) {
        // 匹配到对应的 tool 响应，移除
        const idx = pendingToolCallIds.indexOf(msg.toolCallId);
        if (idx !== -1) pendingToolCallIds.splice(idx, 1);
      }

      // 遇到 user 或 assistant(无 toolCalls) 消息时，如果还有未匹配的 toolCallId，
      // 说明之前 assistant 的 toolCalls 缺少足够 tool 响应——补全占位消息
      if ((msg.role === 'user' || (msg.role === 'assistant' && !msg.toolCalls?.length)) && pendingToolCallIds.length > 0) {
        flushPending(result);
      }
    }

    // 收尾：如果数组结束时仍有未匹配的 toolCallId，补全占位消息
    if (pendingToolCallIds.length > 0) flushPending(result);

    return result;
  }

  /**
   * 斜杠命令 — promptBody 类型（/review、/explain 等）
   * 聊天显示原始命令文本，后台发送解析后的 promptBody 给 LLM
   */
  const handleSlashPromptMessage = async (resolved: ReturnType<typeof resolveSlashCommand>) => {
    if (!resolved) return;
    const asstId = currentAssistantId();
    const topicId = currentTopicId();
    if (!asstId || !topicId) return;

    const currentMdl = selectedModel();
    const currentAsst = currentAssistant();
    const currentTopic = activeTopic();
    if (!currentMdl || !currentAsst || !currentTopic) return;

    const displayText = resolved.command.label + (resolved.args ? ` ${resolved.args}` : '');
    let content = resolved.resolvedBody || resolved.command.label;

    // 无参数时自动注入当前项目路径，给 LLM 提供上下文
    if (!resolved.args) {
      const project = currentProject();
      if (project) {
        content = `${content}\n\n当前项目路径: ${project.path}`;
      }
    }

    const newUserMsg = {
      id: crypto.randomUUID(),
      role: 'user' as const,
      content,
      displayText,
    };

    // 构建完整消息数组（与普通发送流程一致）
    const reasoningPrompt = (() => {
      switch (reasoningLevel()) {
        case 'low':    return '在回答前先进行简单思考. 用 <think> 标签包裹你的推理过程, 再给出最终回答. 控制思考长度, 简单问题不要过度展开.';
        case 'medium': return '在回答前先进行中等深度的思考. 用 <think> 标签包裹你的推理过程 (分析问题、拆解步骤、对比方案), 再给出最终回答.';
        case 'high':   return '在回答前进行深入的多步推理. 必须在 <think> 标签中详细分析问题、列出前提、考虑边界情况、对比多种方案, 再给出严谨的最终回答. 思考越充分越好.';
        default:       return null;
      }
    })();

    const agentMode = currentAsst?.agentMode || 'off';
    const pid = currentProjectId();
    const projectInfo = pid && currentProject() ? { path: currentProject()!.path, name: currentProject()!.name } : null;
    const agentPromptContent = agentMode !== 'off' && projectInfo
      ? buildAgentSystemPrompt(agentMode as any, projectInfo)
      : null;
    const agentSystemPrompt = agentPromptContent
      ? [{ role: 'system' as const, content: agentPromptContent }]
      : [];

    let messagesForAI: any[] = [
      { role: 'system', content: currentAsst.prompt },
      ...resolveAssistantSkills(currentAsst).map(skill => ({
        role: 'system',
        content: `[Skill: ${skill.name}]\n${skill.content}`,
      })),
      ...agentSystemPrompt,
      ...(reasoningPrompt ? [{ role: 'system', content: reasoningPrompt }] : []),
      ...(webSearchEnabled() ? [{ role: 'system', content: '你可以使用 web_fetch(url) 获取网页内容（仅 HTTPS），以及 web_search(query, count?) 通过 DuckDuckGo 搜索网页。如有需要获取最新信息，请直接调用这些工具。' }] : []),
      ...(currentTopic.summary ? [{
        role: 'system',
        content: `这是之前对话的摘要记忆，请结合这些上下文回答：\n${currentTopic.summary}`
      }] : []),
      ...currentTopic.history.flatMap((m: any) => buildApiMessages(m)),
      { role: 'user', content: newUserMsg.content }
    ];
    messagesForAI = ensureToolMessagesComplete(messagesForAI);

    // 持久化用户消息
    try {
      await invoke('append_message', { topicId, message: newUserMsg });
    } catch (err) {
      alert(`保存消息失败: ${err}`);
      return;
    }

    // 更新本地状态
    setDatas('assistants', (a: any) => a.id === asstId,
      'topics', (t: any) => t.id === topicId,
      'history', (h: any[]) => [...h, newUserMsg]);

    // 调用 LLM
    try {
      setIsThinking(true);
      await invoke('run_agent_turn', {
        apiUrl: currentMdl.api_url,
        apiKey: currentMdl.api_key,
        model: currentMdl.model_id,
        assistantId: asstId,
        topicId: topicId,
        messages: messagesForAI,
        mcpServerIds: currentAsst?.mcpServerIds ?? [],
        agentMode: agentMode,
        projectId: currentProjectId() ?? null,
        webSearchEnabled: webSearchEnabled(),
      });
    } catch (err) {
      alert(err);
      setIsThinking(false);
      setTypingIndex(null);
    }
  };

  /**
   * 斜杠命令 — 纯操作类型（/compact、/clear、/search、/settings、/help）
   * 聊天显示命令 + 执行 handler + 简短反馈
   */
  const handleSlashActionMessage = async (resolved: ReturnType<typeof resolveSlashCommand>) => {
    if (!resolved) return;
    const asstId = currentAssistantId();
    const topicId = currentTopicId();
    if (!asstId || !topicId) return;

    const cmd = resolved.command;
    const displayText = cmd.label;

    // 添加用户消息
    setDatas('assistants', (a: any) => a.id === asstId,
      'topics', (t: any) => t.id === topicId,
      'history', (h: any[]) => [...h, {
        id: crypto.randomUUID(),
        role: 'user' as const,
        content: displayText,
        displayText,
      }]);

    // 执行 handler
    await cmd.handler();

    // 根据命令类型给出反馈消息
    let feedback = '';
    if (cmd.id === 'slash-compact') {
      feedback = '✓ /compact — 上下文已压缩';
    } else if (cmd.id === 'slash-clear') {
      // /clear handler 已清空历史，无需额外反馈
      return;
    } else if (cmd.id === 'slash-search') {
      feedback = webSearchEnabled() ? '✓ 联网搜索已开启' : '✓ 联网搜索已关闭';
    } else if (cmd.id === 'slash-help') {
      // /help handler 已添加帮助信息，无需额外反馈
      return;
    } else if (cmd.id === 'slash-settings') {
      // /settings handler 已跳转页面
      return;
    } else {
      feedback = `✓ ${cmd.label} — 已执行`;
    }

    // 添加反馈消息
    setDatas('assistants', (a: any) => a.id === asstId,
      'topics', (t: any) => t.id === topicId,
      'history', (h: any[]) => [...h, {
        id: crypto.randomUUID(),
        role: 'assistant' as const,
        content: feedback,
        displayText: feedback,
      }]);
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

    // ---- 斜杠命令拦截 ----
    if (userInput.startsWith('/')) {
      const resolved = resolveSlashCommand(userInput);
      if (resolved) {
        if (resolved.resolvedBody) {
          handleSlashPromptMessage(resolved);
        } else {
          handleSlashActionMessage(resolved);
        }
        setInputMessage('');
        setPendingFiles([]);
        return;
      }
      // 未识别的斜杠命令 — 给出反馈，阻止发送给 LLM
      const unknownAsstId = currentAssistantId();
      const unknownTopicId = currentTopicId();
      if (unknownAsstId && unknownTopicId) {
        setDatas('assistants', (a: any) => a.id === unknownAsstId,
          'topics', (t: any) => t.id === unknownTopicId,
          'history', (h: any[]) => [...h, {
            id: crypto.randomUUID(),
            role: 'user' as const,
            content: userInput,
            displayText: userInput,
          }, {
            id: crypto.randomUUID(),
            role: 'assistant' as const,
            content: `✗ 未知命令: \`${userInput}\`\n\n输入 **/help** 查看所有可用命令。`,
          }]);
      }
      setInputMessage('');
      setPendingFiles([]);
      return;
    }

    const asstId = currentAssistantId();
    const topicId = currentTopicId();
    if (!asstId || !topicId) return;

    const newUserMsg = {
      id: crypto.randomUUID(),
      role: 'user' as const,
      content: userInput,
      displayFiles: files.map(f => ({
        id: f.id,
        name: f.name,
        mimeType: f.mimeType,
        size: f.size,
      })),
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

    const agentMode = currentAsst?.agentMode || 'off';
    const pid = currentProjectId();
    const projectInfo = pid && currentProject() ? { path: currentProject()!.path, name: currentProject()!.name } : null;
    const agentPromptContent = agentMode !== 'off' && projectInfo
      ? buildAgentSystemPrompt(agentMode as any, projectInfo)
      : null;
    const agentSystemPrompt = agentPromptContent
      ? [{ role: 'system' as const, content: agentPromptContent }]
      : [];
    let messagesForAI: any[] = [
        { role: 'system', content: currentAsst.prompt },
        ...resolveAssistantSkills(currentAsst).map(skill => ({
          role: 'system',
          content: `[Skill: ${skill.name}]\n${skill.content}`,
        })),
        ...agentSystemPrompt,
        ...(reasoningPrompt ? [{ role: 'system', content: reasoningPrompt }] : []),
      ...(webSearchEnabled() ? [{ role: 'system', content: '你可以使用 web_fetch(url) 获取网页内容（仅 HTTPS），以及 web_search(query, count?) 通过 DuckDuckGo 搜索网页。如有需要获取最新信息，请直接调用这些工具。' }] : []),
        ...(currentTopic.summary ? [{
          role: 'system',
          content: `这是之前对话的摘要记忆，请结合这些上下文回答：\n${currentTopic.summary}`
        }] : []),
      ...currentTopic.history.flatMap((m: any) => buildApiMessages(m)),
        { role: 'user', content: newUserMsg.content }
      ];
      // 安全网：确保每个 assistant(tool_calls) 都有对应的 role:tool 消息
      messagesForAI = ensureToolMessagesComplete(messagesForAI);

    const lastMsg = messagesForAI[messagesForAI.length - 1];
    if (lastMsg.role !== 'user') {
      console.error("错误：发送给 API 的最后一条消息不是 User!", lastMsg);
      return;
    }

    // 先持久化用户消息和附件关联，避免流式请求期间退出或重复上传导致附件成为孤儿。
    try {
      await invoke('append_message', {
        topicId,
        message: newUserMsg,
      });
    } catch (err) {
      alert(`保存消息失败: ${err}`);
      return;
    }

    // 更新本地 Store：添加用户消息，聊天模式下同时创建空 assistant 占位消息
    // （项目/Agent 模式下占位消息由 llm-round-start 事件创建）
    if (isChatMode()) {
      setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId, 'history', h => [
        ...h,
        newUserMsg,
        {
          id: crypto.randomUUID(),
          role: 'assistant' as const,
          content: '',
          modelId: currentMdl?.model_id,
          reasoning: '',
          agentStartTime: Date.now(),
          agentSteps: [] as any[],
        },
      ]);
    } else {
      setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId, 'history', h => [
        ...h,
        newUserMsg,
      ]);
    }

    // 清空输入状态和文件列表，设置生成中状态
    setInputMessage("");
    setPendingFiles([]);
    setIsThinking(true);

    // 设置打字机索引（聊天模式下刚创建的 assistant 占位消息位置）
    if (isChatMode()) {
      const asst = datas.assistants.find(a => a.id === asstId);
      const topic = asst?.topics.find((t: Topic) => t.id === topicId);
      if (topic) setTypingIndex(topic.history.length - 1);
    }

      try {
        // 聊天模式：纯对话，直接调用 call_llm_stream（无 agent 循环、无工具调用）
        // 项目模式/Agent 模式：调用 run_agent_turn（含工具调用、子智能体等）
        if (isChatMode()) {
          await invoke('call_llm_stream', {
            apiUrl: currentMdl.api_url,
            apiKey: currentMdl.api_key,
            model: currentMdl.model_id,
            assistantId: asstId,
            topicId: topicId,
            messages: messagesForAI,
          });
        } else {
          // 解析 per-profile 模型覆盖（仅 agent 模式需要）
          const resolvedOverrides: Array<{ profileId: string; modelId: string; apiUrl: string; apiKey: string }> = [];
          for (const [profileId, mKey] of Object.entries(profileModelOverrides())) {
            if (!mKey) continue;
            const mdl = allAvailableModels().find(m => modelKey(m) === mKey);
            if (mdl) {
              resolvedOverrides.push({
                profileId,
                modelId: mKey,
                apiUrl: mdl.api_url,
                apiKey: mdl.api_key,
              });
            }
          }

          await invoke('run_agent_turn', {
            apiUrl: currentMdl.api_url,
            apiKey: currentMdl.api_key,
            model: currentMdl.model_id,
            assistantId: asstId,
            topicId: topicId,
            messages: messagesForAI,
            mcpServerIds: currentAsst?.mcpServerIds ?? [],
            agentMode: agentMode,
            projectId: currentProjectId() ?? null,
            webSearchEnabled: webSearchEnabled(),
            profileModelOverrides: resolvedOverrides,
            customSubagentProfiles: customSubagentProfiles(),
          });
        }

    } catch (err) {
      alert(err);
      setIsThinking(false);
      setTypingIndex(null);
    }
  };

  /**
   * 停止当前的AI生成过程
   */
  const handleStopGeneration = async () => {
    try {
      await invoke('stop_llm_stream', {
        assistantId: currentAssistantId(),
        topicId: currentTopicId()
      });
    } catch (err) {
      console.error('stop_llm_stream 失败:', err);
    }
    setPendingApprovals([]);
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

  /** 切换到聊天模式 */
  const switchToChat = () => {
    setCurrentAssistantId(DEFAULT_ASST_ID);
    const asst = datas.assistants.find(a => a.id === DEFAULT_ASST_ID);
    if (asst?.topics?.length) {
      setCurrentTopicId(asst.topics[0].id);
    }
  };

  /** 切换到指定项目 */
  const switchToProject = async (projectId: string) => {
    try {
      const asstId = await ensureProjectAssistant(projectId);
      setCurrentAssistantId(asstId);
      const asst = datas.assistants.find(a => a.id === asstId);
      if (asst?.topics?.length) {
        setCurrentTopicId(asst.topics[0].id);
      }
    } catch (e) {
      console.error('切换项目失败:', e);
    }
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
    const navigate = useNavigate();

    // ---- 注册快捷键命令 ----
    const cmdToggleLeft = registerCommand({
      id: 'toggle-left-sidebar',
      label: '切换左侧边栏',
      description: '显示或隐藏助手列表侧边栏',
      category: 'navigation',
      defaultKeys: 'Ctrl+B',
      handler: () => {
        const newState = !isLeftCollapsed();
        setIsLeftCollapsed(newState);
        localStorage.setItem('left-collapsed', String(newState));
        if (!newState && leftPanelWidth() < 5) setLeftPanelWidth(18);
      },
    });

    const cmdToggleRight = registerCommand({
      id: 'toggle-right-sidebar',
      label: '切换右侧边栏',
      description: '显示或隐藏话题列表侧边栏',
      category: 'navigation',
      defaultKeys: 'Ctrl+Alt+B',
      handler: () => {
        const newState = !isRightCollapsed();
        setIsRightCollapsed(newState);
        localStorage.setItem('right-collapsed', String(newState));
        if (!newState && rightPanelWidth() < 5) setRightPanelWidth(18);
      },
    });

    const cmdNewTopic = registerCommand({
      id: 'new-topic',
      label: '新建话题',
      description: '在当前助手中创建新的话题',
      category: 'chat',
      defaultKeys: 'Ctrl+N',
      handler: () => { addTopic(); },
    });

    const cmdNewProject = registerCommand({
      id: 'new-project',
      label: '新建项目',
      description: '创建新的项目',
      category: 'chat',
      defaultKeys: 'Ctrl+Shift+N',
      handler: () => { setShowProjectCreateModal(true); },
    });

    const cmdStopGen = registerCommand({
      id: 'stop-generation',
      label: '停止生成',
      description: '停止当前 AI 回复的生成',
      category: 'chat',
      defaultKeys: 'Escape',
      handler: () => {
        if (isThinking()) handleStopGeneration();
      },
    });

    const cmdPrevAsst = registerCommand({
      id: 'prev-assistant',
      label: '上一个',
      description: '切换到上一个聊天或项目',
      category: 'sidebar',
      defaultKeys: 'Ctrl+[',
      handler: () => {
        // 构建顺序: [chat, ...projects]
        const chatIdx = 0;
        const projList = projects();
        const currentProjId = currentAssistant()?.projectId;
        if (!currentProjId) {
          // 当前是聊天模式，上一个 = 最后一个项目
          if (projList.length > 0) {
            const lastProj = projList[projList.length - 1];
            void switchToProject(lastProj.id);
          }
        } else {
          const idx = projList.findIndex(p => p.id === currentProjId);
          if (idx > 0) {
            void switchToProject(projList[idx - 1].id);
          } else {
            // 第一个项目，上一个 = 聊天
            switchToChat();
          }
        }
      },
    });

    const cmdNextAsst = registerCommand({
      id: 'next-assistant',
      label: '下一个',
      description: '切换到下一个聊天或项目',
      category: 'sidebar',
      defaultKeys: 'Ctrl+]',
      handler: () => {
        const projList = projects();
        const currentProjId = currentAssistant()?.projectId;
        if (!currentProjId) {
          // 当前是聊天模式，下一个 = 第一个项目
          if (projList.length > 0) {
            void switchToProject(projList[0].id);
          }
        } else {
          const idx = projList.findIndex(p => p.id === currentProjId);
          if (idx >= 0 && idx < projList.length - 1) {
            void switchToProject(projList[idx + 1].id);
          } else {
            // 最后一个项目，下一个 = 聊天
            switchToChat();
          }
        }
      },
    });

    const cmdPrevTopic = registerCommand({
      id: 'prev-topic',
      label: '上一个话题',
      description: '切换到上一个话题',
      category: 'sidebar',
      defaultKeys: 'Ctrl+Shift+[',
      handler: () => {
        const asst = datas.assistants.find((a: any) => a.id === currentAssistantId());
        if (!asst) return;
        const idx = asst.topics.findIndex((t: any) => t.id === currentTopicId());
        if (idx > 0) setCurrentTopicId(asst.topics[idx - 1].id);
      },
    });

    const cmdNextTopic = registerCommand({
      id: 'next-topic',
      label: '下一个话题',
      description: '切换到下一个话题',
      category: 'sidebar',
      defaultKeys: 'Ctrl+Shift+]',
      handler: () => {
        const asst = datas.assistants.find((a: any) => a.id === currentAssistantId());
        if (!asst) return;
        const idx = asst.topics.findIndex((t: any) => t.id === currentTopicId());
        if (idx >= 0 && idx < asst.topics.length - 1) setCurrentTopicId(asst.topics[idx + 1].id);
      },
    });

    const cmdGoSettings = registerCommand({
      id: 'go-to-settings',
      label: '打开设置',
      description: '导航到应用设置页面',
      category: 'navigation',
      defaultKeys: 'Ctrl+,',
      handler: () => { navigate('/settings/app'); },
    });

    const cmdGoChat = registerCommand({
      id: 'go-to-chat',
      label: '回到聊天',
      description: '导航到聊天页面',
      category: 'navigation',
      defaultKeys: 'Ctrl+1',
      handler: () => { navigate('/chat'); },
    });

    const cmdToggleSearch = registerCommand({
      id: 'toggle-web-search',
      label: '切换联网搜索',
      description: '开启或关闭联网搜索功能',
      category: 'chat',
      defaultKeys: 'Ctrl+Shift+S',
      handler: () => {
        const next = !webSearchEnabled();
        setWebSearchEnabled(next);
        localStorage.setItem('chat-web-search', String(next));
      },
    });

    const cmdCycleReasoning = registerCommand({
      id: 'cycle-reasoning',
      label: '切换推理强度',
      description: '循环切换推理深度：关闭 → 低 → 中 → 高',
      category: 'chat',
      defaultKeys: 'Ctrl+Shift+R',
      handler: () => {
        const levels: Array<'off' | 'low' | 'medium' | 'high'> = ['off', 'low', 'medium', 'high'];
        const current = reasoningLevel();
        const idx = levels.indexOf(current);
        const next = levels[(idx + 1) % levels.length];
        setReasoningLevel(next);
        localStorage.setItem('chat-reasoning-level', next);
      },
    });

    const cmdCycleAgent = registerCommand({
      id: 'cycle-agent-mode',
      label: '切换 Agent 模式',
      description: '循环切换 Agent 执行模式：对话 → 普通 → 自动 → 计划',
      category: 'chat',
      defaultKeys: 'Ctrl+Shift+M',
      handler: async () => {
        const id = currentAssistantId();
        if (!id) return;
        const asst = datas.assistants.find((a: any) => a.id === id);
        const current = (asst?.agentMode || 'off') as string;
        const modes = ['off', 'normal', 'auto', 'plan'];
        const idx = modes.indexOf(current);
        const next = modes[(idx + 1) % modes.length];
        setDatas('assistants', (a: any) => a.id === id, 'agentMode', next);
        await saveSingleAssistantToBackend(id);
      },
    });

    // ---- 斜杠命令处理器（覆盖 shortcuts.ts 中的 no-op） ----
    const cmdSlashClear = registerCommand({
      id: 'slash-clear',
      handler: () => {
        const asstId = currentAssistantId();
        const topicId = currentTopicId();
        if (!asstId || !topicId) return;
        if (!confirm('确定要清空当前对话历史吗？此操作不可撤销。')) return;
        setDatas('assistants', (a: any) => a.id === asstId,
          'topics', (t: any) => t.id === topicId,
          'history', []);
        saveSingleAssistantToBackend(asstId);
      },
    });

    const cmdSlashCompact = registerCommand({
      id: 'slash-compact',
      handler: async () => {
        setIsThinking(true);
        try {
          await checkAndSummarize();
        } finally {
          setIsThinking(false);
        }
      },
    });

    const cmdSlashSearch = registerCommand({
      id: 'slash-search',
      handler: () => {
        const next = !webSearchEnabled();
        setWebSearchEnabled(next);
        localStorage.setItem('chat-web-search', String(next));
      },
    });

    const cmdSlashSettings = registerCommand({
      id: 'slash-settings',
      handler: () => { navigate('/settings/app'); },
    });

    const cmdSlashHelp = registerCommand({
      id: 'slash-help',
      handler: () => {
        const asstId = currentAssistantId();
        const topicId = currentTopicId();
        if (!asstId || !topicId) return;
        const allSlash = getSlashCommands();
        const lines = allSlash.map(c => `- **${c.label}** — ${c.description}`);
        const helpText = `## 可用斜杠命令\n\n${lines.join('\n')}`;
        setDatas('assistants', (a: any) => a.id === asstId,
          'topics', (t: any) => t.id === topicId,
          'history', (h: any[]) => [...h, {
            id: crypto.randomUUID(),
            role: 'assistant' as const,
            content: helpText,
          }]);
      },
    });

    // ---- 原有初始化逻辑 ----
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
      // 新一轮 LLM 调用开始：多轮 Agent 工作时复用同一条 assistant 消息
      listen<any>('llm-round-start', (e) => {
        const { assistant_id, topic_id, round } = e.payload;

        // 后续轮次：关闭上一轮的步骤，准备新轮次（不再整体压扁到 interimContent）
        if (round > 1) {
          const asst = datas.assistants.find(a => a.id === assistant_id);
          const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
          if (topic) {
            const lastIdx = topic.history.length - 1;
            const lastMsg = topic.history[lastIdx];
            if (lastMsg?.role === 'assistant') {
              // 关闭当前运行的步骤
              const now = Date.now();
              setDatas('assistants', a => a.id === assistant_id,
                'topics', t => t.id === topic_id,
                'history', lastIdx, 'agentSteps', (steps: any[] = []) => {
                  if (steps.length === 0) return steps;
                  const last = steps[steps.length - 1];
                  if (last.status !== 'running') return steps;
                  return [...steps.slice(0, -1), { ...last, status: 'complete', duration: now - last.timestamp }];
                });
              // 重置 content / reasoning，准备接收新轮次输出
              setDatas('assistants', a => a.id === assistant_id,
                'topics', t => t.id === topic_id,
                'history', lastIdx, {
                  content: '',
                  reasoning: '',
                });
              // 打字机索引保持不变（仍指向同一条消息）
              return;
            }
          }
        }

        // 第一轮：创建新 assistant 占位消息
        const currentMdl = selectedModel();
        setDatas('assistants', a => a.id === assistant_id, 'topics', t => t.id === topic_id,
          'history', h => [...h, {
            id: crypto.randomUUID(),
            role: 'assistant' as const,
            content: "",
            modelId: currentMdl?.model_id,
            reasoning: '',
            agentStartTime: Date.now(),
            agentSteps: [],
          }]);
        // 设置打字机索引为新 assistant 消息位置
        const asst = datas.assistants.find(a => a.id === assistant_id);
        const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
        if (topic) setTypingIndex(topic.history.length - 1);
      }),
      listen<any>('llm-chunk', async (e) => {
        const { assistant_id, topic_id, content, done, error, input_tokens, output_tokens, context_tokens } = e.payload;
        if (done) {
          // 刷新可能残余的 rAF 批量内容
          if (streamBatchRAF !== undefined) {
            clearTimeout(streamBatchRAF);
            streamBatchRAF = undefined;
          }
          if (streamBatch) {
            const batch = streamBatch;
            streamBatch = null;
            setDatas('assistants', a => a.id === batch.assistant_id,
              'topics', t => t.id === batch.topic_id,
              'history', batch.lastIdx, 'content', (old: string) => old + batch.content);
          }
          // 整轮真正结束（后端 run_agent_turn epilogue 唯一发出 done）
          if (error) {
            // 错误：追加错误文本到最后一条 assistant 消息
            const asst = datas.assistants.find(a => a.id === assistant_id);
            const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
            if (topic) {
              const lastIdx = topic.history.length - 1;
              if (lastIdx >= 0) {
                setDatas('assistants', a => a.id === assistant_id,
                  'topics', t => t.id === topic_id,
                  'history', lastIdx, 'content', (old: string) => (old ?? '') + content);
              }
            }
          }
          // 关闭未完结状态：最后一个 running 步骤、以及中断产生的孤儿 toolCalls。
          // 用户点停止时后端 cancel 直接 break，不会为在途工具发 llm-tool-result，
          // 导致 msg.toolCalls 中残留 state==='calling' 的条目。下一次请求重建 API 载荷时，
          // 这些条目会被 filter 跳过，assistant.tool_calls 失去配对的 role:tool 消息，
          // 触发 API 400（missing field tool_call_id）。这里统一标记为 error 并写明中断原因，
          // 重建逻辑会自动为它们生成配对的 tool 消息，模型也能感知“上次操作被中断”。
          const finishTime = Date.now();
          setDatas('assistants', (a: any) => a.id === assistant_id,
            'topics', (t: any) => t.id === topic_id,
            'history', (h: any[]) => {
              const lastIdx = h.length - 1;
              if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
              const msg = h[lastIdx];
              const steps: any[] = msg.agentSteps || [];
              const toolCalls: any[] = msg.toolCalls || [];
              const updatedMsg: any = { ...msg };
              // 保存 token 用量
              if (input_tokens != null) updatedMsg.inputTokens = input_tokens;
              if (output_tokens != null) updatedMsg.outputTokens = output_tokens;
              if (context_tokens != null) updatedMsg.contextTokens = context_tokens;
              // 孤儿 toolCalls：仍为 calling 的条目标记为中断错误
              if (toolCalls.some((tc: any) => tc.state === 'calling')) {
                updatedMsg.toolCalls = toolCalls.map((tc: any) =>
                  tc.state === 'calling' ? { ...tc, state: 'error', error: TOOL_INTERRUPTED } : tc
                );
              }
              if (steps.length === 0) {
                return [...h.slice(0, lastIdx), updatedMsg];
              }
              return [...h.slice(0, lastIdx), {
                ...updatedMsg,
                agentSteps: steps.map((s: any, i: number) => {
                  if (s.status !== 'running') return s;
                  // 中断的 tool_call 步骤与其 toolCall 状态保持一致（标记 error）；
                  // 其余 running 步骤（含最后一个）按完成关闭
                  if (s.type === 'tool_call' && s.toolCall?.state === 'calling') {
                    return {
                      ...s,
                      status: 'error',
                      duration: finishTime - s.timestamp,
                      toolCall: { ...s.toolCall, state: 'error', error: TOOL_INTERRUPTED },
                    };
                  }
                  return i === steps.length - 1
                    ? { ...s, status: 'complete', duration: finishTime - s.timestamp }
                    : s;
                }),
              }];
            });
          setIsThinking(false);
          setTypingIndex(null);
          saveSingleAssistantToBackend(assistant_id);
          // 串行执行：先尝试自动重命名（非默认话题的首次对话），再触发历史压缩总结
          setTimeout(async () => {
            await checkAndRename(assistant_id, topic_id);
            await checkAndSummarize();
          }, 500);
          return;
        }

        // 流式数据追加：rAF 批量合并，避免逐 token 的打字机效果
        const asst = datas.assistants.find(a => a.id === assistant_id);
        const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
        if (topic) {
          const lastIdx = topic.history.length - 1;
          if (lastIdx >= 0) {
            if (!streamBatch) {
              streamBatch = { assistant_id, topic_id, content, agentStepsContent: content, lastIdx };
              streamBatchRAF = window.setTimeout(() => {
                const batch = streamBatch!;
                streamBatch = null;
                streamBatchRAF = undefined;
                // 一次性应用批量内容（50ms 窗口内累积的所有 token）
                setDatas('assistants', a => a.id === batch.assistant_id,
                  'topics', t => t.id === batch.topic_id,
                  'history', batch.lastIdx, 'content', (old: string) => old + batch.content);
                if (batch.agentStepsContent) {
                  const batchStepsContent = batch.agentStepsContent;
                  setDatas('assistants', a => a.id === batch.assistant_id,
                    'topics', t => t.id === batch.topic_id,
                    'history', batch.lastIdx, 'agentSteps', (steps: any[] = []) => {
                      const lastStep = steps[steps.length - 1];
                      if (lastStep && lastStep.type === 'content' && lastStep.status === 'running') {
                        return [...steps.slice(0, -1), { ...lastStep, contentText: (lastStep.contentText || '') + batchStepsContent }];
                      }
                      const now = Date.now();
                      const closed = lastStep && lastStep.status === 'running'
                        ? [...steps.slice(0, -1), { ...lastStep, status: 'complete', duration: now - lastStep.timestamp }]
                        : steps;
                      return [...closed, { id: crypto.randomUUID(), type: 'content', timestamp: now, status: 'running', contentText: batchStepsContent }];
                    });
                }
              }, 80) as unknown as number;
            } else {
              streamBatch.content += content;
              streamBatch.agentStepsContent += content;
            }
          }
        }
      }),
      // 思维链片段追加：原生 reasoning_content 流式累积到对应消息的 reasoning 字段
      // 同时构建 agentSteps 时间线中的 thinking 步骤
      listen<any>('llm-reasoning', (e) => {
        if (reasoningLevel() === 'off') return;
        const { assistant_id, topic_id, content } = e.payload;
        const asst = datas.assistants.find(a => a.id === assistant_id);
        const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
        if (topic) {
          const lastIdx = topic.history.length - 1;
          if (lastIdx >= 0) {
            // 追加到 reasoning 字段（保持向后兼容）
            setDatas('assistants', a => a.id === assistant_id,
              'topics', t => t.id === topic_id,
              'history', lastIdx, 'reasoning', (old: string) => (old ?? '') + content);
            // 构建 agentSteps 时间线：若最后一步不是 thinking 则新建，否则追加
            setDatas('assistants', a => a.id === assistant_id,
              'topics', t => t.id === topic_id,
              'history', lastIdx, 'agentSteps', (steps: any[] = []) => {
                const lastStep = steps[steps.length - 1];
                if (lastStep && lastStep.type === 'thinking' && lastStep.status === 'running') {
                  // 追加到当前 thinking 步骤
                  return [...steps.slice(0, -1), {
                    ...lastStep,
                    thinkingText: (lastStep.thinkingText || '') + content,
                  }];
                }
                // 关闭上一步（如果仍在运行），新建 thinking 步骤
                const now = Date.now();
                const closed = lastStep && lastStep.status === 'running'
                  ? [...steps.slice(0, -1), { ...lastStep, status: 'complete', duration: now - lastStep.timestamp }]
                  : steps;
                return [...closed, {
                  id: crypto.randomUUID(),
                  type: 'thinking',
                  timestamp: now,
                  status: 'running',
                  thinkingText: content,
                }];
              });
          }
        }
      }),
      // LLM 工具调用事件：展示"调用中"气泡并构建 agentSteps 时间线
      listen<any>('llm-tool-call', (e) => {
        const { assistant_id, topic_id, tool_call_id, name, arguments: argsJson } = e.payload;
        const newTc = { id: tool_call_id, type: 'function', function: { name, arguments: argsJson }, state: 'calling' };
        setDatas('assistants', (a: any) => a.id === assistant_id, 'topics', (t: Topic) => t.id === topic_id,
          'history', (h: any[]) => {
            const lastIdx = h.length - 1;
            if (lastIdx >= 0 && h[lastIdx]?.role === 'assistant') {
              const msg = h[lastIdx];
              // 保持 toolCalls 数组（消息重建用）
              const existing = msg.toolCalls || [];
              if (existing.find((tc: any) => tc.id === tool_call_id)) return h;
              const newToolCalls = [...existing, newTc];
              // 构建 agentSteps：关闭上一步，新建 tool_call 步骤
              const now = Date.now();
              const steps: any[] = msg.agentSteps || [];
              const lastStep = steps[steps.length - 1];
              const closed = lastStep && lastStep.status === 'running'
                ? [...steps.slice(0, -1), { ...lastStep, status: 'complete', duration: now - lastStep.timestamp }]
                : steps;
              const newSteps = [...closed, {
                id: crypto.randomUUID(),
                type: 'tool_call',
                timestamp: now,
                status: 'running',
                toolCall: newTc,
              }];
              return [
                ...h.slice(0, lastIdx),
                { ...msg, toolCalls: newToolCalls, agentSteps: newSteps },
              ];
            }
            return h;
          }
        );
      }),
      // 工具执行结果：更新 assistant 消息中对应 toolCall 的状态与 agentSteps 时间线
      listen<any>('llm-tool-result', (e) => {
        const { assistant_id, topic_id, tool_call_id, content, result, is_error, file_changes } = e.payload;
        setDatas('assistants', (a: any) => a.id === assistant_id, 'topics', (t: Topic) => t.id === topic_id,
          'history', (h: any[]) => h.map((m: any) => {
            if (m.role !== 'assistant' || !m.toolCalls) return m;
            const newToolCalls = m.toolCalls.map((tc: any) =>
              tc.id === tool_call_id
                ? { ...tc, state: is_error ? 'error' : 'success', result, content, error: is_error ? content : undefined, fileChanges: file_changes }
                : tc
            );
            // 同步更新 agentSteps 中对应 tool_call 步骤
            const now = Date.now();
            const newSteps = (m.agentSteps || []).map((s: any) => {
              if (s.type === 'tool_call' && s.toolCall?.id === tool_call_id) {
                return {
                  ...s,
                  status: is_error ? 'error' : 'complete',
                  duration: now - s.timestamp,
                  toolCall: {
                    ...s.toolCall,
                    state: is_error ? 'error' : 'success',
                    result,
                    content,
                    error: is_error ? content : undefined,
                    fileChanges: file_changes,
                  },
                };
              }
              return s;
            });
            return { ...m, toolCalls: newToolCalls, agentSteps: newSteps };
          })
        );
      }),
      // 工具调用审批请求事件：后端需要用户确认才能执行工具（字段 snake_case 与后端对齐）
      listen<any>('tool-approval-requested', (e) => {
        const { approval_id, server_id, tool_name, arguments: args, reason } = e.payload;
        setPendingApprovals(prev => [...prev, {
          approvalId: approval_id,
          serverId: server_id,
          toolName: tool_name,
          arguments: args,
          reason,
        }]);
      }),

      // ===== 子智能体事件监听 =====

      // 子智能体启动：在当前 assistant 消息的 agentSteps 中推入一个 subagent 步骤
      listen<any>('subagent-start', (e) => {
        const { parent_assistant_id, parent_topic_id, subagent_id, profile_id, profile_name, task_summary } = e.payload;
        setDatas('assistants', (a: any) => a.id === parent_assistant_id,
          'topics', (t: any) => t.id === parent_topic_id,
          'history', (h: any[]) => {
            const lastIdx = h.length - 1;
            if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
            const msg = h[lastIdx];
            const steps: any[] = msg.agentSteps || [];
            const newStep = {
              id: `subagent-${subagent_id}`,
              type: 'subagent',
              timestamp: Date.now(),
              status: 'running',
              subagentId: subagent_id,
              subagentProfile: profile_id,
              subagentName: profile_name,
              subagentTask: task_summary,
              subagentSteps: [],
              subagentResult: '',
            };
            return [...h.slice(0, lastIdx), { ...msg, agentSteps: [...steps, newStep] }];
          });
      }),

      // 子智能体步骤更新：追加子 Agent 内部步骤
      listen<any>('subagent-step', (e) => {
        const { parent_assistant_id, parent_topic_id, subagent_id, round, step_type, summary } = e.payload;
        setDatas('assistants', (a: any) => a.id === parent_assistant_id,
          'topics', (t: any) => t.id === parent_topic_id,
          'history', (h: any[]) => {
            const lastIdx = h.length - 1;
            if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
            const msg = h[lastIdx];
            const steps: any[] = msg.agentSteps || [];
            const subIdx = steps.findIndex((s: any) => s.subagentId === subagent_id);
            if (subIdx < 0) return h;
            const subStep = steps[subIdx];
            const newSubStep = {
              id: `substep-${round}-${Date.now()}`,
              type: (step_type || '').startsWith('tool_result') ? 'tool_call' as const : step_type as any,
              timestamp: Date.now(),
              status: 'complete' as const,
              summary: summary || '',
              toolName: (step_type || '').startsWith('tool_result') ? (step_type as string).replace('tool_result:', '') : undefined,
            };
            const updatedSub = {
              ...subStep,
              subagentSteps: [...(subStep.subagentSteps || []), newSubStep],
            };
            return [...h.slice(0, lastIdx), {
              ...msg,
              agentSteps: steps.map((s: any, i: number) => i === subIdx ? updatedSub : s),
            }];
          });
      }),

      // 子智能体完成：标记 subagent 步骤为完成，记录结果
      listen<any>('subagent-done', (e) => {
        const { parent_assistant_id, parent_topic_id, subagent_id, profile_name, result } = e.payload;
        const finishTime = Date.now();
        setDatas('assistants', (a: any) => a.id === parent_assistant_id,
          'topics', (t: any) => t.id === parent_topic_id,
          'history', (h: any[]) => {
            const lastIdx = h.length - 1;
            if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
            const msg = h[lastIdx];
            const steps: any[] = msg.agentSteps || [];
            const subIdx = steps.findIndex((s: any) => s.subagentId === subagent_id);
            if (subIdx < 0) return h;
            const subStep = steps[subIdx];
            const updatedSub = {
              ...subStep,
              status: 'complete',
              duration: finishTime - subStep.timestamp,
              subagentResult: result || '',
            };
            return [...h.slice(0, lastIdx), {
              ...msg,
              agentSteps: steps.map((s: any, i: number) => i === subIdx ? updatedSub : s),
            }];
          });
      }),

      // 子智能体出错
      listen<any>('subagent-error', (e) => {
        const { parent_assistant_id, parent_topic_id, subagent_id, error } = e.payload;
        setDatas('assistants', (a: any) => a.id === parent_assistant_id,
          'topics', (t: any) => t.id === parent_topic_id,
          'history', (h: any[]) => {
            const lastIdx = h.length - 1;
            if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
            const msg = h[lastIdx];
            const steps: any[] = msg.agentSteps || [];
            const subIdx = steps.findIndex((s: any) => s.subagentId === subagent_id);
            if (subIdx < 0) return h;
            const subStep = steps[subIdx];
            const updatedSub = {
              ...subStep,
              status: 'error',
              duration: Date.now() - subStep.timestamp,
              subagentResult: error || '子智能体执行出错',
            };
            return [...h.slice(0, lastIdx), {
              ...msg,
              agentSteps: steps.map((s: any, i: number) => i === subIdx ? updatedSub : s),
            }];
          });
      }),
      // ---- 工作流事件 ----
      listen<WorkflowState>('workflow-start', (e) => {
        setWorkflowState({
          workflowId: e.payload.workflowId,
          title: e.payload.title,
          steps: e.payload.steps.map(s => ({ ...s, status: 'pending' as const })),
          active: true,
        });
      }),
      listen<{ stepId: string }>('workflow-step-start', (e) => {
        setWorkflowState(prev => prev ? {
          ...prev,
          steps: prev.steps.map(s => s.stepId === e.payload.stepId ? { ...s, status: 'running' as const, startedAt: Date.now() } : s),
        } : null);
      }),
      listen<{ stepId: string; status: 'completed' | 'failed'; duration?: number }>('workflow-step-complete', (e) => {
        setWorkflowState(prev => prev ? {
          ...prev,
          steps: prev.steps.map(s => s.stepId === e.payload.stepId ? { ...s, status: e.payload.status, duration: e.payload.duration } : s),
        } : null);
      }),
      listen<{ workflowId: string }>('workflow-complete', () => {
        setWorkflowState(prev => prev ? { ...prev, active: false } : null);
        setTimeout(() => setWorkflowState(null), 5000);
      }),
    ];

    // 组件卸载时清理所有事件监听和命令注册
    onCleanup(() => {
      unlistens.forEach(u => u.then(fn => fn()));
      unregisterCommand(cmdToggleLeft);
      unregisterCommand(cmdToggleRight);
      unregisterCommand(cmdNewTopic);
      unregisterCommand(cmdNewProject);
      unregisterCommand(cmdStopGen);
      unregisterCommand(cmdPrevAsst);
      unregisterCommand(cmdNextAsst);
      unregisterCommand(cmdPrevTopic);
      unregisterCommand(cmdNextTopic);
      unregisterCommand(cmdGoSettings);
      unregisterCommand(cmdGoChat);
      unregisterCommand(cmdToggleSearch);
      unregisterCommand(cmdCycleReasoning);
      unregisterCommand(cmdCycleAgent);
      unregisterCommand(cmdSlashClear);
      unregisterCommand(cmdSlashCompact);
      unregisterCommand(cmdSlashSearch);
      unregisterCommand(cmdSlashSettings);
    });
  });


  // 监听手动触发的"重新生成标题"请求（来自 TopicSidebar 右键菜单）
  // 消费后立即清空信号，避免后续误触发
  createEffect(() => {
    const req = pendingRenameRequest();
    if (req) {
      setPendingRenameRequest(null);
      void checkAndRename(req.asstId, req.topicId);
    }
  });

  // 监听 currentTopicId 变化：若用户切换到与正在生成标题的话题不同的话题，
  // 取消该任务的结果处理（abort）
  let prevTitleGenTopicId: string | null = null;
  createEffect(() => {
    const tId = currentTopicId();
    // 首次运行时不处理（仅记录）
    if (prevTitleGenTopicId === null && activeTitleGen === null) {
      prevTitleGenTopicId = tId;
      return;
    }
    if (activeTitleGen && activeTitleGen.topicId !== tId) {
      activeTitleGen.cancelled = true;
      activeTitleGen = null;
    }
    prevTitleGenTopicId = tId;
  });
  createEffect(() => {
    localStorage.setItem('chat-left-panel-width', leftPanelWidth().toString());
  });
  createEffect(() => {
    localStorage.setItem('chat-right-panel-width', rightPanelWidth().toString());
  });

  // 项目切换时自动检测并启动语言服务器
  createEffect(async () => {
    const pid = currentProjectId();
    if (!pid) {
      // 没有打开项目，清除诊断
      clearAllDiagnostics();
      return;
    }
    const project = currentProject();
    if (!project) return;

    try {
      // 自动检测语言
      const result: any = await invoke('auto_detect_ls', {
        projectPath: project.path,
      });
      const languages: Array<{ languageId: string }> = result?.languages || [];
      
      // 为检测到的每种语言启动语言服务器
      for (const lang of languages) {
        try {
          await invoke('start_lsp_server', {
            projectPath: project.path,
            languageId: lang.languageId,
          });
        } catch (e) {
          // 静默失败（语言服务器可能未安装）
          console.debug(`[LSP] 无法启动 ${lang.languageId} 服务器:`, e);
        }
      }
    } catch (e) {
      console.debug('[LSP] 自动检测失败:', e);
    }
  });

  /**
   * 模型跟随当前助手：
   * 切换助手 / 助手绑定模型变化 / 可用模型列表变化时，
   * 把全局 selectedModel 同步为该助手解析出的有效模型。
   * 助手未绑定 modelId 时 resolveAssistantModel 会回退到当前 selectedModel，跳过覆盖。
   * resolved 为 null（尚无可用模型）时不覆盖，避免启动早期清空。
   */
  createEffect(() => {
    const id = currentAssistantId();
    const asst = datas.assistants.find((a: any) => a.id === id) as Assistant | undefined;
    // 触发依赖：助手 modelId 与可用模型列表（providerConfigs / activatedModels 在 store 内驱动）
    void asst?.modelId;
    const resolved = resolveAssistantModel(asst ?? null);
    if (resolved && resolved.model_id !== selectedModel()?.model_id) {
      setSelectedModel(resolved);
    }
  });

  return (
    <div class="h-full flex gap-[3px] px-[6px] pt-[1px] pb-[6px]" style="background: transparent;"
      classList={{ 'is-resizing': isResizing() }} ref={chatPageRef}>
      <ProjectSidebar
        width={displayLeftWidth()}
        isCollapsed={isLeftCollapsed()}
        onToggle={toggleLeft}
        onResize={(e) => !isLeftCollapsed() && startResize(e, 'left')}
        isResizing={isResizing()}
        onOpenSettings={(id) => setSettingsAsstId(id)}
      />

      <div class="flex-1 flex flex-col min-w-0 min-h-0">
        <Show when={workflowState()}>
          <WorkflowVisualization />
        </Show>

        <ChatInterface
          activeTopic={activeTopic()}
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
          pendingApprovals={pendingApprovals()}
          onResolveApproval={(id) => setPendingApprovals(prev => prev.filter(a => a.approvalId !== id))}
          canShare={!!activeTopic()}
          onOpenShare={() => enterSelectionMode(null)}
          isSelectingMessages={isSelectingMessages()}
          selectedMessageIds={selectedMessageIds()}
          onToggleMessage={handleToggleMessage}
          onSelectAll={handleSelectAll}
          onCancelSelection={handleCancelSelection}
          onConfirmSelection={handleConfirmSelection}
        />
      </div>

      <TopicSidebar
        width={displayRightWidth()}
        isCollapsed={isRightCollapsed()}
        onToggle={toggleRight}
        onResize={(e) => !isRightCollapsed() && startResize(e, 'right')}
        currentAssistant={currentAssistant()}
        editingTopicId={editingTopicId()}
        setEditingTopicId={setEditingTopicId}
        addTopic={addTopic}
        onExportTopic={(topicId) => enterSelectionMode(topicId)}
        isResizing={isResizing()}
      />

      <Portal>
        <ShareModal
          open={showShareModal()}
          onClose={() => { setShowShareModal(false); setActiveShareTopicId(null); setSelectedMessageIds(new Set<string>()); }}
          topic={shareTopic()}
          selectedMessageIds={selectedMessageIds().size > 0 ? selectedMessageIds() : undefined}
        />
      </Portal>
      <Portal>
        <ProjectSettingsModal
          show={settingsAsstId() !== null}
          assistantId={settingsAsstId()}
          onClose={() => setSettingsAsstId(null)}
        />
      </Portal>

      <ProblemsPanel />
    </div>
  );
};

export default ChatPage;
