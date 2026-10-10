import { Component, For, Show, createEffect, createSignal, onMount, onCleanup } from 'solid-js';
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

  // ---- Branch fold state ----
  // Component-local, default expanded. Auto-expand fires when a folded topic
  // gains a new child (e.g. branching from a message while the subtree is
  // folded). Switching assistants resets fold state and counters wholesale.
  const [collapsedTopicIds, setCollapsedTopicIds] = createSignal<Set<string>>(new Set());
  let prevChildCounts = new Map<string, number>();
  let foldAssistantId: string | undefined;
  createEffect(() => {
    const asst = props.currentAssistant;
    if (!asst) return;
    if (asst.id !== foldAssistantId) {
      foldAssistantId = asst.id;
      prevChildCounts = new Map();
      if (collapsedTopicIds().size > 0) setCollapsedTopicIds(new Set<string>());
      return;
    }
    const counts = new Map<string, number>();
    for (const tp of asst.topics) {
      if (tp.parentTopicId) counts.set(tp.parentTopicId, (counts.get(tp.parentTopicId) ?? 0) + 1);
    }
    const next = new Set(collapsedTopicIds());
    let changed = false;
    for (const [pid, n] of counts) {
      if (n > (prevChildCounts.get(pid) ?? 0) && next.has(pid)) {
        next.delete(pid);
        changed = true;
      }
    }
    prevChildCounts = counts;
    if (changed) setCollapsedTopicIds(next);
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
                  // Render the topic tree recursively. Connector geometry bridges the
                  // list gaps so the vertical rail reads as one continuous line:
                  // - isLast / isFirst: sibling position in the parent's child list;
                  // - railBelowInherited: the rail continues past this whole subtree
                  //   (a parent's next sibling exists somewhere up the chain). Root
                  //   subtrees are hard-cut — never bridge across root topics.
                  // - folded: this topic's children are collapsed; the rail treats
                  //   it as a leaf and the children block animates to zero height.
                  const renderTopic = (
                    topic: Topic,
                    depth: number,
                    isLast = true,
                    isFirst = false,
                    railBelowInherited = false,
                  ): any => {
                    const isChild = depth > 0;
                    const kids = children.get(topic.id) ?? [];
                    const hasKids = kids.length > 0;
                    // Gap between this topic's children block and the rows
                    // above/below it (root children get space-y-1's 4px, deeper
                    // nesting space-y-0.5's 2px).
                    const childGap = depth === 0 ? 4 : 2;
                    // upExt: the first child reaches up to the parent row's bottom
                    // edge. Non-first siblings also extend 2px up: the fold wrapper
                    // clips the previous sibling's 2px stem overhang below its own
                    // row, so each next row re-draws that strip from its side and
                    // the rail stays continuous under clipping.
                    const parentGap = depth === 1 ? 4 : 2;
                    const upExt = isChild ? (isFirst ? parentGap : 2) : 0;
                    const midY = upExt + 14;
                    // Fold state must be read ONLY inside the closures / JSX
                    // attribute expressions below, never directly in this body:
                    // the whole IIFE is a memo, so a body-level signal read would
                    // rebuild the entire subtree on every toggle — fresh DOM nodes
                    // carry no transition, which is why the fold animation never
                    // played. Closure reads compile to fine-grained per-attribute
                    // effects that update the SAME nodes in place, letting the
                    // grid-template-rows / opacity transitions actually run.
                    const isFolded = () => hasKids && collapsedTopicIds().has(topic.id);
                    // stemOn: the rail continues below this row — toward the next
                    // sibling, into this row's own (expanded) children, or toward
                    // an uncle (inherited). Folded children do not continue the
                    // rail; without a stem the line ends at the row's elbow (L).
                    const stemOn = () =>
                      isChild && (!isLast || (hasKids && !isFolded()) || railBelowInherited);
                    const railH = () => 28 + upExt + (stemOn() ? 2 : 0);
                    const connectorPath = () =>
                      stemOn()
                        ? `M5 0 V${railH()} M5 ${upExt + 10} Q5 ${midY} 9 ${midY} H22`
                        : `M5 0 V${upExt + 10} Q5 ${midY} 9 ${midY} H22`;
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
                          {/* Branch marker for child topics: rounded elbow connector.
                              The SVG may overflow the row box (margin-top negative /
                              extra height) so the vertical rail spans the list gaps
                              and connects with the rows above and below. */}
                          {isChild && (
                            <svg
                              viewBox={`0 0 22 ${railH()}`}
                              width="22"
                              height={railH()}
                              class="shrink-0 self-start"
                              style={upExt ? { 'margin-top': `-${upExt}px` } : undefined}
                              fill="none"
                              stroke="rgba(var(--text-base-rgb),0.35)"
                              stroke-width="1.5"
                              stroke-linecap="round"
                              aria-hidden="true"
                            >
                              <title>{t('chat.branchTopic')}</title>
                              <path d={connectorPath()} />
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
                          <div class="flex items-center gap-0.5 shrink-0">
                            {/* Fold toggle: only for topics with children, hover-only
                                visibility (same as the more button). Caret icons:
                                v (caret-down) when expanded, ^ (caret-up) when
                                folded. */}
                            <Show when={hasKids}>
                              <button
                                type="button"
                                class="flex items-center justify-center border-none bg-transparent cursor-pointer rounded-md text-white/40 hover:text-white/85 transition-all duration-200 active:scale-90 opacity-0 group-hover:opacity-100"
                                style={{
                                  width: isChild ? '18px' : '24px',
                                  height: isChild ? '18px' : '24px',
                                }}
                                title={
                                  isFolded() ? t('chat.expandBranches') : t('chat.collapseBranches')
                                }
                                aria-expanded={!isFolded()}
                                aria-label={
                                  isFolded() ? t('chat.expandBranches') : t('chat.collapseBranches')
                                }
                                onClick={(e) => {
                                  e.stopPropagation();
                                  setCollapsedTopicIds((prev) => {
                                    const next = new Set(prev);
                                    if (next.has(topic.id)) next.delete(topic.id);
                                    else next.add(topic.id);
                                    return next;
                                  });
                                }}
                              >
                                <Icon
                                  name={isFolded() ? 'caret-up' : 'caret-down'}
                                  size={isChild ? 13 : 15}
                                />
                              </button>
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
                        </div>
                        {/* Render child topics recursively. The grid wrapper animates the
                            fold via grid-template-rows 0fr↔1fr; its negative top margin
                            cancels the list gap and the inner padding restores it, so the
                            connector bridge stays inside the clipped box and rows land
                            exactly where they did before the wrapper existed. */}
                        <Show when={hasKids}>
                          <div
                            class="grid transition-[grid-template-rows] duration-200 ease-out"
                            style={{
                              'margin-top': `-${childGap}px`,
                              'grid-template-rows': isFolded() ? '0fr' : '1fr',
                            }}
                          >
                            <div class="overflow-hidden min-h-0">
                              <div
                                class="space-y-0.5 transition-opacity duration-200"
                                style={{
                                  'padding-top': `${childGap}px`,
                                  opacity: isFolded() ? '0' : '1',
                                }}
                              >
                                <For each={children.get(topic.id)}>
                                  {(child) => {
                                    const siblings = children.get(topic.id) ?? [];
                                    const isLastSibling =
                                      siblings[siblings.length - 1]?.id === child.id;
                                    const isFirstSibling = siblings[0]?.id === child.id;
                                    // Children inherit the rail-below flag: the rail
                                    // continues past this subtree when this topic has a
                                    // next sibling, or when the flag was inherited from
                                    // above. Root subtrees never bridge across roots.
                                    const kidsRailBelow =
                                      depth === 0 ? false : !isLast || railBelowInherited;
                                    return renderTopic(
                                      child,
                                      depth + 1,
                                      isLastSibling,
                                      isFirstSibling,
                                      kidsRailBelow,
                                    );
                                  }}
                                </For>
                              </div>
                            </div>
                          </div>
                        </Show>
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
