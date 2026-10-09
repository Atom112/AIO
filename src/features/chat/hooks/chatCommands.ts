// Keyboard/slash command registration for ChatPage (19 registerCommand calls) — extracted from ChatPage.onMount.
//
// label/description strings go through t() (the command palette is user-facing).
//
// Returns the registered command ids (the component unregisters them in onCleanup).

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
  type CompactOutcome,
  type DatasState,
} from '../../../core/store/store';

/** Deps the ChatPage component provides to command registration (component-scope setters/signals/utilities). */
export interface ChatCommandDeps {
  // Panel collapse / width
  isLeftCollapsed: () => boolean;
  /** Single entry point for the left-panel collapsed state (persistence plus width fallback restore) */
  setLeftCollapsed: (collapsed: boolean) => void;
  isRightCollapsed: () => boolean;
  setIsRightCollapsed: (v: boolean) => void;
  rightPanelWidth: () => number;
  setRightPanelWidth: (v: number) => void;
  // Topic / project
  addTopic: () => Promise<void>;
  setShowProjectCreateModal: (v: boolean) => void;
  switchToChat: () => void;
  switchToProject: (id: string) => Promise<void>;
  // Generation control
  isThinking: () => boolean;
  setIsThinking: (v: boolean) => void;
  handleStopGeneration: () => Promise<void>;
  // Mode & search
  reasoningLevel: () => 'off' | 'low' | 'medium' | 'high';
  setReasoningLevel: (v: 'off' | 'low' | 'medium' | 'high') => void;
  webSearchEnabled: () => boolean;
  setWebSearchEnabled: (v: boolean) => void;
  // Data access
  datas: DatasState;
  setDatas: typeof storeSetDatas;
  projects: () => { id: string; name: string }[];
  currentAssistant: () => Assistant | undefined;
  currentAssistantId: () => string | null;
  currentTopicId: () => string | null;
  setCurrentTopicId: (id: string | null) => void;
  saveSingleAssistantToBackend: (id: string) => Promise<void>;
  /** force=true skips the auto-compaction threshold guard (manual /compact runs immediately) */
  checkAndSummarize: (
    keepBudgetOverride?: number,
    opts?: { force?: boolean },
  ) => Promise<CompactOutcome>;
  navigate: (path: string) => void;
}

/** Registers all shortcuts and slash commands; returns the registered command ids (for cleanup). */
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
      // Ordering: [chat, ...projects]
      const projList = projects();
      const currentProjId = currentAssistant()?.projectId;
      if (!currentProjId) {
        // In chat mode: previous = last project
        if (projList.length > 0) {
          const lastProj = projList[projList.length - 1];
          void switchToProject(lastProj.id);
        }
      } else {
        const idx = projList.findIndex((p) => p.id === currentProjId);
        if (idx > 0) {
          void switchToProject(projList[idx - 1].id);
        } else {
          // First project: previous = chat
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
        // In chat mode: next = first project
        if (projList.length > 0) {
          void switchToProject(projList[0].id);
        }
      } else {
        const idx = projList.findIndex((p) => p.id === currentProjId);
        if (idx >= 0 && idx < projList.length - 1) {
          void switchToProject(projList[idx + 1].id);
        } else {
          // Last project: next = chat
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

  // ---- Slash command handlers (override the no-ops in shortcuts.ts) ----
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

  /** Maps a compaction outcome to user-facing feedback text (compressed / archived fallback / skipped / failed). */
  const compactOutcomeFeedback = (outcome: CompactOutcome): string => {
    switch (outcome.kind) {
      case 'compressed':
        return `✓ /compact — ${t('slash.feedback.compactDone', {
          summarized: outcome.summarizedCount,
          kept: outcome.keptCount,
        })}`;
      case 'archived':
        return `✓ /compact — ${t('slash.feedback.compactArchived', {
          count: outcome.count,
        })}`;
      case 'skipped':
        return `✓ /compact — ${t('slash.feedback.compactSkipped')}`;
      case 'failed':
        return `✗ /compact — ${t('slash.feedback.compactFailed', { error: outcome.error })}`;
    }
  };

  const cmdSlashCompact = registerCommand({
    id: 'slash-compact',
    handler: async (args?: string) => {
      // Optional arg: kept token budget (`/compact 10000`), default 20k;
      // manual execution skips the auto-compaction 75% threshold guard and compacts immediately (matches mainstream agent behavior)
      const budgetArg = args ? Number(args.trim()) : NaN;
      const budget = Number.isFinite(budgetArg) && budgetArg > 0 ? budgetArg : undefined;
      setIsThinking(true);
      try {
        const outcome = await checkAndSummarize(budget, { force: true });
        return compactOutcomeFeedback(outcome);
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
  // Collect all command ids so the caller can unregister them in onCleanup
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
