/**
 * @file DiffPanel.tsx
 * @description Content of one file-diff tab in the chat right-hand panel: the file
 * path, action chip, line summary and the full unified diff. The bubble in the
 * transcript keeps only the compact file row; detail lives here.
 */
import { For, Show } from 'solid-js';
import { t } from '../../../core/i18n';
import type { DiffPanelTab } from '../../../core/store/store';
import DiffLines, {
  ACTION_COLORS,
  actionLabel,
  parseSummary,
} from '../../../shared/components/DiffLines';

const DiffPanel = (props: { tab: DiffPanelTab }) => {
  return (
    <div class="flex flex-col h-full min-h-0">
      <div class="flex items-center gap-2 pb-3 shrink-0 min-w-0">
        <span class="truncate font-mono text-[13px] text-white/75 flex-1 min-w-0">
          {props.tab.filePath}
        </span>
        <span
          class="px-1.5 py-px rounded text-[12px] font-medium shrink-0"
          style={{
            color: ACTION_COLORS[props.tab.action],
            background: 'rgba(var(--text-base-rgb),0.04)',
            border: `1px solid ${ACTION_COLORS[props.tab.action]}33`,
          }}
        >
          {actionLabel(props.tab.action)}
        </span>
        <span class="font-mono text-[12px] shrink-0">
          <For each={parseSummary(props.tab.summary)}>
            {(seg) => (
              <span
                style={
                  seg.color ? { color: seg.color } : { color: 'rgba(var(--text-base-rgb),0.35)' }
                }
              >
                {seg.text}
              </span>
            )}
          </For>
        </span>
        <Show when={props.tab.allReverted}>
          <span
            class="px-1.5 py-px rounded text-[12px] font-medium shrink-0"
            style={{ color: 'var(--color-success)' }}
          >
            {t('agent.diff.reverted')}
          </span>
        </Show>
      </div>
      <div
        class="flex-1 min-h-0 overflow-auto rounded-md border border-white/[0.06] bg-white/[0.01]"
        classList={{ 'opacity-40': props.tab.allReverted }}
        style={{
          'font-family': "'Cascadia Code', 'Fira Code', 'JetBrains Mono', Consolas, monospace",
          'font-size': '12px',
          'line-height': '1.5',
        }}
      >
        <DiffLines diff={props.tab.diff} />
      </div>
    </div>
  );
};

export default DiffPanel;
