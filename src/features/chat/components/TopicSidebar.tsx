import { Component, For, Show, createSignal, onMount, onCleanup } from 'solid-js';
import { Portal } from 'solid-js/web';
import {
  Assistant,
  Topic,
  datas,
  setDatas,
  currentTopicId,
  setCurrentTopicId,
  saveSingleAssistantToBackend,
  requestRenameTopic,
} from '../../../core/store/store';
import Icon from '../../../shared/components/Icon';
import { formatDateTime, t } from '../../../core/i18n';

interface TopicSidebarProps {
  width: number;
  onResize: (e: MouseEvent) => void;
  currentAssistant: Assistant | undefined;
  editingTopicId: string | null;
  setEditingTopicId: (id: string | null) => void;
  addTopic: () => void;
  isCollapsed: boolean;
  onToggle: (e: MouseEvent) => void;
  isResizing: boolean;
  /** Opens the export dialog for the given topic */
  onExportTopic: (topicId: string) => void;
}

const createTopic = (name?: string): Topic => ({
  id: Date.now().toString(),
  name:
    name ||
    t('chat.untitledTopic', {
      time: formatDateTime(Date.now(), { hour: '2-digit', minute: '2-digit' }),
    }),
  history: [],
  summary: '',
});

/** Build a topic tree from parentTopicId. Returns [root topic list, children map (parentId → children)] */
const buildTopicTree = (topics: Topic[]): { roots: Topic[]; children: Map<string, Topic[]> } => {
  const children = new Map<string, Topic[]>();
  const roots: Topic[] = [];
  for (const t of topics) {
    if (t.parentTopicId) {
      const list = children.get(t.parentTopicId) || [];
      list.push(t);
      children.set(t.parentTopicId, list);
    } else {
      roots.push(t);
    }
  }
  return { roots, children };
};

