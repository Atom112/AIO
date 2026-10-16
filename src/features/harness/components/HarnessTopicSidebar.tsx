/**
 * Harness topic sidebar — the topics of the currently selected (tool, workspace).
 * Flat list (no branch tree): select on click, inline rename on edit, delete on
 * the trash action. Mirrors the chat TopicSidebar's visual language.
 */
import { Component, createSignal, For, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import {
  currentHarnessTopicId,
  harnessTopics,
  selectHarnessTopic,
} from '../../../core/store/harness';
import { formatDateTime, t } from '../../../core/i18n';

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
  const [editText, setEditText] = createSignal('');

  const startEdit = (id: string, current: string) => {
    setEditingId(id);
    setEditText(current);
  };

  const commitEdit = () => {
    const id = editingId();
    const text = editText().trim();
    if (id && text) props.onRenameTopic(id, text);
    setEditingId(null);
  };

  return (
    <Show
      when={!props.isCollapsed}
      fallback={
        // Collapsed: floating expand button on the right edge
        <div class="relative w-0 h-full">
          <button
            class="absolute top-3 right-0 z-[5] text-white/30 hover:text-white/70 transition-colors"
            onClick={(e) => props.onToggle(e)}
            title={t('harness.topic.expand')}
          >
            <Icon name="chevron-left" class="w-4 h-4" />
          </button>
        </div>
      }
    >
      <div
        class="relative flex flex-col flex-shrink-0 min-w-0 h-full"
        style={{
          width: `${props.width}%`,
          padding: '6px 0px 6px 3px',
          transition: props.isResizing ? 'none' : 'all 0.3s cubic-bezier(0.4, 0, 0.2, 1)',
        }}
      >
        {/* Three-pane divider line */}
        <div
          class="absolute top-0 left-0 h-full w-px z-[1]"
          style={{ background: 'var(--border-dim)' }}
        />
        {/* Resize handle */}
        <div
          class="absolute top-0 left-[-3px] h-full w-[6px] cursor-ew-resize z-[3] hover:bg-white/[0.04]"
          onMouseDown={(e) => props.onResize(e)}
          style={{ 'touch-action': 'none' }}
        />

        <div class="relative flex flex-col h-full pl-[18px] pr-[15px] pt-2">
          {/* Header */}
          <div class="flex items-center justify-between mb-2 pr-1">
            <button
              class="text-white/30 hover:text-white/70 transition-colors"
              onClick={(e) => props.onToggle(e)}
              title={t('harness.topic.collapse')}
            >
              <Icon name="chevron-right" class="w-4 h-4" />
            </button>
            <button
              class="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs font-medium bg-white/[0.06] border border-white/10 text-white/60 hover:bg-white/[0.1] hover:text-white/85 active:scale-95 transition-all duration-200 cursor-pointer"
              onClick={() => props.onAddTopic()}
            >
              <Icon name="plus" size={12} />
              {t('harness.topic.newTopic')}
            </button>
          </div>

          {/* Topic list */}
          <div class="flex-1 min-h-0 overflow-y-auto">
            <For each={harnessTopics()}>
              {(topic) => (
                <Show
                  when={editingId() !== topic.id}
                  fallback={
                    <div class="group flex items-center gap-1.5 px-2 py-1.5 rounded-md mb-0.5 bg-pri-20/40">
                      <input
                        class="flex-1 min-w-0 bg-transparent border-none outline-none text-xs text-white"
                        value={editText()}
                        onInput={(e) => setEditText(e.currentTarget.value)}
                        onBlur={commitEdit}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') commitEdit();
                          if (e.key === 'Escape') setEditingId(null);
                        }}
                        ref={(el) => setTimeout(() => el.focus())}
                      />
                    </div>
                  }
                >
                  <div
                    class={`group flex items-center gap-2 px-2.5 py-2 rounded-md mb-0.5 cursor-pointer transition-colors ${
                      currentHarnessTopicId() === topic.id
                        ? 'bg-pri-20 text-white'
                        : 'text-white/55 hover:text-white/85 hover:bg-white/[0.05]'
                    }`}
                    onClick={() => selectHarnessTopic(topic.id)}
                    title={`${topic.title}\n${formatDateTime(new Date(topic.updatedAt).getTime() || Date.now())}`}
                  >
                    <Icon name="terminal" size={12} class="shrink-0" />
                    <span class="text-xs truncate flex-1">{topic.title}</span>
                    <span class="shrink-0 hidden group-hover:flex items-center gap-1">
                      <button
                        class="text-white/30 hover:text-white/80 transition-colors"
                        title={t('harness.topic.rename')}
                        onClick={(e) => {
                          e.stopPropagation();
                          startEdit(topic.id, topic.title);
                        }}
                      >
                        <Icon name="pen-line" class="w-3 h-3" />
                      </button>
                      <button
                        class="text-white/30 hover:text-danger transition-colors"
                        title={t('harness.topic.delete')}
                        onClick={(e) => {
                          e.stopPropagation();
                          if (confirm(t('harness.topic.deleteConfirm', { name: topic.title }))) {
                            props.onDeleteTopic(topic.id);
                          }
                        }}
                      >
                        <Icon name="trash" class="w-3 h-3" />
                      </button>
                    </span>
                  </div>
                </Show>
              )}
            </For>
          </div>
        </div>
      </div>
    </Show>
  );
};

export default HarnessTopicSidebar;
