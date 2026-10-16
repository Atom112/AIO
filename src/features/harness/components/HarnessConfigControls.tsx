/**
 * Harness session-config controls — renders the config options the agent (or
 * the native driver, or the registry's spawn-option declarations) reported:
 * model selector, thought-level selector, mode/approval selectors, as compact
 * dropdowns in the input bar's bottom-left slot (the chat page's
 * AgentModeSelector position). Boolean options are not rendered in v1.
 *
 * Data sources (first non-empty wins) so the controls render eagerly:
 * 1. live: the current topic's reported option set (a driver has spawned);
 * 2. snapshot: the tool's last reported set persisted in localStorage;
 * 3. synthesized: the backend's static set (`harness_tool_config_options`) —
 *    native presets plus registry-declared spawn options (e.g. omp's
 *    --approval-mode), so they render even before the first spawn.
 *
 * Option semantics by category:
 * - agent options (mode/model/thinking_level/...): live change via
 *   `session/set_config_option` (ACP) or the native driver's handler;
 * - category "spawn" (registry-declared CLI-flag options): spawn-time only —
 *   the change persists the preference and stops the session runtime, and the
 *   next prompt respawns with the new flag (never forwarded to the agent).
 *
 * Model options whose value id carries a `provider/model` shape (omp's 588-
 * model catalog) render as `provider · name` so the source is visible.
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
  harnessSpawnPrefs,
  harnessTools,
  saveHarnessNativePref,
  saveHarnessSpawnPref,
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

/** Placeholder for an unset selector: known spawn-option ids i18n first, then
 * the agent/registry name, then a category default. */
const placeholderFor = (option: HarnessConfigOption) => {
  if (option.optionId === 'approval-mode') return t('harness.config.approvalMode');
  if (option.optionId === 'goose-mode') return t('harness.config.permissionMode');
  if (option.optionId === 'sandbox') return t('harness.config.sandbox');
  return (
    option.name ??
    (option.category === 'thought_level' ? t('harness.config.thinking') : t('harness.config.model'))
  );
};

/** i18n label for a known spawn-option value id (null → fall back to the
 * registry/agent-provided name). Value ids are the tools' own vocabulary
 * (omp/gemini/qwen flags, goose env modes, codex sandbox policies). */
const spawnValueLabel = (id: string): string | null => {
  switch (id) {
    case 'always-ask':
      return t('harness.spawnValue.ask');
    case 'default':
      return t('harness.spawnValue.default');
    case 'write':
      return t('harness.spawnValue.write');
    case 'auto_edit':
      return t('harness.spawnValue.autoEdit');
    case 'yolo':
      return t('harness.spawnValue.yolo');
    case 'plan':
      return t('harness.spawnValue.plan');
    case 'auto':
      return t('harness.spawnValue.auto');
    case 'approve':
      return t('harness.spawnValue.approve');
    case 'smart_approve':
      return t('harness.spawnValue.smartApprove');
    case 'chat':
      return t('harness.spawnValue.chat');
    case 'read-only':
      return t('harness.spawnValue.readOnly');
    case 'workspace-write':
      return t('harness.spawnValue.workspaceWrite');
    case 'danger-full-access':
      return t('harness.spawnValue.fullAccess');
    default:
      return null;
  }
};

/** Display label for one option value: i18n for known spawn-option values,
 * provider-prefixed for `provider/model` ids (omp's catalog), else name/id. */
const valueLabel = (option: HarnessConfigOption, id: string, name?: string) => {
  if (option.category === 'spawn') {
    const localized = spawnValueLabel(id);
    if (localized) return localized;
  }
  const slash = id.indexOf('/');
  if (slash > 0) return `${id.slice(0, slash)} · ${name || id.slice(slash + 1)}`;
  return name || id;
};

const HarnessConfigControls: Component<HarnessConfigControlsProps> = (props) => {
  const tool = () => harnessTools().find((x) => x.id === currentHarnessToolId());
  const adapter = () => tool()?.adapter ?? 'acp';
  const isClaudeNative = () => adapter() === 'claude-native';
  const isCodexNative = () => adapter() === 'codex-native';

  /** Backend-synthesized presets (native adapters) and registry-declared
   * spawn options (all adapters) — the pre-spawn dropdown data. Fetched once
   * per tool. */
  const [synth, setSynth] = createSignal<Record<string, HarnessConfigOption[]>>({});
  createEffect(() => {
    const toolId = currentHarnessToolId();
    if (!toolId || synth()[toolId]) return;
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
    const synthesized = synth()[toolId] ?? [];
    if (cached && cached.length > 0) {
      // A snapshot from an older version may predate spawn options — merge in
      // any synthesized spawn options the snapshot lacks.
      const missing = synthesized.filter(
        (s) => s.category === 'spawn' && !cached.some((c) => c.optionId === s.optionId),
      );
      return missing.length > 0 ? [...cached, ...missing] : cached;
    }
    return synthesized;
  };

  /** Display value for one option: agent-reported current, else the local
   * preference (spawn prefs for spawn options, native prefs for model/effort). */
  const currentOf = (option: HarnessConfigOption) => {
    const toolId = currentHarnessToolId();
    const pref =
      option.category === 'spawn'
        ? toolId
          ? harnessSpawnPrefs()[toolId]?.[option.optionId]
          : undefined
        : option.optionId === 'effort'
          ? toolId
            ? harnessNativeThinking(toolId)
            : undefined
          : toolId
            ? harnessNativeModel(toolId)
            : undefined;
    return option.current && option.values.some((v) => v.id === option.current)
      ? option.current
      : option.current || pref || '';
  };

  /** Change handler per category/adapter (see the module doc). */
  const applyChange = (option: HarnessConfigOption, value: string) => {
    const sid = currentHarnessTopicId();
    const toolId = currentHarnessToolId();
    if (!toolId) return;
    if (option.category === 'spawn') {
      // CLI-flag-backed: persist + respawn (a running turn dies — confirm).
      if (props.isBusy && !confirm(t('harness.config.spawnRestartConfirm'))) return;
      saveHarnessSpawnPref(toolId, option.optionId, value);
      if (sid) void harnessStopSession(sid).catch(console.error);
      return;
    }
    if (isClaudeNative()) {
      if (option.optionId === 'effort') {
        if (props.isBusy && !confirm(t('harness.config.effortRestartConfirm'))) return;
        saveHarnessNativePref(toolId, 'thinking', value);
        if (sid) void harnessStopSession(sid).catch(console.error);
        return;
      }
      saveHarnessNativePref(toolId, 'model', value);
    } else if (isCodexNative()) {
      saveHarnessNativePref(toolId, option.optionId === 'effort' ? 'thinking' : 'model', value);
    }
    if (sid) void harnessSetConfigOption(sid, option.optionId, value).catch(console.error);
  };

  return (
    <For each={options()}>
      {(option) => (
        <Show when={option.kind === 'select' && option.values.length > 0}>
          {(() => {
            const current = () => currentOf(option);
            const extra = () =>
              option.current && !option.values.some((v) => v.id === option.current)
                ? [{ id: option.current as string, name: undefined as string | undefined }]
                : [];
            const dropdownOptions = () => [
              ...option.values.map((v) => ({
                value: v.id,
                label: valueLabel(option, v.id, v.name),
              })),
              ...extra().map((v) => ({ value: v.id, label: valueLabel(option, v.id, v.name) })),
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
