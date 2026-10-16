/**
 * Harness workbench page — the CLI-agent GUI compatibility layer.
 *
 * Three-column layout mirroring the chat page (reuses usePanelResize + the
 * layout persistence helpers): left = two-level tool/workspace sidebar, middle =
 * harness conversation column, right = topic list. Data flows through
 * core/store/harness; the global listeners (registered in Layout) keep feeding
 * it while the user is on any page.
 */
import { Component, createEffect, createSignal, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import {
  loadPanelCollapsed,
  loadPanelWidth,
  savePanelCollapsed,
  savePanelWidth,
} from '../../core/store/layout';
import { usePanelResize } from '../chat/hooks/usePanelResize';
import {
  HARNESS_TOOL_NAMES,
  harnessAddWorkspace,
  harnessDeleteSession,
  harnessDetectTools,
  harnessListSessions,
  harnessListRunning,
  harnessListWorkspaces,
  harnessLoadMessages,
  harnessRemoveWorkspace,
  harnessRenameSession,
  harnessSendMessage,
  harnessStopSession,
} from '../../core/api/harness';
import {
  currentHarnessRun,
  currentHarnessTopicId,
  currentHarnessToolId,
  currentHarnessWorkspace,
  currentHarnessWorkspaceId,
  harnessMessages,
  harnessRunStates,
  harnessTopics,
  harnessTurnErrors,
  harnessWorkspaces,
  restoreHarnessSelection,
  selectHarnessTool,
  selectHarnessTopic,
  selectHarnessWorkspace,
  setHarnessMessages,
  setHarnessRunStates,
  setHarnessTopics,
  setHarnessTools,
  setHarnessTurnErrors,
  setHarnessWorkspaces,
  type HarnessRunInfo,
} from '../../core/store/harness';
import { t } from '../../core/i18n';
import { harnessNewSession } from '../../core/api/harness';
import HarnessToolSidebar from './components/HarnessToolSidebar';
import HarnessTopicSidebar from './components/HarnessTopicSidebar';
import HarnessChatInterface from './components/HarnessChatInterface';
import HarnessAddWorkspaceModal from './components/HarnessAddWorkspaceModal';

const HarnessPage: Component = () => {
  // ===== Panel layout (same persistence/pattern as the chat page) =====
  const [leftPanelWidth, setLeftPanelWidth] = createSignal(loadPanelWidth('left'));
  const [rightPanelWidth, setRightPanelWidth] = createSignal(loadPanelWidth('right'));
  const [isLeftCollapsed, setIsLeftCollapsed] = createSignal(loadPanelCollapsed('left'));
  const [isRightCollapsed, setIsRightCollapsed] = createSignal(loadPanelCollapsed('right'));
  const [inputMessage, setInputMessage] = createSignal('');
  const [sending, setSending] = createSignal(false);
  const [showAddWorkspace, setShowAddWorkspace] = createSignal(false);
  let pageRef: HTMLDivElement | undefined;

  const { isResizing, leftDragWidth, leftRevealMinPx, startLeftResize, startResizeRight } =
    usePanelResize({
      chatPageRef: () => pageRef,
      isLeftCollapsed,
      setLeftCollapsed: (collapsed: boolean) => {
        setIsLeftCollapsed(collapsed);
        savePanelCollapsed('left', collapsed);
        if (!collapsed && leftPanelWidth() < 5) setLeftPanelWidth(18);
      },
      leftPanelWidth,
      setLeftPanelWidth: (v: number) => {
        setLeftPanelWidth(v);
        savePanelWidth('left', v);
      },
      rightPanelWidth,
      setRightPanelWidth: (v: number) => {
        setRightPanelWidth(v);
        savePanelWidth('right', v);
      },
      isRightCollapsed,
    });

  const displayLeftWidth = () => leftDragWidth() ?? leftPanelWidth();
  const leftPanelCollapsed = () => isLeftCollapsed() && leftDragWidth() === null;
  const displayRightWidth = () => (isRightCollapsed() ? 0 : rightPanelWidth());

  // ===== Initial load + selection restore + running reconciliation =====
  const loadMessages = async (topicId: string) => {
    try {
      setHarnessMessages(await harnessLoadMessages(topicId));
    } catch (e) {
      console.error('[harness] load messages failed:', e);
    }
  };

  onMount(async () => {
    try {
      const [statuses, workspaces] = await Promise.all([
        harnessDetectTools(),
        harnessListWorkspaces(),
      ]);
      setHarnessTools(statuses.map((s) => ({ ...s, name: HARNESS_TOOL_NAMES[s.id] ?? s.id })));
      setHarnessWorkspaces(workspaces);
    } catch (e) {
      console.error('[harness] initial load failed:', e);
    }
    // Running-runtime reconciliation (agents survive page navigation)
    try {
      const running = await harnessListRunning();
      if (running.length > 0) {
        const next: Record<string, HarnessRunInfo> = { ...harnessRunStates() };
        for (const id of running) next[id] = { state: 'running' };
        setHarnessRunStates(next);
      }
    } catch {
      /* backend cold start — nothing running */
    }
    // Restore last selection (tool → workspace → topic)
    const sel = restoreHarnessSelection();
    if (sel.toolId && harnessWorkspaces().some((w) => w.toolId === sel.toolId)) {
      selectHarnessTool(sel.toolId);
      const ws = harnessWorkspaces().find((w) => w.id === sel.workspaceId);
      if (ws) {
        selectHarnessWorkspace(sel.workspaceId!);
        try {
          const topics = await harnessListSessions(ws.toolId, ws.path);
          setHarnessTopics(topics);
          if (sel.topicId && topics.some((t) => t.id === sel.topicId)) {
            selectHarnessTopic(sel.topicId);
          } else if (topics.length > 0) {
            selectHarnessTopic(topics[0].id);
          }
        } catch (e) {
          console.error('[harness] restore topics failed:', e);
        }
      }
    }
  });

  // ===== Topic list refresh on (tool, workspace) selection change =====
  createEffect(() => {
    const toolId = currentHarnessToolId();
    const wsId = currentHarnessWorkspaceId();
    if (!toolId || !wsId) return;
    const ws = harnessWorkspaces().find((w) => w.id === wsId && w.toolId === toolId);
    if (!ws) return;
    void (async () => {
      try {
        const topics = await harnessListSessions(ws.toolId, ws.path);
        setHarnessTopics(topics);
        if (!currentHarnessTopicId() && topics.length > 0) {
          selectHarnessTopic(topics[0].id);
        }
      } catch (e) {
        console.error('[harness] load topics failed:', e);
      }
    })();
  });

  // ===== Message load on topic switch =====
  createEffect(() => {
    const id = currentHarnessTopicId();
    if (!id) {
      setHarnessMessages([]);
      return;
    }
    void loadMessages(id);
  });

  // ===== Actions =====
  const isBusy = () => {
    const run = currentHarnessRun();
    return sending() || run.state === 'running' || run.state === 'connecting';
  };

  const handleSend = async () => {
    const text = inputMessage().trim();
    const topicId = currentHarnessTopicId();
    if (!text || !topicId || isBusy()) return;
    const topic = harnessTopics().find((t) => t.id === topicId);
    const firstTurn = !topic?.harnessSessionId;
    setSending(true);
    setInputMessage('');
    try {
      const stored = await harnessSendMessage(topicId, text);
      setHarnessMessages([...harnessMessages(), stored]);
      if (firstTurn) {
        const title = text.slice(0, 24);
        setHarnessTopics(harnessTopics().map((t) => (t.id === topicId ? { ...t, title } : t)));
        void harnessRenameSession(topicId, title).catch(() => undefined);
      }
    } catch (e) {
      setInputMessage(text); // restore the draft on failure
      setHarnessTurnErrors({ ...harnessTurnErrors(), [topicId]: String(e) });
    } finally {
      setSending(false);
    }
  };

  const handleStop = () => {
    const topicId = currentHarnessTopicId();
    if (topicId) void harnessStopSession(topicId).catch(console.error);
  };

  const handleAddTopic = async () => {
    const toolId = currentHarnessToolId();
    const ws = currentHarnessWorkspace();
    if (!toolId || !ws) return;
    try {
      const topic = await harnessNewSession(toolId, ws.path, t('harness.topic.untitled'));
      setHarnessTopics([topic, ...harnessTopics()]);
      selectHarnessTopic(topic.id);
    } catch (e) {
      console.error('[harness] new topic failed:', e);
    }
  };

  const handleRenameTopic = async (topicId: string, title: string) => {
    setHarnessTopics(harnessTopics().map((t) => (t.id === topicId ? { ...t, title } : t)));
    try {
      await harnessRenameSession(topicId, title);
    } catch (e) {
      console.error('[harness] rename topic failed:', e);
    }
  };

  const handleDeleteTopic = async (topicId: string) => {
    try {
      await harnessDeleteSession(topicId);
    } catch (e) {
      console.error('[harness] delete topic failed:', e);
      return;
    }
    const remaining = harnessTopics().filter((t) => t.id !== topicId);
    setHarnessTopics(remaining);
    if (currentHarnessTopicId() === topicId) {
      selectHarnessTopic(remaining[0]?.id ?? null);
    }
  };

  const handleRemoveWorkspace = async (workspaceId: string) => {
    try {
      await harnessRemoveWorkspace(workspaceId);
    } catch (e) {
      console.error('[harness] remove workspace failed:', e);
      return;
    }
    const remaining = harnessWorkspaces().filter((w) => w.id !== workspaceId);
    setHarnessWorkspaces(remaining);
    if (currentHarnessWorkspaceId() === workspaceId) {
      selectHarnessWorkspace(remaining[0]?.id ?? null);
    }
  };

  const handleAddWorkspace = async (path: string, name?: string) => {
    const toolId = currentHarnessToolId();
    if (!toolId) return;
    const ws = await harnessAddWorkspace(toolId, path, name);
    if (!harnessWorkspaces().some((w) => w.id === ws.id)) {
      setHarnessWorkspaces([...harnessWorkspaces(), ws]);
    }
    selectHarnessWorkspace(ws.id);
  };

  return (
    <div
      class="h-full flex gap-[3px] px-[6px] pt-[1px] pb-[6px]"
      style={{ background: 'transparent' }}
      classList={{ 'is-resizing': isResizing() }}
      ref={pageRef}
    >
      <HarnessToolSidebar
        width={displayLeftWidth()}
        isCollapsed={leftPanelCollapsed()}
        revealMinPx={leftRevealMinPx()}
        onToggle={(e) => {
          e.stopPropagation();
          const next = !isLeftCollapsed();
          setIsLeftCollapsed(next);
          savePanelCollapsed('left', next);
        }}
        onResize={(e) => startLeftResize(e)}
        isResizing={isResizing()}
        onAddWorkspace={() => setShowAddWorkspace(true)}
        onRemoveWorkspace={handleRemoveWorkspace}
      />

      <div class="flex-1 flex flex-col min-w-0 min-h-0">
        <HarnessChatInterface
          inputMessage={inputMessage()}
          setInputMessage={setInputMessage}
          isBusy={isBusy()}
          onSend={() => void handleSend()}
          onStop={handleStop}
          onOpenAddWorkspace={() => setShowAddWorkspace(true)}
        />
      </div>

      <HarnessTopicSidebar
        width={displayRightWidth()}
        isCollapsed={isRightCollapsed()}
        onToggle={(e) => {
          e.stopPropagation();
          const next = !isRightCollapsed();
          setIsRightCollapsed(next);
          savePanelCollapsed('right', next);
          if (!next && rightPanelWidth() < 5) {
            setRightPanelWidth(18);
            savePanelWidth('right', 18);
          }
        }}
        onResize={(e) => !isRightCollapsed() && startResizeRight(e)}
        onAddTopic={() => void handleAddTopic()}
        onRenameTopic={(id, title) => void handleRenameTopic(id, title)}
        onDeleteTopic={(id) => void handleDeleteTopic(id)}
        isResizing={isResizing()}
      />

      <Show when={showAddWorkspace()}>
        <Portal>
          <HarnessAddWorkspaceModal
            onClose={() => setShowAddWorkspace(false)}
            onAdd={(path, name) =>
              void handleAddWorkspace(path, name).then(() => setShowAddWorkspace(false))
            }
          />
        </Portal>
      </Show>
    </div>
  );
};

export default HarnessPage;
