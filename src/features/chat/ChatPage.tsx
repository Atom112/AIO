import { Component, createSignal, onMount, onCleanup, createEffect, Show } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import {
  appendMessage,
  askBtwQuestion as askBtwQuestionCmd,
  branchTopic,
  callLlmStream,
  generateImage,
  generateTopicTitle,
  getActiveCheckpoint,
  llmCreds,
  loadAssistants,
  runAgentTurn,
  stopLlmStream,
  summarizeHistory,
  type ActiveCheckpoint,
} from '../../core/api/chat';
import { storeChatAttachment } from '../../core/api/attachment';
import { registerChatEventListeners } from './hooks/chatEventListeners';
import { registerChatCommands } from './hooks/chatCommands';
import { TOOL_INTERRUPTED } from './constants';
import { autoDetectLsp, startLspServer } from '../../core/api/lsp';
import { listen } from '@tauri-apps/api/event';
import { useNavigate } from '@solidjs/router';
import {
  datas,
  setDatas,
  currentAssistantId,
  setCurrentAssistantId,
  currentTopicId,
  setCurrentTopicId,
  saveSingleAssistantToBackend,
  Assistant,
  Topic,
  Message,
  PendingAttachment,
  selectedModel,
  setSelectedModel,
  resolveAssistantModel,
  modelKey,
  reasoningLevel,
  setReasoningLevel,
  webSearchEnabled,
  setWebSearchEnabled,
  pendingRenameRequest,
  setPendingRenameRequest,
  resolveAssistantSkills,
  currentProjectId,
  currentProject,
  profileModelOverrides,
  allAvailableModels,
  customSubagentProfiles,
  workflowState,
  setWorkflowState,
  isChatMode,
  projects,
  ensureProjectAssistant,
  setShowProjectCreateModal,
  getLastAgentProjectId,
  initMcpServers,
} from '../../core/store/store';
import { EVENTS } from '../../core/api/events';
import { buildAgentSystemPrompt } from '../../core/agent-prompts';
import { unregisterCommand, resolveSlashCommand } from '../../core/shortcuts';
import { locale, t } from '../../core/i18n';
import Icon from '../../shared/components/Icon';
import { isImageGenModel } from '../../core/utils/models';
import ProjectSidebar from './components/ProjectSidebar';
import ProjectSettingsModal from './components/ProjectSettingsModal';
import ChatInterface from './components/ChatInterface';
import TopicSidebar from './components/TopicSidebar';
import { Portal } from 'solid-js/web';
import ShareModal from './components/ShareModal';
import ProblemsPanel from './components/ProblemsPanel';
import WorkflowVisualization from './components/WorkflowVisualization';

function getReasoningPrompt(level: string): string | null {
  if (level === 'low') return t('agent.prompt.reasoningLow');
  if (level === 'medium') return t('agent.prompt.reasoningMedium');
  if (level === 'high') return t('agent.prompt.reasoningHigh');
  return null;
}

/**
 * 获取当前选中模型的上下文窗口大小（从模型目录读取，缺省 128k）。
 * 用于后端 Agent 循环的 token 预算计算与前端历史压缩阈值。
 */
async function getModelContextWindow(): Promise<number> {
  let maxContext = 128_000; // 默认 128K
  try {
    const { getCachedCatalog } = await import('../../core/utils/models');
    const cat = getCachedCatalog();
    const mdl = selectedModel();
    if (cat && mdl) {
      const modelMeta = cat.models?.find((m: any) => m.id === mdl.model_id);
      if (modelMeta?.contextWindow && modelMeta.contextWindow > 0) {
        maxContext = modelMeta.contextWindow;
      }
    }
  } catch {
    // 模型目录不可用时保持默认值
  }
  return maxContext;
}

/**
 * 构建 Skill 系统消息（token 效率）：
 * - 聊天模式（无 read_skill 工具）：全量注入技能内容（保持原有行为）；
 * - Agent 模式：小技能（≤800 字符）始终全量注入（避免模型漏读、保持可用性）；
 *   大技能仅注入目录行（名称+描述），由模型通过 read_skill 工具按需读取完整说明，
 *   避免 docx/pptx/xlsx 等大技能每次请求全量计费。
 */
function buildSkillMessages(asst: any, fullInject: boolean): { role: 'system'; content: string }[] {
  const skills = resolveAssistantSkills(asst);
  if (skills.length === 0) return [];
  if (fullInject) {
    return skills.map((skill) => ({
      role: 'system' as const,
      content: '[Skill: ' + skill.name + ']\n' + skill.content,
    }));
  }
  const SMALL_SKILL_MAX_CHARS = 800;
  const small = skills.filter((s) => s.content.length <= SMALL_SKILL_MAX_CHARS);
  const large = skills.filter((s) => s.content.length > SMALL_SKILL_MAX_CHARS);
  const msgs = small.map((skill) => ({
    role: 'system' as const,
    content: '[Skill: ' + skill.name + ']\n' + skill.content,
  }));
  if (large.length > 0) {
    msgs.push({
      role: 'system' as const,
      content:
        t('chat.skillCatalogNotice') +
        '\n' +
        large.map((s) => '- ' + s.name + ': ' + (s.description || s.name)).join('\n'),
    });
  }
  return msgs;
}
import type { PendingApproval } from './components/ToolApprovalBubble';
import { clearAllDiagnostics } from '../../core/store/diagnostics';

let isFirstAppLaunch = true;
const DEFAULT_ASST_ID = 'default-assistant-id';

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
  id: Date.now().toString(), // 使用当前时间戳作为唯一标识符
  name: name || t('chat.untitledTopic', { time: new Date().toLocaleTimeString(locale()) }), // 默认名称包含创建时间
  history: [], // 消息历史记录数组
  summary: '', // SQLite 存储方案新增：长期记忆摘要，用于压缩历史上下文
});

/**
 * 辅助函数：创建新助手对象
 * @param name - 可选的助手名称
 * @param id - 可选的指定 ID（用于从数据库恢复时使用已有 ID）
 * @returns Assistant 对象，包含 ID、名称、系统提示词和默认话题
 */
const createAssistant = (name?: string, id?: string): Assistant => ({
  id: id ?? Date.now().toString(), // 若未提供 ID 则生成新的时间戳 ID
  name: name || t('chat.newAssistant'), // 默认助手名称
  prompt: t('chat.defaultAssistantPrompt'), // 默认系统提示词
  modelId: selectedModel() ? modelKey(selectedModel()!) : undefined, // 继承当前生效模型（复合键）作为新助手默认模型
  // 有项目上下文时自动启用内置文件系统 MCP，让 agent 开箱即用
  mcpServerIds: currentProjectId() ? ['__aio-filesystem__'] : [],
  skillIds: [],
  projectId: currentProjectId() ?? undefined, // 关联当前项目
  assistantType: id === DEFAULT_ASST_ID ? 'chat' : 'project',
  topics: [createTopic(t('chat.defaultTopic'))], // 每个助手默认创建一个"默认话题"
});

