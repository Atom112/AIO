/**
 * Harness topic sidebar — strict replica of the chat page's TopicSidebar
 * visual language (flat list; harness topics have no branch tree):
 * - Panel collapses to 0% width with the same left-edge divider, floating-pill
 *   resize handle (drag to resize, click pill to toggle), and fade transitions;
 * - New-topic button: the chat page's rounded-3xl full-width action button;
 * - Topic rows: the chat page's root-topic card style (rounded-3xl h-12 cards +
 *   active-glow) with the hover dot-menu opening the acrylic context menu
 *   (contextMenuIn/Out animations) for rename / delete; rename is inline
 *   (same input style as the chat page).
 */
import { Component, createSignal, For, Show, onCleanup, onMount } from 'solid-js';
import { Portal } from 'solid-js/web';
import Icon from '../../../shared/components/Icon';
import {
  currentHarnessTopicId,
  harnessTopics,
  selectHarnessTopic,
} from '../../../core/store/harness';
import { t } from '../../../core/i18n';

interface HarnessTopicSidebarProps {
  width: number;
  isCollapsed: boolean;
  onToggle: (e: MouseEvent) => void;
  onResize: (e: MouseEvent) => void;
  onAddTopic: () => void;
  onRenameTopic: (id: string, title: string) => void;
  onDeleteTopic: (id: string) => void;
  isResizing: boolean;
}

