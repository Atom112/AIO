import { createSignal, For, Show } from 'solid-js';
import { revertFileChange } from '../../core/api/project';
import { currentProject, openDiffPanelTab } from '../../core/store/store';
import { t } from '../../core/i18n';
import type { FileChangeInfo } from '../../core/store/store';
import Icon from './Icon';
import DiffLines, { ACTION_COLORS, actionLabel, parseSummary } from './DiffLines';

interface DiffViewProps {
  changes: FileChangeInfo[];
  /** True when the whole round of file changes was batch-reverted; all cards dim and per-item revert buttons are hidden */
  allReverted?: boolean;
  /**
   * Where a file's diff detail opens:
   * - `inline` (default): expand inside the bubble (harness approval preview);
   * - `side-panel`: keep the bubble compact and open the diff as a tab in the
   *   chat's right-hand panel (the bubbles themselves stay in the transcript).
   */
  detailMode?: 'inline' | 'side-panel';
}

const DiffView = (props: DiffViewProps) => {
  const sidePanel = () => props.detailMode === 'side-panel';

  return (
    <For each={props.changes}>
      {(change) => {
        const [expanded, setExpanded] = createSignal(false);

        const handleRevert = async () => {
          try {
            const projectPath = currentProject()?.path;
            if (!projectPath) return;
            await revertFileChange(projectPath, change.filePath);
          } catch (err) {
            console.error('Revert failed:', err);
          }
        };

        return (
          <div
            class="rounded-md overflow-hidden mb-1.5 border border-white/[0.06] bg-white/[0.01] transition-opacity duration-300"
            classList={{ 'opacity-40': !!props.allReverted }}
          >
            <button
              type="button"
              class="flex items-center gap-2 w-full px-2.5 py-1.5 cursor-pointer select-none bg-white/[0.03] border-none text-[13px] transition-colors hover:bg-white/[0.05]"
              title={sidePanel() ? t('chat.sidePanel.openDiff') : undefined}
              onClick={() =>
                sidePanel()
                  ? openDiffPanelTab(change, !!props.allReverted)
                  : setExpanded(!expanded())
              }
            >
              <span
                class="flex items-center justify-center w-4 h-4 rounded bg-white/[0.05] shrink-0"
                style={{ 'font-size': '11px' }}
              >
                {'<>'}
              </span>
              <span
                class="truncate font-mono text-white/70 flex-1 min-w-0 text-left"
                style={props.allReverted ? 'text-decoration: line-through;' : ''}
              >
                {change.filePath}
              </span>
              <span
                class="px-1.5 py-px rounded text-[12px] font-medium shrink-0"
                style={{
                  color: ACTION_COLORS[change.action],
                  background: 'rgba(var(--text-base-rgb),0.04)',
                  border: `1px solid ${ACTION_COLORS[change.action]}33`,
                }}
              >
                {actionLabel(change.action)}
              </span>
              <span class="font-mono text-[12px] shrink-0">
                <For each={parseSummary(change.summary)}>
                  {(seg) => (
                    <span
                      style={
                        seg.color
                          ? { color: seg.color }
                          : { color: 'rgba(var(--text-base-rgb),0.35)' }
                      }
                    >
                      {seg.text}
                    </span>
                  )}
                </For>
              </span>
              <Show when={!props.allReverted}>
                <span
                  class="flex items-center justify-center w-5 h-5 rounded hover:bg-white/[0.08] shrink-0 text-white/25 hover:text-red-400 transition-colors cursor-pointer"
                  title={t('diff.revert')}
                  onClick={(e) => {
                    e.stopPropagation();
                    handleRevert();
                  }}
                >
                  <svg
                    xmlns="http://www.w3.org/2000/svg"
                    fill="none"
                    viewBox="0 0 24 24"
                    stroke-width="2"
                    stroke="currentColor"
                    class="w-3.5 h-3.5"
                  >
                    <path
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      d="M9 15L3 9m0 0l6-6M3 9h12a6 6 0 010 12h-3"
                    />
                  </svg>
                </span>
              </Show>
              <span
                class="flex items-center justify-center shrink-0 text-white/20 transition-transform duration-150 w-4 h-4"
                style={{
                  transform: !sidePanel() && expanded() ? 'rotate(180deg)' : 'rotate(0deg)',
                }}
              >
                <Show when={!sidePanel()} fallback={<Icon name="chevron-right" size={12} />}>
                  <svg
                    xmlns="http://www.w3.org/2000/svg"
                    fill="none"
                    viewBox="0 0 24 24"
                    stroke-width="2"
                    stroke="currentColor"
                    class="w-3 h-3"
                  >
                    <path
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      d="M19.5 8.25l-7.5 7.5-7.5-7.5"
                    />
                  </svg>
                </Show>
              </span>
            </button>
            <Show when={!sidePanel()}>
              <div
                style={{
                  'max-height': expanded() ? '320px' : '0px',
                  opacity: expanded() ? '1' : '0',
                  transition: 'max-height 0.2s ease, opacity 0.15s ease',
                  overflow: 'hidden',
                }}
              >
                <div
                  class="overflow-x-auto overflow-y-auto"
                  style={{
                    'max-height': '320px',
                    'font-family':
                      "'Cascadia Code', 'Fira Code', 'JetBrains Mono', Consolas, monospace",
                    'font-size': '12px',
                    'line-height': '1.5',
                  }}
                >
                  <DiffLines diff={change.diff} />
                </div>
              </div>
            </Show>
          </div>
        );
      }}
    </For>
  );
};

export default DiffView;
