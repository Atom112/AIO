/**
 * @file SidePanelShell.tsx
 * @description Right-hand side panel chrome of the chat page: width, padding,
 * divider, drag handle, collapse toggle and the scrollable content area.
 *
 * Extracted from TopicSidebar so the topic list and the file-diff viewer share one
 * persistent shell — switching tabs only swaps the content while the width animates
 * (the shell keeps `transition: all 0.3s` unless a resize drag is in progress).
 */
import { JSX, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import { t } from '../../../core/i18n';

interface SidePanelShellProps {
  /** Panel width percentage (topic width or the wider diff width). */
  width: number;
  isCollapsed: boolean;
  isResizing: boolean;
  /** 'hover' (topic list, historical behavior) or 'always' (diff viewer) scrollbar. */
  scrollMode?: 'hover' | 'always';
  onResize: (e: MouseEvent) => void;
  onToggle: (e: MouseEvent) => void;
  /** Tab row rendered above the scrolling content. */
  header?: JSX.Element;
  children: JSX.Element;
}

const SidePanelShell = (props: SidePanelShellProps) => {
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
      {/* Faint divider between the columns: sits on the panel edge (+/-0.5px), centered exactly with the resize handle below; hidden when collapsed */}
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
        class="flex flex-col h-full w-full min-h-0 transition-opacity duration-300"
        classList={{ 'opacity-0 pointer-events-none overflow-hidden': props.isCollapsed }}
      >
        {props.header}
        <div
          class="flex-1 min-h-0 w-full overflow-x-hidden"
          classList={{
            'overflow-y-auto': props.scrollMode === 'always',
            'overflow-hidden hover:overflow-y-auto': props.scrollMode !== 'always',
          }}
        >
          {props.children}
        </div>
      </div>
    </div>
  );
};

export default SidePanelShell;
