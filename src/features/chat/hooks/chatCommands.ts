// ChatPage 的键盘/斜杠命令注册（19 个 registerCommand）—— 从 ChatPage.onMount 抽出。
//
// 抽取策略与 chatEventListeners 一致：函数顶部从 deps 解构依赖，命令主体逐字迁移；
// label/description 走 t() 取词（命令面板为面向用户界面）。
//
// 返回注册返回的 id 列表（组件 onCleanup 时 unregister）。

import {
  getCommandDisplayDescription,
  getSlashCommands,
  registerCommand,
} from '../../../core/shortcuts';
import { t } from '../../../core/i18n';
import {
  setDatas as storeSetDatas,
  type AgentMode,
  type Assistant,
  type DatasState,
} from '../../../core/store/store';

/** ChatPage 提供给命令注册的依赖包（组件作用域的 setter/信号/工具）。 */
export interface ChatCommandDeps {
  // 面板折叠/宽度
  isLeftCollapsed: () => boolean;
  /** 左侧面板收起状态统一入口（含持久化与宽度兜底恢复） */
  setLeftCollapsed: (collapsed: boolean) => void;
  isRightCollapsed: () => boolean;
  setIsRightCollapsed: (v: boolean) => void;
  rightPanelWidth: () => number;
  setRightPanelWidth: (v: number) => void;
  // 话题/项目
  addTopic: () => Promise<void>;
  setShowProjectCreateModal: (v: boolean) => void;
  switchToChat: () => void;
  switchToProject: (id: string) => Promise<void>;
  // 生成控制
  isThinking: () => boolean;
  setIsThinking: (v: boolean) => void;
  handleStopGeneration: () => Promise<void>;
  // 模式与搜索
  reasoningLevel: () => 'off' | 'low' | 'medium' | 'high';
  setReasoningLevel: (v: 'off' | 'low' | 'medium' | 'high') => void;
  webSearchEnabled: () => boolean;
  setWebSearchEnabled: (v: boolean) => void;
  // 数据访问
  datas: DatasState;
  setDatas: typeof storeSetDatas;
  projects: () => { id: string; name: string }[];
  currentAssistant: () => Assistant | undefined;
  currentAssistantId: () => string | null;
  currentTopicId: () => string | null;
  setCurrentTopicId: (id: string | null) => void;
  saveSingleAssistantToBackend: (id: string) => Promise<void>;
  checkAndSummarize: (keepBudgetOverride?: number) => Promise<void>;
  navigate: (path: string) => void;
}