/**
 * 聊天页面主组件
 * 管理三栏布局、所有业务逻辑和状态流转
 * @component
 */
const ChatPage: Component = () => {
  const [leftPanelWidth, setLeftPanelWidth] = createSignal(
    Number(localStorage.getItem('chat-left-panel-width')) || 18,
  ); // 左侧面板宽度百分比（助手列表），默认 18%，范围 15%-30%
  const [rightPanelWidth, setRightPanelWidth] = createSignal(
    Number(localStorage.getItem('chat-right-panel-width')) || 18,
  ); // 右侧面板宽度百分比（话题列表），默认 18%，范围 15%-30%
  /**
   * 左侧面板：宽度与光标 1:1 跟手（展开/收起两个方向都是），松手时才吸附。
   * 最窄跟手宽度即收起轨道宽度，保证把手始终可抓取、内容不外溢。
   */
  const LEFT_RAIL_WIDTH_PX = 48; // 收起轨道宽度，与 ProjectSidebar 保持一致
  const LEFT_MAX_WIDTH_PCT = 30;
  /** 松手判定死区（px）：向外拖出不足该距离视为未真正拉开，松手仍收起 */
  const LEFT_RELEASE_DEADZONE_PX = 12;
  const [isResizing, setIsResizing] = createSignal(false);
  /** 拖拽中的实时宽度预览（%）：跟手期间用它渲染，松手后清空并落到 leftPanelWidth 记忆值 */
  const [leftDragWidth, setLeftDragWidth] = createSignal<number | null>(null);
  const [isLeftCollapsed, setIsLeftCollapsed] = createSignal(
    localStorage.getItem('left-collapsed') === 'true',
  ); // 左右两侧面板宽度调整逻辑
  const [isRightCollapsed, setIsRightCollapsed] = createSignal(
    localStorage.getItem('right-collapsed') === 'true',
  );
  const [inputMessage, setInputMessage] = createSignal(''); // 当前输入框中的消息文本
  const [pendingFiles, setPendingFiles] = createSignal<PendingAttachment[]>([]); // 待发送的文件列表（已复制到应用附件目录但尚未关联消息）
  const [isThinking, setIsThinking] = createSignal(false); // AI 是否正在思考/生成回复（控制加载动画和停止按钮）
  const [isProcessing, setIsProcessing] = createSignal(false); // 是否正在处理文件（控制文件解析加载状态）
  const [isDragging, setIsDragging] = createSignal(false); // 是否正在拖拽文件到窗口（控制拖拽状态样式）
  const [typingIndex, setTypingIndex] = createSignal<number | null>(null); // 当前正在打字机效果显示的消息索引，null 表示无打字效果
  const [editingTopicId, setEditingTopicId] = createSignal<string | null>(null); // 当前正在编辑名称的话题 ID，null 表示无编辑中
  const [settingsAsstId, setSettingsAsstId] = createSignal<string | null>(null); // 当前打开设置弹窗的助手 ID，null 表示弹窗关闭
  // 待用户审批的工具调用列表
  /** 分享弹窗状态 */
  const [showShareModal, setShowShareModal] = createSignal(false);
  const [activeShareTopicId, setActiveShareTopicId] = createSignal<string | null>(null);
  /** 消息级选择模式 */
  const [isSelectingMessages, setIsSelectingMessages] = createSignal(false);
  const [selectedMessageIds, setSelectedMessageIds] = createSignal<Set<string>>(new Set());
  const [pendingApprovals, setPendingApprovals] = createSignal<PendingApproval[]>([]);
  /** /btw 悬浮问答框列表（临时展示，不存历史） */
  const [btwOverlays, setBtwOverlays] = createSignal<
    Array<{ id: string; question: string; answer: string | null; loading: boolean }>
  >([]);
  /** 页面根元素引用，用于计算拖拽调整面板宽度时的相对位置 */
  let chatPageRef: HTMLDivElement | undefined;
  /**
   * 计算左侧面板显示宽度
   * @returns {number} 拖拽中返回实时跟手宽度，否则返回记忆宽度（收起态由子组件处理为轨道宽）
   */
  const displayLeftWidth = () => leftDragWidth() ?? leftPanelWidth();
  /**
   * 左侧面板当前是否按"收起轨道"渲染。
   * 跟手过程中（leftDragWidth 非空）始终按展开态渲染，这样从轨道往外拖时
   * 宽度才能真正由光标驱动，而不是等松手后靠 CSS 过渡"弹"开。
   */
  const leftPanelCollapsed = () => isLeftCollapsed() && leftDragWidth() === null;

  // 历史遗留：旧版拖拽可能把 <15% 的中间宽度写进记忆值（localStorage），展开态下兜底恢复默认
  createEffect(() => {
    if (!isLeftCollapsed() && leftPanelWidth() < 15) setLeftPanelWidth(18);
  });
  /**
   * 计算右侧面板显示宽度
   * @returns {number} 右侧面板宽度，如果折叠则返回0
   */
  const displayRightWidth = () => (isRightCollapsed() ? 0 : rightPanelWidth());

  /**
   * 设置左侧面板收起状态并持久化（折叠/展开统一入口，含宽度兜底恢复）。
   * @param collapsed - 目标收起状态
   */
  const setLeftCollapsed = (collapsed: boolean) => {
    setIsLeftCollapsed(collapsed);
    localStorage.setItem('left-collapsed', String(collapsed));
    if (!collapsed && leftPanelWidth() < 5) {
      setLeftPanelWidth(18);
    }
  };

  /**
   * 切换左侧面板的折叠状态
   * @param {MouseEvent} e - 鼠标事件
   */
  const toggleLeft = (e: MouseEvent) => {
    e.stopPropagation(); // 防止触发拖拽
    setLeftCollapsed(!isLeftCollapsed());
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
  const currentAssistant = () => datas.assistants.find((a) => a.id === currentAssistantId());

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
    const fileName = filePath.split(/[\\/]/).pop() || t('chat.unknownFile');
    const ext = (fileName.split('.').pop() || '').toLowerCase();
    const ALLOWED_IMG = ['png', 'jpg', 'jpeg', 'webp'];
    const ALLOWED_DOC = [
      'pdf',
      'docx',
      'pptx',
      'txt',
      'md',
      'json',
      'csv',
      'log',
      'xml',
      'yaml',
      'yml',
      'ini',
      'tsv',
      // 源代码 / 文本
      'rs',
      'c',
      'h',
      'cpp',
      'hpp',
      'cc',
      'cxx',
      'cs',
      'go',
      'java',
      'rb',
      'py',
      'js',
      'mjs',
      'cjs',
      'ts',
      'tsx',
      'jsx',
      'php',
      'swift',
      'kt',
      'kts',
      'scala',
      'lua',
      'sql',
      'toml',
      'sh',
      'bash',
      'zsh',
      'dart',
      'html',
      'css',
      'scss',
      'less',
      'vue',
      'svelte',
      'gradle',
      'properties',
      'r',
      'pl',
    ];
    const isImg = fileType === 'image' || ALLOWED_IMG.includes(ext);
    const isDoc = ALLOWED_DOC.includes(ext);
    if (!isImg && !isDoc) {
      alert(
        t('chat.unsupportedFile', {
          extension: ext,
          supported: [...ALLOWED_IMG, ...ALLOWED_DOC].join(', '),
        }),
      );
      return;
    }
    setIsProcessing(true);
    try {
      const stored = await storeChatAttachment(filePath);
      setPendingFiles((prev) =>
        prev.some((file) => file.id === stored.id)
          ? prev
          : [
              ...prev,
              {
                ...stored,
                name: fileName,
                type: isImg ? 'image' : 'text',
                previewUrl: isImg ? convertFileSrc(stored.storagePath) : undefined,
              },
            ],
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
  const checkAndSummarize = async (keepBudgetOverride?: number) => {
    const topic = activeTopic();
    const currentMdl = selectedModel();
    if (!topic || !currentMdl) return;

    // 获取当前模型的上下文窗口大小
    const maxContext = await getModelContextWindow();

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
        const text = typeof msg.content === 'string' ? msg.content : msg.displayText || '';
        totalUsed += Math.ceil(text.length / 4);
      }
    }

    const usageRatio = maxContext > 0 ? totalUsed / maxContext : 0;
    if (usageRatio < 0.75 || topic.history.length <= 10) return;

    console.log(
      `[Context] 触发压缩: ${(usageRatio * 100).toFixed(0)}% (${totalUsed}/${maxContext} tokens, ${topic.history.length} 条消息)`,
    );

    // Token 预算制保留：默认从尾部累计约 2 万 token（至少保留最近 4 条消息，即一个完整回合）；
    // 可通过 `/compact <n>` 指定本次保留预算
    const KEEP_TOKEN_BUDGET =
      keepBudgetOverride && keepBudgetOverride > 0 ? keepBudgetOverride : 20_000;
    const estimateMsgTokens = (m: any): number => {
      const text = typeof m.content === 'string' ? m.content : (m.displayText ?? '');
      return Math.ceil(text.length / 4) + 8;
    };
    let keepCount = 0;
    let acc = 0;
    for (let i = topic.history.length - 1; i >= 0; i--) {
      acc += estimateMsgTokens(topic.history[i]);
      keepCount = topic.history.length - i;
      if (acc >= KEEP_TOKEN_BUDGET) break;
    }
    keepCount = Math.max(keepCount, 4);
    const summarizeCount = topic.history.length - keepCount;
    if (summarizeCount <= 0) return;
    // 覆盖式摘要：把旧摘要作为输入上下文注入，新摘要整体替换旧摘要，避免拼接式膨胀
    const messagesToSummarize = topic.summary
      ? [
          { role: 'system' as const, content: t('chat.historyBackground') + topic.summary },
          ...topic.history.slice(0, summarizeCount),
        ]
      : topic.history.slice(0, summarizeCount);

    try {
      const newSummarySnippet = await summarizeHistory(
        llmCreds(currentMdl),
        JSON.stringify(messagesToSummarize),
      );
      const latestTopic = activeTopic();
      if (!latestTopic) return;
      const updatedHistory = latestTopic.history.slice(summarizeCount);
      setDatas(
        'assistants',
        (a) => a.id === currentAssistantId(),
        'topics',
        (t) => t.id === latestTopic.id,
        {
          history: updatedHistory,
          summary: newSummarySnippet,
        },
      );
      await saveSingleAssistantToBackend(currentAssistantId()!);
      console.log('[Context] 压缩完成');
    } catch (e) {
      console.error('生成总结失败:', e);
    }
  };

  /**
   * 启发式后备方案：从历史中提取一段文本作为标题。
   * 用于 LLM 标题生成失败时保证至少能给出有意义的命名。
   * 取第一条 user 消息的前 12 个字符。
   */
  const fallbackTitle = (history: Message[]): string => {
    const firstUser = history.find((m) => m.role === 'user');
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
    if (!asst) {
      if (activeTitleGen === task) activeTitleGen = null;
      return;
    }
    // 必须用事件中的 topicId 重新定位（用户可能已切换话题）
    const topic = (asst.topics as Topic[]).find((t: Topic) => t.id === eventTopicId);
    if (!topic) {
      if (activeTitleGen === task) activeTitleGen = null;
      return;
    }

    // 1. 默认话题不重命名
    if (asst.topics[0]?.id === topic.id) {
      if (activeTitleGen === task) activeTitleGen = null;
      return;
    }
    // 2. 已经重命名过的话题不再重命名
    if (topic.renamed) {
      if (activeTitleGen === task) activeTitleGen = null;
      return;
    }
    // 3. 没有可参考的对话内容
    if (topic.history.length < 2) {
      if (activeTitleGen === task) activeTitleGen = null;
      return;
    }

    // 4. 检查最后一条 AI 消息是否有效（避免错误响应触发重命名）
    const lastMsg = topic.history[topic.history.length - 1];
    if (lastMsg?.role !== 'assistant') {
      if (activeTitleGen === task) activeTitleGen = null;
      return;
    }
    const lastContent = typeof lastMsg.content === 'string' ? lastMsg.content.trim() : '';
    if (lastContent.length < 2 || lastContent.startsWith('[Error:')) {
      if (activeTitleGen === task) activeTitleGen = null;
      return;
    }

    const currentMdl = selectedModel();
    if (!currentMdl) {
      if (activeTitleGen === task) activeTitleGen = null;
      return;
    }

    // 截断到前 2 条消息：业界标准做法（user + assistant 即可概括主题，省 token）
    const sample = topic.history.slice(0, 2);

    // 第一阶段：尝试 LLM 生成
    let newName: string | null = null;
    let llmError: unknown = null;
    try {
      const result = await generateTopicTitle(llmCreds(currentMdl), JSON.stringify(sample));
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
    if (!latestAsst || !latestTopic) {
      if (activeTitleGen === task) activeTitleGen = null;
      return;
    }
    if (latestAsst.topics[0]?.id === latestTopic.id) {
      if (activeTitleGen === task) activeTitleGen = null;
      return;
    }
    if (latestTopic.renamed) {
      if (activeTitleGen === task) activeTitleGen = null;
      return;
    }

    // 实在拿不到任何名字：仅标记为已重命名，避免下次再尝试
    if (!newName) {
      setDatas(
        'assistants',
        (a) => a.id === eventAsstId,
        'topics',
        (t) => t.id === eventTopicId,
        'renamed',
        true,
      );
      await saveSingleAssistantToBackend(eventAsstId);
      if (activeTitleGen === task) activeTitleGen = null;
      console.warn('自动重命名失败，无可用后备:', llmError);
      return;
    }

    // 与旧名相同则视为无效，但仍标记为已重命名
    if (newName === latestTopic.name) {
      setDatas(
        'assistants',
        (a) => a.id === eventAsstId,
        'topics',
        (t) => t.id === eventTopicId,
        'renamed',
        true,
      );
      await saveSingleAssistantToBackend(eventAsstId);
      if (activeTitleGen === task) activeTitleGen = null;
      return;
    }

    setDatas(
      'assistants',
      (a) => a.id === eventAsstId,
      'topics',
      (t) => t.id === eventTopicId,
      {
        name: newName,
        renamed: true,
      },
    );
    await saveSingleAssistantToBackend(eventAsstId);
    if (activeTitleGen === task) activeTitleGen = null;
    console.log(`话题已自动重命名: ${latestTopic.name} → ${newName}`);
  };

  /**
   * 截断过长的工具返回内容（头尾保留式，与后端 truncate_tool_result 对齐：前 60% + 后 40%，
   * 20k 字符），防止 LLM 上下文膨胀，同时保留 read_file 已保留的文件首尾。
   * 完整内容保留在 agentSteps 中供用户查看。
   */
  const truncateToolResult = (content: string): string => {
    const MAX_LEN = 20000;
    if (content.length <= MAX_LEN) return content;
    const head = Math.floor((MAX_LEN * 6) / 10);
    const tail = MAX_LEN - head;
    const headEnd = head;
    const tailStart = content.length - tail;
    if (tailStart > headEnd) {
      return (
        content.slice(0, headEnd) +
        `\n\n${t('chat.truncatedHeadTail', { total: content.length })}\n\n` +
        content.slice(tailStart)
      );
    }
    return content.slice(0, MAX_LEN) + `\n\n${t('chat.truncatedTail', { total: content.length })}`;
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
    if (m.images?.length) obj.images = m.images; // 后端据此剥离 aio-image 标记并展开 image_url 块
    if (m.displayFiles?.length) obj.displayFiles = m.displayFiles; // 携带附件（含图片），后端据此读取并展开 image_url 块
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
      if (
        (msg.role === 'user' || (msg.role === 'assistant' && !msg.toolCalls?.length)) &&
        pendingToolCallIds.length > 0
      ) {
        flushPending(result);
      }
    }

    // 收尾：如果数组结束时仍有未匹配的 toolCallId，补全占位消息
    if (pendingToolCallIds.length > 0) flushPending(result);

    return result;
  }

  /**
   * /btw 侧问：基于已有对话上下文流式回答（无工具、不修改对话历史）。
   * 监听 `btw-chunk` 事件逐 chunk 更新 overlay。
   * unlisten 在收到 done/error 事件时触发，避免 invoke resolve 与事件交付的竞态。
   */
  const askBtwQuestion = async (question: string, overlayId: string) => {
    const currentMdl = selectedModel();
    const asstObj = currentAssistant();
    const topicObj = activeTopic();
    if (!currentMdl || !asstObj || !topicObj) return;

    let resolved = false;

    // 在 invoke 之前注册监听，确保不丢失首个 chunk
    const unlisten = await listen<{
      overlay_id: string;
      content: string;
      done: boolean;
      error?: string;
    }>(EVENTS.btwChunk, (e) => {
      const { overlay_id, content, done, error } = e.payload;
      if (overlay_id !== overlayId) return;

      if (error) {
        resolved = true;
        setBtwOverlays((prev) =>
          prev.map((o) =>
            o.id === overlayId ? { ...o, answer: `[Error] ${error}`, loading: false } : o,
          ),
        );
        unlisten();
        return;
      }
      if (done) {
        resolved = true;
        setBtwOverlays((prev) =>
          prev.map((o) => (o.id === overlayId ? { ...o, loading: false } : o)),
        );
        unlisten();
        return;
      }
      setBtwOverlays((prev) =>
        prev.map((o) => (o.id === overlayId ? { ...o, answer: (o.answer || '') + content } : o)),
      );
    });

    try {
      // 使用 BTW 专用 system prompt：禁止模型继续执行主任务，仅基于上下文回答
      const btwSystemPrompt = `[SIDE QUESTION — DO NOT CONTINUE THE MAIN TASK]
You are answering a brief side question about the conversation above.
Do NOT continue, repeat, or redo any work from the conversation.
Only answer the specific question asked. Be concise.

${asstObj.prompt}`;

      const messagesForAI: Record<string, unknown>[] = [
        { role: 'system', content: btwSystemPrompt },
        ...topicObj.history.flatMap((m: any) => buildApiMessages(m)),
        { role: 'user', content: question },
      ];

      await askBtwQuestionCmd(llmCreds(currentMdl), overlayId, JSON.stringify(messagesForAI));
    } catch (err) {
      if (!resolved) {
        setBtwOverlays((prev) =>
          prev.map((o) =>
            o.id === overlayId ? { ...o, answer: `[Error] ${err}`, loading: false } : o,
          ),
        );
        unlisten();
      }
    }
  };

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
        content = `${content}\n\n${t('agent.prompt.projectContext', { path: project.path })}`;
      }
    }

    const newUserMsg = {
      id: crypto.randomUUID(),
      role: 'user' as const,
      content,
      displayText,
    };

    // 构建完整消息数组（与普通发送流程一致）
    const reasoningPrompt = getReasoningPrompt(reasoningLevel());

    const agentMode = currentAsst?.agentMode || 'off';
    const pid = currentProjectId();
    const projectInfo =
      pid && currentProject()
        ? { path: currentProject()!.path, name: currentProject()!.name }
        : null;
    const agentPromptContent =
      agentMode !== 'off' && projectInfo
        ? buildAgentSystemPrompt(agentMode as any, projectInfo)
        : null;
    const agentSystemPrompt = agentPromptContent
      ? [{ role: 'system' as const, content: agentPromptContent }]
      : [];

    let messagesForAI: any[] = [
      { role: 'system', content: currentAsst.prompt },
      ...buildSkillMessages(currentAsst, agentMode === 'off'),
      ...agentSystemPrompt,
      ...(reasoningPrompt ? [{ role: 'system', content: reasoningPrompt }] : []),
      ...(webSearchEnabled() ? [{ role: 'system', content: t('agent.prompt.web') }] : []),
      ...(currentTopic.summary
        ? [
            {
              role: 'system',
              content: t('agent.prompt.summary', { summary: currentTopic.summary }),
            },
          ]
        : []),
      ...currentTopic.history.flatMap((m: any) => buildApiMessages(m)),
      { role: 'user', content: newUserMsg.content },
    ];
    messagesForAI = ensureToolMessagesComplete(messagesForAI);

    // 持久化用户消息
    try {
      await appendMessage(topicId, newUserMsg);
    } catch (err) {
      console.error('[chat] save message failed:', err);
      alert(t('error.save'));
      return;
    }

    // 更新本地状态
    setDatas(
      'assistants',
      (a: any) => a.id === asstId,
      'topics',
      (t: any) => t.id === topicId,
      'history',
      (h: any[]) => [...h, newUserMsg],
    );

    // 调用 LLM
    try {
      setIsThinking(true);
      await runAgentTurn({
        ...llmCreds(currentMdl),
        assistantId: asstId,
        topicId,
        messages: messagesForAI,
        mcpServerIds: currentAsst?.mcpServerIds ?? [],
        agentMode,
        projectId: currentProjectId() ?? null,
        webSearchEnabled: webSearchEnabled(),
        profileModelOverrides: [],
        customSubagentProfiles: [],
        locale: locale(),
        contextWindow: await getModelContextWindow(),
        skills: agentMode === 'off' ? [] : resolveAssistantSkills(currentAsst),
      });
    } catch (err) {
      alert(err);
      setIsThinking(false);
      setTypingIndex(null);
    }
  };

  // ===== P4.4 检查点恢复：崩溃残留回合的"继续未完成的回合"入口 =====
  const [activeCheckpoint, setActiveCheckpoint] = createSignal<ActiveCheckpoint | null>(null);
  createEffect(() => {
    const tid = currentTopicId();
    setActiveCheckpoint(null);
    if (!tid) return;
    void (async () => {
      try {
        const cp = await getActiveCheckpoint(tid);
        // 竞态防护：仅当话题未再切换时应用
        if (currentTopicId() === tid) setActiveCheckpoint(cp);
      } catch {
        /* 查询失败静默：入口不展示 */
      }
    })();
  });

  const handleResumeTurn = async () => {
    const cp = activeCheckpoint();
    const tid = currentTopicId();
    const asstId = currentAssistantId();
    const currentMdl = selectedModel();
    if (!cp || !tid || !asstId || !currentMdl || isThinking()) return;
    const currentAsst = datas.assistants.find((a: Assistant) => a.id === asstId);
    const resumeAgentMode = currentAsst?.agentMode || 'off';
    setActiveCheckpoint(null);
    setIsThinking(true);
    try {
      await runAgentTurn({
        ...llmCreds(currentMdl),
        assistantId: asstId,
        topicId: tid,
        messages: [], // 恢复时后端用检查点快照替换，前端消息不参与
        mcpServerIds: currentAsst?.mcpServerIds ?? [],
        agentMode: resumeAgentMode,
        projectId: currentProjectId() ?? null,
        webSearchEnabled: webSearchEnabled(),
        profileModelOverrides: [],
        customSubagentProfiles: [],
        locale: locale(),
        contextWindow: await getModelContextWindow(),
        skills: resumeAgentMode === 'off' ? [] : resolveAssistantSkills(currentAsst),
        resumeTurnId: cp.turnId,
      });
    } catch (err) {
      alert(err);
      setIsThinking(false);
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
    setDatas(
      'assistants',
      (a: any) => a.id === asstId,
      'topics',
      (t: any) => t.id === topicId,
      'history',
      (h: any[]) => [
        ...h,
        {
          id: crypto.randomUUID(),
          role: 'user' as const,
          content: displayText,
          displayText,
        },
      ],
    );

    // 执行 handler（Action 型命令可携带参数，如 `/compact 10000` 指定保留 token 预算）
    await cmd.handler(resolved.args);

    // 根据命令类型给出反馈消息
    let feedback = '';
    if (cmd.id === 'slash-compact') {
      feedback = `✓ /compact — ${t('slash.feedback.compacted')}`;
    } else if (cmd.id === 'slash-clear') {
      // /clear handler 已清空历史，无需额外反馈
      return;
    } else if (cmd.id === 'slash-search') {
      feedback = webSearchEnabled()
        ? `✓ ${t('slash.feedback.searchOn')}`
        : `✓ ${t('slash.feedback.searchOff')}`;
    } else if (cmd.id === 'slash-help') {
      // /help handler 已添加帮助信息，无需额外反馈
      return;
    } else if (cmd.id === 'slash-settings') {
      // /settings handler 已跳转页面
      return;
    } else {
      feedback = `✓ ${t('slash.feedback.executed', { command: cmd.label })}`;
    }

    // 添加反馈消息
    setDatas(
      'assistants',
      (a: any) => a.id === asstId,
      'topics',
      (t: any) => t.id === topicId,
      'history',
      (h: any[]) => [
        ...h,
        {
          id: crypto.randomUUID(),
          role: 'assistant' as const,
          content: feedback,
          displayText: feedback,
        },
      ],
    );
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
    if (!currentMdl || !topicObj || !asstObj) return;

    const userInput = inputMessage().trim();
    const files = pendingFiles();
    // isThinking 时阻止普通消息发送（/btw 由下方独立处理）
    if (isThinking() && !userInput.startsWith('/btw')) return;
    // 必须满足：有文本输入或有文件附件
    if (!userInput && files.length === 0) return;

    // ---- 斜杠命令拦截 ----
    if (userInput.startsWith('/')) {
      const resolved = resolveSlashCommand(userInput);
      if (resolved) {
        // /btw 独立处理：立即基于当前上下文回答，不中断主 agent
        if (resolved.command.id === 'slash-btw') {
          const question = resolved.args || '';
          const id = crypto.randomUUID();
          setBtwOverlays((prev) => [...prev, { id, question, answer: '', loading: true }]);
          askBtwQuestion(question, id);
          setInputMessage('');
          setPendingFiles([]);
          return;
        }
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
        setDatas(
          'assistants',
          (a: any) => a.id === unknownAsstId,
          'topics',
          (t: any) => t.id === unknownTopicId,
          'history',
          (h: any[]) => [
            ...h,
            {
              id: crypto.randomUUID(),
              role: 'user' as const,
              content: userInput,
              displayText: userInput,
            },
            {
              id: crypto.randomUUID(),
              role: 'assistant' as const,
              content: t('slash.feedback.unknown', { command: userInput }),
            },
          ],
        );
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
      displayFiles: files.map((f) => ({
        id: f.id,
        name: f.name,
        mimeType: f.mimeType,
        size: f.size,
        storagePath: f.storagePath,
      })),
      displayText: userInput,
    };

    const currentAsst = currentAssistant();
    const currentTopic = activeTopic();
    if (!currentAsst || !currentTopic) return;

    /** 根据推理强度注入对应的 system 提示, 让模型使用 <think>...</think> 输出思考过程 */
    const reasoningPrompt = getReasoningPrompt(reasoningLevel());

    const agentMode = currentAsst?.agentMode || 'off';
    const pid = currentProjectId();
    const projectInfo =
      pid && currentProject()
        ? { path: currentProject()!.path, name: currentProject()!.name }
        : null;
    const agentPromptContent =
      agentMode !== 'off' && projectInfo
        ? buildAgentSystemPrompt(agentMode as any, projectInfo)
        : null;
    const agentSystemPrompt = agentPromptContent
      ? [{ role: 'system' as const, content: agentPromptContent }]
      : [];
    let messagesForAI: any[] = [
      { role: 'system', content: currentAsst.prompt },
      ...buildSkillMessages(currentAsst, isChatMode()),
      ...agentSystemPrompt,
      ...(reasoningPrompt ? [{ role: 'system', content: reasoningPrompt }] : []),
      ...(webSearchEnabled() ? [{ role: 'system', content: t('agent.prompt.web') }] : []),
      ...(currentTopic.summary
        ? [
            {
              role: 'system',
              content: t('agent.prompt.summary', { summary: currentTopic.summary }),
            },
          ]
        : []),
      ...currentTopic.history.flatMap((m: any) => buildApiMessages(m)),
      { role: 'user', content: newUserMsg.content, displayFiles: newUserMsg.displayFiles },
    ];
    // 安全网：确保每个 assistant(tool_calls) 都有对应的 role:tool 消息
    messagesForAI = ensureToolMessagesComplete(messagesForAI);

    const lastMsg = messagesForAI[messagesForAI.length - 1];
    if (lastMsg.role !== 'user') {
      console.error('错误：发送给 API 的最后一条消息不是 User!', lastMsg);
      return;
    }

    // 先持久化用户消息和附件关联，避免流式请求期间退出或重复上传导致附件成为孤儿。
    try {
      await appendMessage(topicId, newUserMsg);
    } catch (err) {
      console.error('save_message_with_attachments failed:', err);
      alert(t('error.save'));
      return;
    }

    // 更新本地 Store：添加用户消息，聊天模式下同时创建空 assistant 占位消息
    // （项目/Agent 模式下占位消息由 llm-round-start 事件创建）
    if (isChatMode()) {
      setDatas(
        'assistants',
        (a) => a.id === asstId,
        'topics',
        (t) => t.id === topicId,
        'history',
        (h) => [
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
        ],
      );
    } else {
      setDatas(
        'assistants',
        (a) => a.id === asstId,
        'topics',
        (t) => t.id === topicId,
        'history',
        (h) => [...h, newUserMsg],
      );
    }

    // 清空输入状态和文件列表，设置生成中状态
    setInputMessage('');
    setPendingFiles([]);
    setIsThinking(true);

    // 设置打字机索引（聊天模式下刚创建的 assistant 占位消息位置）
    if (isChatMode()) {
      const asst = datas.assistants.find((a) => a.id === asstId);
      const topic = asst?.topics.find((t: Topic) => t.id === topicId);
      if (topic) setTypingIndex(topic.history.length - 1);
    }

    try {
      // 聊天模式：纯对话，直接调用 call_llm_stream（无 agent 循环、无工具调用）
      // 项目模式/Agent 模式：调用 run_agent_turn（含工具调用、子智能体等）
      if (isChatMode() && isImageGenModel(currentMdl.model_id, currentMdl.api_url)) {
        // 专用端点（dall-e-2/3 等）：非流式生成，结果经 llm-chunk(done) 回传
        await generateImage(llmCreds(currentMdl), asstId, topicId, userInput);
      } else if (isChatMode()) {
        await callLlmStream(llmCreds(currentMdl), asstId, topicId, messagesForAI);
      } else {
        // 解析 per-profile 模型覆盖（仅 agent 模式需要）
        const resolvedOverrides: Array<{
          profileId: string;
          modelId: string;
          apiUrl: string;
          apiKey: string;
        }> = [];
        for (const [profileId, mKey] of Object.entries(profileModelOverrides())) {
          if (!mKey) continue;
          const mdl = allAvailableModels().find((m) => modelKey(m) === mKey);
          if (mdl) {
            resolvedOverrides.push({
              profileId,
              modelId: mdl.model_id,
              apiUrl: mdl.api_url,
              apiKey: mdl.api_key ?? '',
            });
          }
        }

        await runAgentTurn({
          ...llmCreds(currentMdl),
          assistantId: asstId,
          topicId,
          messages: messagesForAI,
          mcpServerIds: currentAsst?.mcpServerIds ?? [],
          agentMode,
          projectId: currentProjectId() ?? null,
          webSearchEnabled: webSearchEnabled(),
          profileModelOverrides: resolvedOverrides,
          customSubagentProfiles: customSubagentProfiles(),
          locale: locale(),
          contextWindow: await getModelContextWindow(),
          skills: isChatMode() ? [] : resolveAssistantSkills(currentAsst),
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
      await stopLlmStream(currentAssistantId() ?? '', currentTopicId() ?? '');
    } catch (err) {
      console.error('stop_llm_stream 失败:', err);
    }
    setPendingApprovals([]);
  };

  /** 切换到聊天模式 */
  const switchToChat = () => {
    setCurrentAssistantId(DEFAULT_ASST_ID);
    const asst = datas.assistants.find((a) => a.id === DEFAULT_ASST_ID);
    if (asst?.topics?.length) {
      setCurrentTopicId(asst.topics[0].id);
    }
  };

  /** 切换到指定项目 */
  const switchToProject = async (projectId: string) => {
    try {
      const asstId = await ensureProjectAssistant(projectId);
      setCurrentAssistantId(asstId);
      const asst = datas.assistants.find((a) => a.id === asstId);
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
    setDatas(
      'assistants',
      (a) => a.id === asstId,
      'topics',
      (prev) => [...prev, newT],
    );
    setCurrentTopicId(newT.id);
    await saveSingleAssistantToBackend(asstId);
  };

  /**
   * 从指定消息处分叉出新话题（会话分支）。
   * 将当前话题中该消息及之前的所有消息复制到新话题，并导航过去。
   * @param sourceMessageId - 分支点的消息 ID
   */
  const branchFromMessage = async (sourceMessageId: string) => {
    const asstId = currentAssistantId();
    const topicId = currentTopicId();
    if (!asstId || !topicId) return;
    try {
      const newTopic: Topic = await branchTopic(topicId, sourceMessageId);
      setDatas(
        'assistants',
        (a: any) => a.id === asstId,
        'topics',
        (prev: Topic[]) => [...prev, newTopic],
      );
      setCurrentTopicId(newTopic.id);
      await saveSingleAssistantToBackend(asstId);
    } catch (e) {
      console.error('分支创建失败:', e);
    }
  };

  /**
   * 通用拖拽调整宽度：绑定 mousemove/mouseup，按百分比回调更新宽度。
   * @param e - MouseEvent 鼠标事件
   * @param onDragMove - 每次鼠标移动时回调，入参为光标相对整页宽度换算的百分比
   */
  const attachResizeDrag = (e: MouseEvent, onDragMove: (pct: number) => void) => {
    e.preventDefault();
    setIsResizing(true);

    const totalW = chatPageRef?.offsetWidth ?? window.innerWidth;
    const handleMove = (moveEvent: MouseEvent) => {
      onDragMove((moveEvent.clientX / totalW) * 100);
    };

    const stopResize = () => {
      setIsResizing(false);
      document.removeEventListener('mousemove', handleMove);
      document.removeEventListener('mouseup', stopResize);
    };

    document.addEventListener('mousemove', handleMove);
    document.addEventListener('mouseup', stopResize);
  };

  /**
   * 拖拽调整左侧面板宽度：展开/收起两个方向都严格 1:1 跟手，松手时才吸附。
   *
   * - 跟手原点：把面板"当前占用的像素宽"折算到光标坐标轴上——收起态用轨道宽，
   *   展开态用 DOM 实测的渲染宽（把手就在面板右缘，实测能保证首帧零位移、无跳变）。
   *   宽度增量 = 光标位移，严格 1:1；
   * - 百分比换算基用 `clientWidth`（内容宽），与 CSS `width:%` 的解析基准一致；
   * - 最窄限制：跟手宽度不低于收起轨道宽，把手拖不到最左，内容不外溢；
   * - 松手吸附：顶到最窄或拖出距离仍在死区内 → 收起；否则落到 ≥15% 并记住宽度。
   *
   * @param e - MouseEvent 鼠标事件
   */
  const startLeftResize = (e: MouseEvent) => {
    const cssBase = chatPageRef?.clientWidth ?? window.innerWidth;
    const wasCollapsed = isLeftCollapsed();
    const doCollapse = LEFT_RAIL_WIDTH_PX / cssBase; // 跟手最窄宽度（%）
    // 面板当前占用的像素宽：收起态 = 轨道宽；展开态实测（量不到时退回记忆宽度换算）
    const handleEl = e.currentTarget instanceof Element ? e.currentTarget : null;
    const measuredPx = handleEl?.parentElement?.getBoundingClientRect().width ?? 0;
    const basePx = wasCollapsed
      ? LEFT_RAIL_WIDTH_PX
      : measuredPx > 0
        ? measuredPx
        : (leftPanelWidth() / 100) * cssBase;
    // 光标坐标轴零点：光标横坐标减去它即为面板应占宽度
    const anchorX = e.clientX - basePx;
    setIsResizing(true);
    e.preventDefault();

    const handleMove = (moveEvent: MouseEvent) => {
      const pct = ((moveEvent.clientX - anchorX) / cssBase) * 100;
      setLeftDragWidth(Math.min(Math.max(pct, doCollapse), LEFT_MAX_WIDTH_PCT));
    };

    const stopResize = () => {
      setIsResizing(false);
      document.removeEventListener('mousemove', handleMove);
      document.removeEventListener('mouseup', stopResize);
      const dragged = leftDragWidth();
      setLeftDragWidth(null);
      if (dragged === null) return; // 按下未拖动：保持原状
      const draggedPx = (dragged / 100) * cssBase;
      if (draggedPx <= LEFT_RAIL_WIDTH_PX + LEFT_RELEASE_DEADZONE_PX) {
        setLeftCollapsed(true); // 顶到最窄/没真正拉开：吸附回收起轨道
      } else {
        setLeftPanelWidth(Math.max(dragged, 15));
        setLeftCollapsed(false);
      }
    };

    document.addEventListener('mousemove', handleMove);
    // 立即监听 mouseup：仅在把手上按下后直接松开（无 mousemove）时也能完成吸附
    document.addEventListener('mouseup', stopResize);
  };

  /**
   * 拖拽调整右侧面板宽度（原行为不变：仅展开态可用）。
   * @param e - MouseEvent 鼠标事件
   */
  const startResizeRight = (e: MouseEvent) => {
    attachResizeDrag(e, (pct) => {
      setRightPanelWidth(Math.min(Math.max(100 - pct, 15), 30));
    });
  };

  onMount(() => {
    const navigate = useNavigate();

    // 注册全部键盘/斜杠命令（19 个 registerCommand 已抽取为 hooks/chatCommands.ts）。
    const commandIds = registerChatCommands({
      isLeftCollapsed,
      setLeftCollapsed,
      isRightCollapsed,
      setIsRightCollapsed,
      rightPanelWidth,
      setRightPanelWidth,
      addTopic,
      setShowProjectCreateModal,
      switchToChat,
      switchToProject,
      isThinking,
      setIsThinking,
      handleStopGeneration,
      reasoningLevel,
      setReasoningLevel,
      webSearchEnabled,
      setWebSearchEnabled,
      datas,
      setDatas,
      projects,
      currentAssistant,
      currentAssistantId,
      currentTopicId,
      setCurrentTopicId,
      saveSingleAssistantToBackend,
      checkAndSummarize,
      navigate,
    });

    // ---- 原有初始化逻辑 ----
    // 首次进入：从 SQLite 加载所有助手数据
    loadAssistants()
      .then(async (loaded) => {
        let finalAssistants = [...loaded];

        // 1. 确保默认助手存在
        let defaultAsst = finalAssistants.find((a) => a.id === DEFAULT_ASST_ID);
        if (!defaultAsst) {
          // createAssistant 内部已经带了一个“默认话题”
          defaultAsst = createAssistant(t('chat.defaultAssistant'), DEFAULT_ASST_ID);
          finalAssistants = [defaultAsst, ...finalAssistants];
          setDatas('assistants', finalAssistants);
          await saveSingleAssistantToBackend(DEFAULT_ASST_ID);
        } else {
          setDatas('assistants', finalAssistants);
        }

        // 2. 处理应用启动时的默认选中（仅冷启动触发）
        if (isFirstAppLaunch) {
          // 尝试恢复上次的项目选择，否则默认选中对话
          const lastProjectId = getLastAgentProjectId();
          const lastProjectValid = lastProjectId && projects().find((p) => p.id === lastProjectId);
          if (lastProjectValid) {
            void switchToProject(lastProjectId!);
          } else {
            setCurrentAssistantId(DEFAULT_ASST_ID);

            const asst = datas.assistants.find((a) => a.id === DEFAULT_ASST_ID);
            if (asst && asst.topics.length > 0) {
              setCurrentTopicId(asst.topics[0].id);
            } else if (asst) {
              const newDefaultTopic = createTopic(t('chat.defaultTopic'));
              setDatas('assistants', (a) => a.id === DEFAULT_ASST_ID, 'topics', [newDefaultTopic]);
              setCurrentTopicId(newDefaultTopic.id);
              await saveSingleAssistantToBackend(DEFAULT_ASST_ID);
            }
          }

          isFirstAppLaunch = false;
        }
        console.log('成功加载数据');
      })
      .catch((err) => {
        // 如果后端报错，这里会打印出来
        console.error('加载助手列表失败:', err);
        console.error('load_all_assistants failed:', err);
        alert(t('error.load'));
      });

    // 注册全部事件监听器（18 个 listen 已抽取为 hooks/chatEventListeners.ts；
    // 存储 unlisten 函数用于组件卸载时清理）。
    const unlistens = registerChatEventListeners({
      setDatas,
      setIsDragging,
      setIsThinking,
      setPendingApprovals,
      setTypingIndex,
      setWorkflowState,
      handleFileUpload,
      datas,
      selectedModel,
      saveSingleAssistantToBackend,
      checkAndRename,
      checkAndSummarize,
    });

    // 组件卸载时清理所有事件监听和命令注册
    onCleanup(() => {
      unlistens.forEach((u) => u.then((fn) => fn()));
      // 卸载全部已注册命令（id 由 registerChatCommands 返回）
      commandIds.forEach(unregisterCommand);
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
  createEffect(() => {
    const pid = currentProjectId();
    if (!pid) {
      // 没有打开项目，清除诊断
      clearAllDiagnostics();
      return;
    }
    const project = currentProject();
    if (!project) return;

    void (async () => {
      try {
        // 自动检测语言
        const result = await autoDetectLsp(project.path);
        const languages: Array<{ languageId: string }> = result?.languages || [];

        // 为检测到的每种语言启动语言服务器
        for (const lang of languages) {
          try {
            await startLspServer(project.path, lang.languageId);
          } catch (e) {
            // 静默失败（语言服务器可能未安装）
            console.debug(`[LSP] 无法启动 ${lang.languageId} 服务器:`, e);
          }
        }
      } catch (e) {
        console.debug('[LSP] 自动检测失败:', e);
      }
    })();
  });

  // 项目切换时初始化 MCP 服务器（含内置文件系统 MCP 的自动连接）
  // 必须等 projects 加载完毕，否则 currentProject 为 null，MCP config 读不到
  createEffect(() => {
    const pid = currentProjectId();
    const loaded = projects().length > 0 || !pid;
    if (pid && loaded) {
      void initMcpServers(pid);
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
    <div
      class="h-full flex gap-[3px] px-[6px] pt-[1px] pb-[6px]"
      style={{ background: 'transparent' }}
      classList={{ 'is-resizing': isResizing() }}
      ref={chatPageRef}
    >
      <ProjectSidebar
        width={displayLeftWidth()}
        isCollapsed={leftPanelCollapsed()}
        onToggle={toggleLeft}
        onResize={(e) => startLeftResize(e as MouseEvent)}
        isResizing={isResizing()}
        onOpenSettings={(id) => setSettingsAsstId(id)}
      />

      <div class="flex-1 flex flex-col min-w-0 min-h-0">
        <Show when={workflowState()}>
          <WorkflowVisualization />
        </Show>

        {/* P4.4 检查点恢复入口：崩溃残留的回合提示 */}
        <Show when={activeCheckpoint() && !isThinking()}>
          <button
            type="button"
            class="flex items-center gap-2 px-4 py-1.5 text-xs border-b cursor-pointer bg-transparent hover:bg-white/[0.03] transition-colors"
            style={{ color: 'rgb(var(--primary-rgb))', 'border-color': 'var(--border-dim)' }}
            onClick={() => void handleResumeTurn()}
          >
            <Icon name="refresh" size={12} />
            <span>{t('agent.checkpoint.resume')}</span>
          </button>
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
          onResolveApproval={(id) =>
            setPendingApprovals((prev) => prev.filter((a) => a.approvalId !== id))
          }
          canShare={!!activeTopic()}
          onOpenShare={() => enterSelectionMode(null)}
          isSelectingMessages={isSelectingMessages()}
          selectedMessageIds={selectedMessageIds()}
          onToggleMessage={handleToggleMessage}
          onSelectAll={handleSelectAll}
          onCancelSelection={handleCancelSelection}
          onConfirmSelection={handleConfirmSelection}
          onBranchFromMessage={branchFromMessage}
          btwOverlays={btwOverlays()}
          onDismissBtw={(id) => setBtwOverlays((prev) => prev.filter((o) => o.id !== id))}
        />
      </div>

      <TopicSidebar
        width={displayRightWidth()}
        isCollapsed={isRightCollapsed()}
        onToggle={toggleRight}
        onResize={(e) => !isRightCollapsed() && startResizeRight(e)}
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
          onClose={() => {
            setShowShareModal(false);
            setActiveShareTopicId(null);
            setSelectedMessageIds(new Set<string>());
          }}
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
