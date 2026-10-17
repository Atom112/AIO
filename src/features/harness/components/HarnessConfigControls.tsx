/**
 * Harness session-config controls — the input bar's bottom-left cluster (the
 * chat page's AgentModeSelector slot), rendered with the chat page's own
 * control visuals:
 * - model options → HarnessModelSelect (icon-only trigger + provider-logo
 *   popup rows, the chat ModelSelector replica);
 * - thought_level options → HarnessThinkingSlider (the chat ReasoningButton
 *   replica, generalized to the tool's own stop set);
 * - mode / spawn / other selects → HarnessModeSelect (the chat
 *   AgentModeSelector replica: icon + label trigger, upward acrylic popup);
 * - the Claude-native permission mode rides the same HarnessModeSelect.
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
 * - category "spawn" (registry-declared CLI-flag/env options): spawn-time
 *   only — the change persists the preference and stops the session runtime,
 *   and the next prompt respawns with the new flag (never forwarded).
 */
import { Component, createEffect, createSignal, For, Show } from 'solid-js';
import type { IconName } from '../../../shared/components/Icon';
import {
  currentHarnessConfigOptions,
  currentHarnessToolId,
  currentHarnessTopicId,
  harnessClaudeMode,
  harnessConfigCache,
  harnessNativeModel,
  harnessNativeThinking,
  harnessSpawnPrefs,
  harnessTools,
  saveHarnessClaudeMode,
  saveHarnessNativePref,
  saveHarnessSpawnPref,
} from '../../../core/store/harness';
import {
  harnessSetConfigOption,
  harnessSetPermissionMode,
  harnessStopSession,
  harnessToolConfigOptions,
} from '../../../core/api/harness';
import { t } from '../../../core/i18n';
import type { HarnessConfigOption } from '../../../core/store/harness';
import HarnessModelSelect from './HarnessModelSelect';
import HarnessModeSelect from './HarnessModeSelect';
import HarnessThinkingSlider from './HarnessThinkingSlider';

interface HarnessConfigControlsProps {
  isBusy: boolean;
}

/** Placeholder/popup title: known spawn-option ids i18n first, then the
 * agent/registry name, then a category default. */
const titleFor = (option: HarnessConfigOption) => {
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

/** i18n label for a thinking-level value id (null → name/id fallback). */
const thinkValueLabel = (id: string): string | null => {
  switch (id) {
    case 'off':
      return t('harness.thinkValue.off');
    case 'minimal':
      return t('harness.thinkValue.minimal');
    case 'low':
      return t('harness.thinkValue.low');
    case 'medium':
      return t('harness.thinkValue.medium');
    case 'high':
      return t('harness.thinkValue.high');
    case 'xhigh':
      return t('harness.thinkValue.xhigh');
    case 'max':
      return t('harness.thinkValue.max');
    case 'auto':
      return t('harness.thinkValue.auto');
    default:
      return null;
  }
};

/** Display label for one option value: i18n for known spawn/thinking values,
 * provider-prefixed for `provider/model` ids (omp's catalog), else name/id. */
const valueLabel = (option: HarnessConfigOption, id: string, name?: string) => {
  if (option.category === 'spawn') {
    const localized = spawnValueLabel(id);
    if (localized) return localized;
  }
  if (option.category === 'thought_level') {
    const localized = thinkValueLabel(id);
    if (localized) return localized;
  }
  const slash = id.indexOf('/');
  if (slash > 0) return `${id.slice(0, slash)} · ${name || id.slice(slash + 1)}`;
  return name || id;
};

/** Trigger icon per mode-kind option id. */
const modeIconFor = (option: HarnessConfigOption): IconName => {
  if (option.optionId === 'approval-mode' || option.optionId === 'goose-mode') return 'eye';
  if (option.optionId === 'sandbox') return 'layers';
  return 'compass';
};

/** Claude permission-mode trigger icons per mode value. */
const CLAUDE_MODE_ICONS: Record<string, IconName> = {
  default: 'eye',
  acceptEdits: 'pen-line',
  plan: 'document',
  dontAsk: 'check-circle',
  bypassPermissions: 'zap',
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

  /** Display order: model first (chat's left slot), mode-kind middle,
   * thinking last (chat's reasoning slot). Stable within a group. */
  const sortWeight = (o: HarnessConfigOption) =>
    o.category === 'model' ? 0 : o.category === 'thought_level' ? 2 : 1;
  const sortedOptions = () =>
    options()
      .map((o, i) => ({ o, i }))
      .sort((a, b) => sortWeight(a.o) - sortWeight(b.o) || a.i - b.i)
      .map((x) => x.o);

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
      // CLI-flag/env-backed: persist + respawn (a running turn dies — confirm).
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

  /** Option values + the out-of-list current appended as an extra entry. */
  const valuesOf = (option: HarnessConfigOption) => {
    const extra =
      option.current && !option.values.some((v) => v.id === option.current)
        ? [{ id: option.current, name: undefined as string | undefined }]
        : [];
    return [...option.values, ...extra];
  };

  return (
    <>
      <For each={sortedOptions()}>
        {(option) => (
          <Show when={option.kind === 'select' && option.values.length > 0}>
            <Show
              when={option.category === 'model'}
              fallback={
                <Show
                  when={option.category === 'thought_level'}
                  fallback={
                    <HarnessModeSelect
                      icon={modeIconFor(option)}
                      title={titleFor(option)}
                      options={valuesOf(option).map((v) => ({
                        value: v.id,
                        label: valueLabel(option, v.id, v.name),
                      }))}
                      value={currentOf(option)}
                      onChange={(v) => applyChange(option, v)}
                    />
                  }
                >
                  <HarnessThinkingSlider
                    title={titleFor(option)}
                    stops={valuesOf(option).map((v) => ({
                      value: v.id,
                      label: valueLabel(option, v.id, v.name),
                    }))}
                    value={currentOf(option)}
                    onChange={(v) => applyChange(option, v)}
                  />
                </Show>
              }
            >
              <HarnessModelSelect
                title={titleFor(option)}
                options={valuesOf(option).map((v) => ({
                  value: v.id,
                  label: valueLabel(option, v.id, v.name),
                  // Logo resolution key: the model's own name (chat parity),
                  // post-slash for `provider/model` ids.
                  modelName:
                    v.name ?? (v.id.indexOf('/') > 0 ? v.id.slice(v.id.indexOf('/') + 1) : v.id),
                }))}
                value={currentOf(option)}
                onChange={(v) => applyChange(option, v)}
              />
            </Show>
          </Show>
        )}
      </For>

      {/* Claude native permission mode rides the same mode-select visuals. */}
      <Show when={isClaudeNative()}>
        <HarnessModeSelect
          icon={CLAUDE_MODE_ICONS[harnessClaudeMode()] ?? 'eye'}
          title={t('harness.config.permissionMode')}
          options={[
            { value: 'default', label: t('harness.mode.default') },
            { value: 'acceptEdits', label: t('harness.mode.acceptEdits') },
            { value: 'plan', label: t('harness.mode.plan') },
            { value: 'dontAsk', label: t('harness.mode.dontAsk') },
            { value: 'bypassPermissions', label: t('harness.mode.bypassPermissions') },
          ]}
          value={harnessClaudeMode()}
          onChange={(mode) => {
            saveHarnessClaudeMode(mode);
            const sid = currentHarnessTopicId();
            if (sid) void harnessSetPermissionMode(sid, mode).catch(console.error);
          }}
        />
      </Show>
    </>
  );
};

export default HarnessConfigControls;