/** 注册全部快捷键与斜杠命令，返回已注册的命令 id 列表（用于清理）。 */
export function registerChatCommands(deps: ChatCommandDeps): string[] {
  const {
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
  } = deps;

  const ids: string[] = [];
  const cmdToggleLeft = registerCommand({
    id: 'toggle-left-sidebar',
    handler: () => {
      setLeftCollapsed(!isLeftCollapsed());
    },
  });

  const cmdToggleRight = registerCommand({
    id: 'toggle-right-sidebar',
    handler: () => {
      const newState = !isRightCollapsed();
      setIsRightCollapsed(newState);
      localStorage.setItem('right-collapsed', String(newState));
      if (!newState && rightPanelWidth() < 5) setRightPanelWidth(18);
    },
  });

  const cmdNewTopic = registerCommand({
    id: 'new-topic',
    handler: () => {
      addTopic();
    },
  });

  const cmdNewProject = registerCommand({
    id: 'new-project',
    handler: () => {
      setShowProjectCreateModal(true);
    },
  });

  const cmdStopGen = registerCommand({
    id: 'stop-generation',
    handler: () => {
      if (isThinking()) handleStopGeneration();
    },
  });

  const cmdPrevAsst = registerCommand({
    id: 'prev-assistant',
    category: 'sidebar',
    handler: () => {
      // 构建顺序: [chat, ...projects]
      const projList = projects();
      const currentProjId = currentAssistant()?.projectId;
      if (!currentProjId) {
        // 当前是聊天模式，上一个 = 最后一个项目
        if (projList.length > 0) {
          const lastProj = projList[projList.length - 1];
          void switchToProject(lastProj.id);
        }
      } else {
        const idx = projList.findIndex((p) => p.id === currentProjId);
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
    category: 'sidebar',
    handler: () => {
      const projList = projects();
      const currentProjId = currentAssistant()?.projectId;
      if (!currentProjId) {
        // 当前是聊天模式，下一个 = 第一个项目
        if (projList.length > 0) {
          void switchToProject(projList[0].id);
        }
      } else {
        const idx = projList.findIndex((p) => p.id === currentProjId);
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
    category: 'sidebar',
    handler: () => {
      const asst = datas.assistants.find((a: any) => a.id === currentAssistantId());
      if (!asst) return;
      const idx = asst.topics.findIndex((t: any) => t.id === currentTopicId());
      if (idx > 0) setCurrentTopicId(asst.topics[idx - 1].id);
    },
  });

  const cmdNextTopic = registerCommand({
    id: 'next-topic',
    category: 'sidebar',
    handler: () => {
      const asst = datas.assistants.find((a: any) => a.id === currentAssistantId());
      if (!asst) return;
      const idx = asst.topics.findIndex((t: any) => t.id === currentTopicId());
      if (idx >= 0 && idx < asst.topics.length - 1) setCurrentTopicId(asst.topics[idx + 1].id);
    },
  });

  const cmdGoSettings = registerCommand({
    id: 'go-to-settings',
    handler: () => {
      navigate('/settings/app');
    },
  });

  const cmdGoChat = registerCommand({
    id: 'go-to-chat',
    handler: () => {
      navigate('/chat');
    },
  });

  const cmdToggleSearch = registerCommand({
    id: 'toggle-web-search',
    handler: () => {
      const next = !webSearchEnabled();
      setWebSearchEnabled(next);
      localStorage.setItem('chat-web-search', String(next));
    },
  });

  const cmdCycleReasoning = registerCommand({
    id: 'cycle-reasoning',
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
    handler: async () => {
      const id = currentAssistantId();
      if (!id) return;
      const asst = datas.assistants.find((a) => a.id === id);
      const current: AgentMode = asst?.agentMode ?? 'off';
      const modes: AgentMode[] = ['off', 'normal', 'auto', 'plan'];
      const idx = modes.indexOf(current);
      const next = modes[(idx + 1) % modes.length];
      setDatas('assistants', (a) => a.id === id, 'agentMode', next);
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
      if (!confirm(t('chat.clearConfirm'))) return;
      setDatas(
        'assistants',
        (a: any) => a.id === asstId,
        'topics',
        (t: any) => t.id === topicId,
        'history',
        [],
      );
      saveSingleAssistantToBackend(asstId);
    },
  });

  const cmdSlashCompact = registerCommand({
    id: 'slash-compact',
    handler: async (args?: string) => {
      // 可选参数：保留的 token 预算（`/compact 10000`），默认 2 万
      const budgetArg = args ? Number(args.trim()) : NaN;
      setIsThinking(true);
      try {
        await checkAndSummarize(
          Number.isFinite(budgetArg) && budgetArg > 0 ? budgetArg : undefined,
        );
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
    handler: () => {
      navigate('/settings/app');
    },
  });

  const cmdSlashHelp = registerCommand({
    id: 'slash-help',
    handler: () => {
      const asstId = currentAssistantId();
      const topicId = currentTopicId();
      if (!asstId || !topicId) return;
      const allSlash = getSlashCommands();
      const lines = allSlash.map((c) => `- **${c.label}** — ${getCommandDisplayDescription(c)}`);
      const helpText = `## ${t('slash.help.title')}\n\n${lines.join('\n')}`;
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
            content: helpText,
          },
        ],
      );
    },
  });
  // 收集全部命令 id 供调用方在 onCleanup 时注销
  ids.push(
    cmdToggleLeft,
    cmdToggleRight,
    cmdNewTopic,
    cmdNewProject,
    cmdStopGen,
    cmdPrevAsst,
    cmdNextAsst,
    cmdPrevTopic,
    cmdNextTopic,
    cmdGoSettings,
    cmdGoChat,
    cmdToggleSearch,
    cmdCycleReasoning,
    cmdCycleAgent,
    cmdSlashClear,
    cmdSlashCompact,
    cmdSlashSearch,
    cmdSlashSettings,
    cmdSlashHelp,
  );

  return ids;
}
