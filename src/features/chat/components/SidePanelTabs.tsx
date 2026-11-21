/**
 * @file SidePanelTabs.tsx
 * @description Tab row of the chat right-hand panel: opened file diffs (positional
 * labels "文件1", "文件2", ...) plus the topic list. Renders nothing while no diff
 * tab is open so the panel keeps its original single-purpose look.
 */
import { For, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import { t } from '../../../core/i18n';
import {
  activeSidePanelTab,
  closeDiffPanelTab,
  diffPanelTabs,
  setActiveSidePanelTab,
} from '../../../core/store/store';
import { btnTabXs, tabActive, tabInactive } from '../../../shared/components/buttonStyles';

const SidePanelTabs = () => {
  return (
    <Show when={diffPanelTabs().length > 0}>
      <div class="flex items-center gap-1 mb-3 shrink-0 overflow-x-auto">
        <For each={diffPanelTabs()}>
          {(tab, index) => (
            <div
              class={`flex items-center rounded-md shrink-0 transition-colors duration-200 ${
                activeSidePanelTab() === tab.id ? tabActive : tabInactive
              }`}
            >
              <button
                type="button"
                class="pl-2.5 pr-1 py-1 text-xs bg-transparent border-none cursor-pointer whitespace-nowrap"
                title={tab.filePath}
                onClick={() => setActiveSidePanelTab(tab.id)}
              >
                {t('chat.sidePanel.file', { index: index() + 1 })}
              </button>
              <button
                type="button"
                class="pl-0.5 pr-1.5 py-1 bg-transparent border-none cursor-pointer opacity-50 hover:opacity-100"
                title={t('chat.sidePanel.closeFile')}
                onClick={(e) => {
                  e.stopPropagation();
                  closeDiffPanelTab(tab.id);
                }}
              >
                <Icon name="x" size={10} />
              </button>
            </div>
          )}
        </For>
        <button
          type="button"
          class={`${btnTabXs} shrink-0 ${activeSidePanelTab() === 'topics' ? tabActive : tabInactive}`}
          onClick={() => setActiveSidePanelTab('topics')}
        >
          {t('chat.sidePanel.topics')}
        </button>
      </div>
    </Show>
  );
};

export default SidePanelTabs;