const TopicSidebar: Component<TopicSidebarProps> = (props) => {
  const [showTopicMenuDiv, setShowTopicMenuDiv] = createSignal(false);
  const [isTopicMenuAnimatingOut, setIsTopicMenuAnimatingOut] = createSignal(false);
  const [topicMenuState, setTopicMenuState] = createSignal({
    isOpen: false,
    x: 0,
    y: 0,
    targetTopicId: null as string | null,
  });

  onMount(() => {
    const h = () => {
      if (topicMenuState().isOpen) closeTopicMenu();
    };
    window.addEventListener('click', h);
    onCleanup(() => window.removeEventListener('click', h));
  });

  const saveTopicRename = async (asstId: string, topicId: string, newName: string) => {
    if (!newName.trim()) return props.setEditingTopicId(null);
    // Mark manual renames as renamed so the auto title won't overwrite them later
    setDatas(
      'assistants',
      (a) => a.id === asstId,
      'topics',
      (t) => t.id === topicId,
      {
        name: newName,
        renamed: true,
      },
    );
    await saveSingleAssistantToBackend(asstId);
    props.setEditingTopicId(null);
  };

  const openTopicMenu = (e: MouseEvent, topicId: string) => {
    e.stopPropagation();
    setShowTopicMenuDiv(true);
    setIsTopicMenuAnimatingOut(false);
    const rect = (e.currentTarget as Element).getBoundingClientRect();
    setTopicMenuState({
      isOpen: true,
      x: rect.left - 100,
      y: rect.top + rect.height,
      targetTopicId: topicId,
    });
  };

  const closeTopicMenu = () => {
    setTopicMenuState((p) => ({ ...p, isOpen: false }));
    setIsTopicMenuAnimatingOut(true);
    setTimeout(() => {
      setShowTopicMenuDiv(false);
      setIsTopicMenuAnimatingOut(false);
    }, 200);
  };

  const deleteTopic = async (asstId: string, topicId: string) => {
    const asst = datas.assistants.find((a) => a.id === asstId);
    if (!asst) return;
    if (asst.topics.length <= 1) {
      const newT = createTopic(t('chat.defaultTopic'));
      setDatas('assistants', (a) => a.id === asstId, 'topics', [newT]);
      setCurrentTopicId(newT.id);
    } else {
      setDatas(
        'assistants',
        (a) => a.id === asstId,
        'topics',
        (topics: any[]) => topics.filter((t: Topic) => t.id !== topicId),
      );
      if (currentTopicId() === topicId) {
        const remaining = asst.topics.filter((t: Topic) => t.id !== topicId);
        if (remaining.length > 0) setCurrentTopicId(remaining[0].id);
      }
    }
    await saveSingleAssistantToBackend(asstId);
    closeTopicMenu();
  };

  /**
   * Manually regenerate the topic title.
   * Only available for non-default topics (the default topic is never renamed).
   * Notifies ChatPage via `requestRenameTopic` to perform the actual LLM call.
   */
  const handleRegenerateTitle = () => {
    const asst = props.currentAssistant;
    const targetId = topicMenuState().targetTopicId;
    if (!asst || !targetId) return;
    const target = asst.topics.find((t: Topic) => t.id === targetId);
    if (!target) return;
    const ok = requestRenameTopic(asst.id, target.id);
    if (!ok) {
      // Default topic: the menu item should already be hidden; this is a defensive warning
      console.warn('默认话题不支持重新生成标题');
    }
    closeTopicMenu();
  };

  /**
   * Whether the context-menu target topic is the default topic.
   * The default topic hides the "regenerate title" menu item.
   */
  const isMenuTargetDefault = (): boolean => {
    const asst = props.currentAssistant;
    const targetId = topicMenuState().targetTopicId;
    if (!asst || !targetId) return true;
    const target = asst.topics.find((t: Topic) => t.id === targetId);
    if (!target) return true;
    return asst.topics[0]?.id === target.id;
  };

  return (
    <div
      class="relative flex flex-col flex-shrink-0 min-w-0 z-10"
      style={{
        width: props.isCollapsed ? '0%' : `${props.width}%`,
        padding: props.isCollapsed ? '0' : '15px',
        transition: props.isResizing ? 'none' : 'all 0.3s cubic-bezier(0.4, 0, 0.2, 1)',
      }}
      onContextMenu={(e) => e.preventDefault()}
    >
      {/* Faint divider between the three columns: sits on the panel edge (+/-0.5px), centered exactly with the resize handle below; hidden when collapsed */}
      <Show when={!props.isCollapsed}>
        <span
          aria-hidden="true"
          class="absolute top-0 bottom-0 w-px pointer-events-none"
          style={{ left: '-0.5px', background: 'rgba(var(--text-base-rgb),0.12)' }}
        />
      </Show>

      {/* Resize handle (left): 6px grab strip centered on the panel edge (concentric with the divider line) */}
      <div
        class="absolute top-0 bottom-0 left-[-3px] w-[6px] flex items-center justify-center cursor-ew-resize z-[1000] group"
        onMouseDown={(e) => props.onResize(e as MouseEvent)}
      >
        <div
          class="absolute z-[1001] w-[10px] h-12 rounded-[20px] backdrop-blur-md cursor-pointer flex items-center justify-center text-xs font-bold transition-all duration-200 opacity-0 group-hover:opacity-100 hover:scale-110"
          style={{
            background: 'rgba(var(--text-base-rgb),0.08)',
            color: 'rgba(var(--text-base-rgb),0.6)',
            'box-shadow': '0 2px 8px rgba(0,0,0,0.3)',
          }}
          title={props.isCollapsed ? t('chat.expandTopics') : t('chat.collapseTopics')}
          onClick={(e) => {
            e.stopPropagation();
            props.onToggle(e);
          }}
        >
          <Icon name={props.isCollapsed ? 'chevron-left' : 'chevron-right'} size={10} />
        </div>
      </div>

      <div
        class="h-full w-full overflow-hidden hover:overflow-y-auto transition-opacity duration-300"
        classList={{ 'opacity-0 pointer-events-none overflow-hidden': props.isCollapsed }}
      >
        <Show when={props.currentAssistant}>
          {(asst) => (
            <div class="flex flex-col h-full">
              <button
                class="w-full px-3 h-12 inline-flex items-center justify-center rounded-3xl cursor-pointer transition-all duration-300"
                style={{
                  background: 'rgba(var(--text-base-rgb),0.04)',
                  border: '1px solid var(--border-dim)',
                  color: 'rgba(var(--text-base-rgb),0.6)',
                }}
                onClick={props.addTopic}
                onMouseEnter={(e) =>
                  (e.currentTarget.style.background = 'rgba(var(--primary-rgb),0.12)')
                }
                onMouseLeave={(e) =>
                  (e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.04)')
                }
              >
                {t('chat.newTopic')}
              </button>
              <div class="mt-[15px] space-y-1">
                {(() => {
                  const { roots, children } = buildTopicTree(asst().topics);
                  // Render the topic tree recursively
                  const renderTopic = (topic: Topic, depth: number): any => {
                    const isChild = depth > 0;
                    return (
                      <>
                        <div
                          class={`group flex items-center justify-between cursor-pointer rounded-2xl transition-all duration-200 text-white/75 hover:bg-white/[0.06] ${isChild ? 'ml-5 h-7 px-1.5 bg-transparent border-transparent' : 'px-3 h-12 rounded-3xl bg-white/[0.03] border border-white/[0.04]'}`}
                          classList={{
                            '!bg-[rgba(var(--primary-rgb),0.22)] !border-[rgba(var(--primary-rgb),0.22)]':
                              topic.id === currentTopicId(),
                          }}
                          onClick={() => setCurrentTopicId(topic.id)}
                        >
                          {/* Branch marker for child topics: rounded elbow connector */}
                          {isChild && (
                            <svg
                              viewBox="0 0 22 28"
                              width="22"
                              height="28"
                              class="shrink-0 self-start"
                              fill="none"
                              stroke="rgba(var(--text-base-rgb),0.35)"
                              stroke-width="1.5"
                              stroke-linecap="round"
                              aria-hidden="true"
                            >
                              <title>{t('chat.branchTopic')}</title>
                              <path d="M5 0 V10 Q5 14 9 14 H22" />
                            </svg>
                          )}
                          <Show
                            when={props.editingTopicId === topic.id}
                            fallback={
                              <span
                                style={{
                                  color: `rgba(var(--text-base-rgb),${isChild ? '0.45' : '0.75'})`,
                                  'font-size': isChild ? '0.78rem' : '0.9rem',
                                  overflow: 'hidden',
                                  'text-overflow': 'ellipsis',
                                  'white-space': 'nowrap',
                                  'padding-right': '8px',
                                  'user-select': 'none',
                                }}
                              >
                                {topic.name}
                              </span>
                            }
                          >
                            <input
                              class="rounded px-2 py-0.5 text-[0.85rem] h-5 outline-none w-[80%]"
                              style={{
                                background: 'rgba(0,0,0,0.3)',
                                border: '1px solid var(--border-dim)',
                                color: 'rgba(var(--text-base-rgb),0.85)',
                              }}
                              value={topic.name}
                              ref={(el) => {
                                setTimeout(() => {
                                  el.focus();
                                  el.select();
                                }, 0);
                              }}
                              onBlur={(e) =>
                                saveTopicRename(asst().id, topic.id, e.currentTarget.value)
                              }
                              onKeyDown={(e) =>
                                e.key === 'Enter' &&
                                saveTopicRename(asst().id, topic.id, e.currentTarget.value)
                              }
                              onClick={(e) => e.stopPropagation()}
                            />
                          </Show>
                          <button
                            class="flex items-center justify-center border-none rounded-full cursor-pointer transition-all duration-200 active:scale-90 opacity-0 group-hover:opacity-100 bg-white/[0.06] text-white/60 hover:bg-pri-10"
                            style={{
                              width: isChild ? '22px' : '30px',
                              height: isChild ? '22px' : '30px',
                            }}
                            onClick={(e) => openTopicMenu(e as MouseEvent, topic.id)}
                          >
                            <Icon
                              src="/icons/app-logo/dot-menu.svg"
                              class={isChild ? 'w-[13px] h-[13px]' : 'w-[18px] h-[18px]'}
                            />
                          </button>
                        </div>
                        {/* Render child topics recursively - tighter spacing */}
                        <div class="space-y-0.5">
                          {
                            <For each={children.get(topic.id)}>
                              {(child) => renderTopic(child, depth + 1)}
                            </For>
                          }
                        </div>
                      </>
                    );
                  };
                  return roots.map((root) => renderTopic(root, 0));
                })()}
              </div>
            </div>
          )}
        </Show>
      </div>

      {showTopicMenuDiv() && (
        <Portal>
          <div
            class="fixed z-[1500] min-w-[150px] rounded-lg shadow-[0_12px_40px_rgba(0,0,0,0.35)] py-1 origin-top-left"
            style={{
              top: `${topicMenuState().y}px`,
              left: `${topicMenuState().x}px`,
              background: 'var(--acrylic-bg)',
              'backdrop-filter': 'blur(24px) saturate(150%)',
              border: '1px solid var(--acrylic-border)',
              animation: isTopicMenuAnimatingOut()
                ? 'contextMenuOut 0.14s ease-in forwards'
                : 'contextMenuIn 0.18s cubic-bezier(0.2, 0.8, 0.2, 1) forwards',
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <button
              class="w-full text-left px-3 py-2 bg-transparent border-none cursor-pointer rounded-lg transition-all duration-200 text-white/75 hover:bg-pri-10 hover:text-white"
              onClick={() => {
                props.setEditingTopicId(topicMenuState().targetTopicId);
                closeTopicMenu();
              }}
            >
              {t('chat.renameTopic')}
            </button>
            <Show when={!isMenuTargetDefault()}>
              <button
                class="w-full text-left px-3 py-2 bg-transparent border-none cursor-pointer rounded-lg transition-all duration-200 text-white/75 hover:bg-pri-10 hover:text-white"
                onClick={handleRegenerateTitle}
              >
                {t('chat.regenerateTitle')}
              </button>
            </Show>
            <button
              class="w-full text-left px-3 py-2 bg-transparent border-none cursor-pointer rounded-lg transition-all duration-200 text-white/75 hover:bg-pri-10 hover:text-white"
              onClick={() => {
                props.onExportTopic(topicMenuState().targetTopicId!);
                closeTopicMenu();
              }}
            >
              {t('chat.export')}
            </button>
            <button
              class="w-full text-left px-3 py-2 bg-transparent border-none cursor-pointer rounded-lg transition-all duration-200 text-white/75 hover:bg-pri-10 hover:text-white"
              style={{ color: 'rgba(255,77,77,0.8)' }}
              onClick={() =>
                deleteTopic(props.currentAssistant!.id, topicMenuState().targetTopicId!)
              }
            >
              {t('chat.deleteTopic')}
            </button>
          </div>
        </Portal>
      )}
    </div>
  );
};

export default TopicSidebar;
