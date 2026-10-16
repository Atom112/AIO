/**
 * Harness session-config controls — renders the config options the agent (or
 * the native driver) reported: model selector, thought-level selector, and
 * other selects, as compact dropdowns in the input bar's bottom-left slot
 * (the chat page's AgentModeSelector position). Boolean options are not
 * rendered in v1 (no known agent exposes them; selects cover model/effort).
 *
 * Dispatch semantics per adapter:
 * - ACP (incl. acp-fallback): every change → `session/set_config_option`
 *   (live; the agent re-emits its config_option_update).
 * - claude-native: "model" → live `set_model` control frame; "effort" →
 *   spawn-time `--effort` flag, so changing it persists the preference and
 *   stops the session runtime (the next prompt respawns with the new flag).
 * - codex-native: model/effort stored on the driver for the next per-turn
 *   spawn (each turn is a fresh `codex exec` process).
 */
import { Component, For, Show } from 'solid-js';
import Dropdown from '../../../shared/components/Dropdown';
import {
  currentHarnessConfigOptions,
  currentHarnessToolId,
  currentHarnessTopicId,
  harnessNativeModel,
  harnessNativeThinking,
  harnessTools,
  saveHarnessNativePref,
} from '../../../core/store/harness';
import { harnessSetConfigOption, harnessStopSession } from '../../../core/api/harness';
import { t } from '../../../core/i18n';
import type { HarnessConfigOption } from '../../../core/store/harness';

interface HarnessConfigControlsProps {
  isBusy: boolean;
}

/** Placeholder for an unset selector: the agent's option name or a category default. */
const placeholderFor = (option: HarnessConfigOption) =>
  option.name ??
  (option.category === 'thought_level' ? t('harness.config.thinking') : t('harness.config.model'));

const HarnessConfigControls: Component<HarnessConfigControlsProps> = (props) => {
  const tool = () => harnessTools().find((x) => x.id === currentHarnessToolId());
  const adapter = () => tool()?.adapter ?? 'acp';
  const isClaudeNative = () => adapter() === 'claude-native';
  const isCodexNative = () => adapter() === 'codex-native';

  /** Change handler per adapter (see the module doc). */
  const applyChange = (option: HarnessConfigOption, value: string) => {
    const sid = currentHarnessTopicId();
    const toolId = currentHarnessToolId();
    if (!sid || !toolId) return;
    if (isClaudeNative()) {
      if (option.optionId === 'effort') {
        // Spawn-time flag: persist + stop the runtime so the next prompt
        // respawns with the new --effort (kills a running turn — confirm).
        if (props.isBusy && !confirm(t('harness.config.effortRestartConfirm'))) return;
        saveHarnessNativePref(toolId, 'thinking', value);
        void harnessStopSession(sid).catch(console.error);
        return;
      }
      saveHarnessNativePref(toolId, 'model', value);
    } else if (isCodexNative()) {
      // Stored on the driver for the next per-turn spawn; persisted for respawn.
      saveHarnessNativePref(toolId, option.optionId === 'effort' ? 'thinking' : 'model', value);
    }
    void harnessSetConfigOption(sid, option.optionId, value).catch(console.error);
  };

  return (
    <For each={currentHarnessConfigOptions()}>
      {(option) => (
        <Show when={option.kind === 'select' && option.values.length > 0}>
          {(() => {
            // Display value: the agent-reported current; for native adapters
            // fall back to the persisted preference when unset. A current not
            // present in the value list (e.g. claude's resolved full model id)
            // is appended as an extra option so the trigger shows it.
            const toolId = currentHarnessToolId();
            const pref =
              option.optionId === 'effort'
                ? toolId
                  ? harnessNativeThinking(toolId)
                  : undefined
                : toolId
                  ? harnessNativeModel(toolId)
                  : undefined;
            const current = () =>
              option.current && option.values.some((v) => v.id === option.current)
                ? option.current
                : option.current || pref || '';
            const extra = () =>
              option.current && !option.values.some((v) => v.id === option.current)
                ? [{ id: option.current, name: option.current }]
                : [];
            const options = () => [
              ...option.values.map((v) => ({ value: v.id, label: v.name || v.id })),
              ...extra().map((v) => ({ value: v.id, label: v.name || v.id })),
            ];
            return (
              <Dropdown
                variant="ghost"
                class="w-[150px]"
                value={current()}
                onChange={(v) => applyChange(option, v)}
                options={options()}
                placeholder={placeholderFor(option)}
              />
            );
          })()}
        </Show>
      )}
    </For>
  );
};

export default HarnessConfigControls;
