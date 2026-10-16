/**
 * Harness session-config controls — renders the config options the agent (or
 * the native driver) reported: model selector, thought-level selector, and
 * other selects, as compact dropdowns in the input bar's bottom-left slot
 * (the chat page's AgentModeSelector position). Boolean options are not
 * rendered in v1 (no known agent exposes them; selects cover model/effort).
 *
 * Data sources (first non-empty wins) so the controls render eagerly:
 * 1. live: the current topic's reported option set (a driver has spawned);
 * 2. snapshot: the tool's last reported set persisted in localStorage;
 * 3. synthesized: the backend's static presets for native adapters
 *    (`harness_tool_config_options`) — first-use dropdown data before any
 *    turn has run. ACP tools never used since install have no data until
 *    their first session/new — the protocol owns their option lists.
 *
 * Dispatch semantics per adapter:
 * - ACP (incl. acp-fallback): every change → `session/set_config_option`
 *   (live when a session runs; otherwise the persisted snapshot refreshes on
 *   the next spawn and the agent applies from the next turn).
 * - claude-native: "model" → live `set_model` control frame; "effort" →
 *   spawn-time `--effort` flag, so changing it persists the preference and
 *   stops the session runtime (the next prompt respawns with the new flag).
 * - codex-native: model/effort stored on the driver for the next per-turn
 *   spawn (each turn is a fresh `codex exec` process).
 */
import { Component, createEffect, createSignal, For, Show } from 'solid-js';
import Dropdown from '../../../shared/components/Dropdown';
import {
  currentHarnessConfigOptions,
  currentHarnessToolId,
  currentHarnessTopicId,
  harnessConfigCache,
  harnessNativeModel,
  harnessNativeThinking,
  harnessTools,
  saveHarnessNativePref,
} from '../../../core/store/harness';
import {
  harnessSetConfigOption,
  harnessStopSession,
  harnessToolConfigOptions,
} from '../../../core/api/harness';
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

  /** Backend-synthesized presets for native adapters (first-use fallback). */
  const [synth, setSynth] = createSignal<Record<string, HarnessConfigOption[]>>({});
  createEffect(() => {
    const toolId = currentHarnessToolId();
    const ad = tool()?.adapter;
    if (!toolId || (ad !== 'claude-native' && ad !== 'codex-native')) return;
    if (synth()[toolId]) return;
    void harnessToolConfigOptions(toolId)
      .then((options) => {
        if (options.length > 0) setSynth((prev) => ({ ...prev, [toolId]: options }));
      })
      .catch(() => undefined);
  });

  /** Option set to render: live report > persisted snapshot > synthesized. */
  const options = () => {
    const live = currentHarnessConfigOptions();
    if (live.length > 0) return live;
    const toolId = currentHarnessToolId();
    if (!toolId) return [];
    const cached = harnessConfigCache()[toolId];
    if (cached && cached.length > 0) return cached;
    return synth()[toolId] ?? [];
  };

  /** Change handler per adapter (see the module doc). */
  const applyChange = (option: HarnessConfigOption, value: string) => {
    const sid = currentHarnessTopicId();
    const toolId = currentHarnessToolId();
    if (!toolId) return;
    if (isClaudeNative()) {
      if (option.optionId === 'effort') {
        // Spawn-time flag: persist + stop the runtime so the next prompt
        // respawns with the new --effort (kills a running turn — confirm).
        if (props.isBusy && !confirm(t('harness.config.effortRestartConfirm'))) return;
        saveHarnessNativePref(toolId, 'thinking', value);
        if (sid) void harnessStopSession(sid).catch(console.error);
        return;
      }
      saveHarnessNativePref(toolId, 'model', value);
    } else if (isCodexNative()) {
      // Stored on the driver for the next per-turn spawn; persisted for respawn.
      saveHarnessNativePref(toolId, option.optionId === 'effort' ? 'thinking' : 'model', value);
    }
    if (sid) void harnessSetConfigOption(sid, option.optionId, value).catch(console.error);
  };

  return (
    <For each={options()}>
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
            const dropdownOptions = () => [
              ...option.values.map((v) => ({ value: v.id, label: v.name || v.id })),
              ...extra().map((v) => ({ value: v.id, label: v.name || v.id })),
            ];
            return (
              <Dropdown
                variant="ghost"
                class="w-[150px]"
                menuUp
                value={current()}
                onChange={(v) => applyChange(option, v)}
                options={dropdownOptions()}
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