const HarnessTopicSidebar: Component<HarnessTopicSidebarProps> = (props) => {
  const [editingId, setEditingId] = createSignal<string | null>(null);
  const [menu, setMenu] = createSignal<{ open: boolean; x: number; y: number; id: string | null }>({
    open: false,
    x: 0,
    y: 0,
    id: null,
  });
  const [menuAnimatingOut, setMenuAnimatingOut] = createSignal(false);
  const [menuDivShown, setMenuDivShown] = createSignal(false);

  const closeMenu = () => {
    setMenu((p) => ({ ...p, open: false }));
    setMenuAnimatingOut(true);
    setTimeout(() => {
      setMenuDivShown(false);
      setMenuAnimatingOut(false);
    }, 200);
  };

  onMount(() => {
    const h = () => {
      if (menu().open) closeMenu();
    };
    window.addEventListener('click', h);
    onCleanup(() => window.removeEventListener('click', h));
  });

  const openMenu = (e: MouseEvent, topicId: string) => {
    e.stopPropagation();
    setMenuAnimatingOut(false);
    setMenuDivShown(true);
    const rect = (e.currentTarget as Element).getBoundingClientRect();
    setMenu({ open: true, x: rect.left - 100, y: rect.top + rect.height, id: topicId });
  };

  const startEdit = (id: string) => setEditingId(id);

  const commitEdit = (topic: { id: string; title: string }, value: string) => {
    const text = value.trim();
    if (text) props.onRenameTopic(topic.id, text);
    setEditingId(null);
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
      {/* Faint divider between the three columns (chat-page convention) */}
      <Show when={!props.isCollapsed}>
        <span
          aria-hidden="true"
          class="absolute top-0 bottom-0 w-px pointer-events-none"
          style={{ left: '-0.5px', background: 'rgba(var(--text-base-rgb),0.12)' }}
        />
      </Show>

      {/* Resize handle (left): 6px grab strip; the pill click toggles collapse —
          the handle lives outside the fading content so it works when collapsed. */}
      <div
        class="absolute top-0 bottom-0 left-[-3px] w-[6px] flex items-center justify-center cursor-ew-resize z-[1000] group"
        onMouseDown={(e) => props.onResize(e)}
      >
        <div
          class="absolute z-[1001] w-[10px] h-12 rounded-[20px] backdrop-blur-md cursor-pointer flex items-center justify-center text-xs font-bold transition-all duration-200 opacity-0 group-hover:opacity-100 hover:scale-110"
          style={{
            background: 'rgba(var(--text-base-rgb),0.08)',
            color: 'rgba(var(--text-base-rgb),0.6)',
            'box-shadow': '0 2px 8px rgba(0,0,0,0.3)',
          }}
          title={props.isCollapsed ? t('harness.topic.expand') : t('harness.topic.collapse')}
          onClick={(e) => {
            e.stopPropagation();
            props.onToggle(e);
          }}
        >
          <Icon name={props.isCollapsed ? 'chevron-left' : 'chevron-right'} size={10} />
        </div>
      </div>

      {/* Content: fades out (and blocks input) when collapsed */}
      <div
        class="h-full w-full overflow-hidden hover:overflow-y-auto transition-opacity duration-300"
        classList={{ 'opacity-0 pointer-events-none overflow-hidden': props.isCollapsed }}
      >
        {/* New topic: chat page's new-topic button style */}
        <button
          class="w-full px-3 h-12 inline-flex items-center justify-center rounded-3xl cursor-pointer transition-all duration-300"
          style={{
            background: 'rgba(var(--text-base-rgb),0.04)',
            border: '1px solid var(--border-dim)',
            color: 'rgba(var(--text-base-rgb),0.6)',
          }}
          onClick={() => props.onAddTopic()}
          onMouseEnter={(e) => (e.currentTarget.style.background = 'rgba(var(--primary-rgb),0.12)')}
          onMouseLeave={(e) =>
            (e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.04)')
          }
        >
          {t('harness.topic.newTopic')}
        </button>

        {/* Topic list: chat page's root-topic card rows */}
        <div class="mt-[15px] space-y-1">
          <For each={harnessTopics()}>
            {(topic) => {
              const isCurrent = () => currentHarnessTopicId() === topic.id;
              return (
                <div
                  class="group flex items-center justify-between cursor-pointer rounded-2xl transition-all duration-200 text-white/75 hover:bg-white/[0.06] px-3 h-12 rounded-3xl bg-white/[0.03] border border-white/[0.04]"
                  classList={{ 'active-glow': isCurrent() }}
                  onClick={() => selectHarnessTopic(topic.id)}
                >
                  <Show
                    when={editingId() === topic.id}
                    fallback={
                      <span
                        class="flex-1 min-w-0"
                        style={{
                          color: 'rgba(var(--text-base-rgb),0.75)',
                          'font-size': '0.9rem',
                          overflow: 'hidden',
                          'text-overflow': 'ellipsis',
                          'white-space': 'nowrap',
                          'padding-right': '8px',
                          'user-select': 'none',
                        }}
                      >
                        {topic.title}
                      </span>
                    }
                  >
                    <input
                      class="rounded px-2 py-0.5 text-[0.85rem] h-6 outline-none w-[80%]"
                      style={{
                        background: 'rgba(0,0,0,0.3)',
                        border: '1px solid var(--border-dim)',
                        color: 'rgba(var(--text-base-rgb),0.85)',
                      }}
                      value={topic.title}
                      ref={(el) => {
                        setTimeout(() => {
                          el.focus();
                          el.select();
                        }, 0);
                      }}
                      onBlur={(e) => commitEdit(topic, e.currentTarget.value)}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') commitEdit(topic, e.currentTarget.value);
                        else if (e.key === 'Escape') setEditingId(null);
                      }}
                      onClick={(e) => e.stopPropagation()}
                    />
                  </Show>
                  <button
                    class="flex items-center justify-center border-none rounded-full cursor-pointer transition-all duration-200 active:scale-90 opacity-0 group-hover:opacity-100 bg-white/[0.06] text-white/60 hover:bg-pri-10"
                    style={{ width: '30px', height: '30px' }}
                    onClick={(e) => openMenu(e as MouseEvent, topic.id)}
                  >
                    <Icon src="/icons/app-logo/dot-menu.svg" class="w-[18px] h-[18px]" />
                  </button>
                </div>
              );
            }}
          </For>
        </div>
      </div>

      {/* Context menu (chat page's acrylic menu with the same animations) */}
      {menuDivShown() && (
        <Portal>
          <div
            class="fixed z-[1500] min-w-[150px] rounded-lg shadow-[0_12px_40px_rgba(0,0,0,0.35)] py-1 origin-top-left"
            style={{
              top: `${menu().y}px`,
              left: `${menu().x}px`,
              background: 'var(--acrylic-bg)',
              'backdrop-filter': 'blur(24px) saturate(150%)',
              border: '1px solid var(--acrylic-border)',
              animation: menuAnimatingOut()
                ? 'contextMenuOut 0.14s ease-in forwards'
                : 'contextMenuIn 0.18s cubic-bezier(0.2, 0.8, 0.2, 1) forwards',
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <button
              class="w-full text-left px-3 py-2 bg-transparent border-none cursor-pointer rounded-lg transition-all duration-200 text-white/75 hover:bg-pri-10 hover:text-white"
              onClick={() => {
                const id = menu().id;
                if (id) startEdit(id);
                closeMenu();
              }}
            >
              {t('harness.topic.rename')}
            </button>
            <button
              class="w-full text-left px-3 py-2 bg-transparent border-none cursor-pointer rounded-lg transition-all duration-200"
              style={{ color: 'rgba(255,77,77,0.8)' }}
              onClick={() => {
                const target = harnessTopics().find((x) => x.id === menu().id);
                if (target && confirm(t('harness.topic.deleteConfirm', { name: target.title }))) {
                  props.onDeleteTopic(target.id);
                }
                closeMenu();
              }}
            >
              {t('harness.topic.delete')}
            </button>
          </div>
        </Portal>
      )}
    </div>
  );
};

export default HarnessTopicSidebar;
