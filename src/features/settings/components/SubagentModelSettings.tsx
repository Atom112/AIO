/**
 * SubagentModelSettings — Per-profile model override configuration.
 *
 * Each subagent profile (9 built-in + custom) can optionally use a
 * different model/provider than the parent agent.  Only cloud models are
 * selectable because subagents run in the same process; a local model can
 * be used by leaving the override unset (falls back to parent model).
 */
import { Component, createMemo, createSignal, For, Show, onMount, onCleanup } from 'solid-js';
import {
  profileModelOverrides,
  setProfileModelOverrides,
  saveProfileModelOverrides,
  allAvailableModels,
  isLocalModel,
  modelKey,
  ActivatedModel,
  modelsCatalog,
  customSubagentProfiles,
  saveCustomSubagentProfile,
  deleteCustomSubagentProfile,
  mcpToolsCache,
  type CustomSubagentProfile,
} from '../../../core/store/store';
import { getLogo as getLogoByIds } from '../../../core/utils/modelLogo';
import { findModel, formatContextWindow } from '../../../core/utils/models';
import Icon from '../../../shared/components/Icon';
import { t } from '../../../core/i18n';

// --- Profile descriptions (mirrors subagent::builtin_profiles) ---

interface ProfileInfo {
  id: string;
  name: string;
  description: string;
}

const BUILTIN_PROFILE_IDS = new Set([
  'explorer',
  'coder',
  'general',
  'architect',
  'debugger',
  'reviewer',
  'writer',
  'tester',
  'requirements',
]);

const BUILTIN_PROFILES: ProfileInfo[] = [
  {
    id: 'explorer',
    name: t('subagent.profile.fileBrowser'),
    description: t('subagent.profileDesc.explorer'),
  },
  {
    id: 'coder',
    name: t('subagent.profile.codeEditor'),
    description: t('subagent.profileDesc.coder'),
  },
  {
    id: 'general',
    name: t('subagent.profile.generalAssistant'),
    description: t('subagent.profileDesc.general'),
  },
  {
    id: 'architect',
    name: t('subagent.profile.architect'),
    description: t('subagent.profileDesc.architect'),
  },
  {
    id: 'debugger',
    name: t('subagent.profile.debugger'),
    description: t('subagent.profileDesc.debugger'),
  },
  {
    id: 'reviewer',
    name: t('subagent.profile.codeReviewer'),
    description: t('subagent.profileDesc.reviewer'),
  },
  {
    id: 'writer',
    name: t('subagent.profile.docWriter'),
    description: t('subagent.profileDesc.writer'),
  },
  {
    id: 'tester',
    name: t('subagent.profile.testEngineer'),
    description: t('subagent.profileDesc.tester'),
  },
  {
    id: 'requirements',
    name: t('subagent.profile.requirementsAnalyst'),
    description: t('subagent.profileDesc.requirements'),
  },
];

/** Auto-generate a profile id from a display name */
const slugify = (name: string): string =>
  name
    .toLowerCase()
    .replace(/[^a-z0-9\u4e00-\u9fff-]/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-|-$/g, '') || 'custom';

const SubagentModelSettings: Component = () => {
  // --- Model override state ---
  const [openDropdown, setOpenDropdown] = createSignal<string | null>(null);

  /** Cloud models only (exclude local models). */
  const cloudModels = createMemo(() => allAvailableModels().filter((m) => !isLocalModel(m)));

  /** Create custom profile form state */
  const [showCreateForm, setShowCreateForm] = createSignal(false);
  const [newId, setNewId] = createSignal('');
  const [newName, setNewName] = createSignal('');
  const [newDescription, setNewDescription] = createSignal('');
  const [newAllowedTools, setNewAllowedTools] = createSignal('');
  const [newDeniedTools, setNewDeniedTools] = createSignal('');
  const [newSystemPrompt, setNewSystemPrompt] = createSignal('');

  /** Derived sets for tool multi-select UI */
  const allowedToolsSet = createMemo(
    () =>
      new Set(
        newAllowedTools()
          .split(',')
          .map((s) => s.trim())
          .filter(Boolean),
      ),
  );
  const deniedToolsSet = createMemo(
    () =>
      new Set(
        newDeniedTools()
          .split(',')
          .map((s) => s.trim())
          .filter(Boolean),
      ),
  );
  const toggleTool = (toolName: string, isAllowed: boolean) => {
    const [getter, setter] = isAllowed
      ? ([newAllowedTools, setNewAllowedTools] as const)
      : ([newDeniedTools, setNewDeniedTools] as const);
    const current = new Set(
      getter()
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean),
    );
    if (current.has(toolName)) current.delete(toolName);
    else current.add(toolName);
    setter(Array.from(current).join(','));
  };
  const [allowedOpen, setAllowedOpen] = createSignal(false);
  const [deniedOpen, setDeniedOpen] = createSignal(false);
  const availableTools = createMemo(() =>
    mcpToolsCache()
      .map((t) => t.function.name)
      .sort(),
  );

  /** Click outside closes tool dropdowns */
  onMount(() => {
    const handler = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      if (!target.closest('[data-tool-dropdown]')) {
        setAllowedOpen(false);
        setDeniedOpen(false);
      }
    };
    document.addEventListener('click', handler);
    onCleanup(() => document.removeEventListener('click', handler));
  });

  /** Combined profiles: built-in first, then custom */
  const allProfiles = createMemo<ProfileInfo[]>(() => [
    ...BUILTIN_PROFILES,
    ...customSubagentProfiles().map((c) => ({
      id: c.id,
      name: c.name,
      description: c.description,
    })),
  ]);

  const isBuiltin = (id: string) => BUILTIN_PROFILE_IDS.has(id);
  // NOTE: 此函数与 ProjectSettingsModal.tsx 中的 getProviderIdFor 存在分支副本；
  // 两者行为略有差异（此版本含 .trim() 和 deepseek.ai 检查），
  // 合并前需确认行为差异是否有意。另见 core/utils/models.ts 中的 detectProviderByUrl。
  const getProviderIdFor = (model: ActivatedModel): string => {
    if ((model as any).provider_id) return (model as any).provider_id;

    const rawUrl = (model.api_url || '').trim();
    let host = '';
    try {
      host = new URL(rawUrl).hostname.toLowerCase();
    } catch {
      host = '';
    }

    const hostMatches = (allowedHost: string) =>
      host === allowedHost || host.endsWith(`.${allowedHost}`);

    if (hostMatches('api.openai.com')) return 'openai';
    if (hostMatches('api.anthropic.com')) return 'anthropic';
    if (hostMatches('generativelanguage.googleapis.com')) return 'google';
    if (hostMatches('api.deepseek.com') || hostMatches('api.deepseek.ai')) return 'deepseek';
    if (hostMatches('api.groq.com')) return 'groq';
    if (hostMatches('api.mistral.ai')) return 'mistral';
    if (hostMatches('api.x.ai')) return 'xai';
    if (hostMatches('api.cohere.ai')) return 'cohere';
    if (hostMatches('openrouter.ai')) return 'openrouter';
    return 'openai';
  };

  const getMeta = (model: ActivatedModel) => {
    const cat = modelsCatalog();
    if (!cat) return null;
    return findModel(cat, getProviderIdFor(model), model.model_id);
  };

  const getModelLogo = (modelName: string) => {
    const src = getLogoByIds(null, modelName);
    return src ? <img src={src} class="w-4 h-4 object-contain" alt="" /> : null;
  };

  /** Current override model for a profile (null = no override). */
  const overrideModel = (profileId: string): ActivatedModel | null => {
    const key = profileModelOverrides()[profileId];
    if (!key) return null;
    return cloudModels().find((m) => modelKey(m) === key) ?? null;
  };

  const isModelSelectedForProfile = (profileId: string, model: ActivatedModel): boolean => {
    const key = profileModelOverrides()[profileId];
    return key === modelKey(model);
  };

  const handlePickModel = async (profileId: string, model: ActivatedModel | null) => {
    const map = { ...profileModelOverrides() };
    if (model) {
      map[profileId] = modelKey(model);
    } else {
      delete map[profileId];
    }
    setProfileModelOverrides(map);
    setOpenDropdown(null);
    await saveProfileModelOverrides();
  };

  const handleDeleteCustom = async (profileId: string) => {
    await deleteCustomSubagentProfile(profileId);
    // Also clean up any model override for this profile
    const map = { ...profileModelOverrides() };
    if (map[profileId]) {
      delete map[profileId];
      setProfileModelOverrides(map);
      await saveProfileModelOverrides();
    }
  };

  const handleCreate = async () => {
    const name = newName().trim();
    if (!name) return;
    let id = newId().trim() || slugify(name);
    if (!id) id = slugify(name);
    // Validate uniqueness
    if (BUILTIN_PROFILE_IDS.has(id) || customSubagentProfiles().some((c) => c.id === id)) {
      alert(t('subagent.duplicateProfileId', { id }));
      return;
    }
    const profile: CustomSubagentProfile = {
      id,
      name,
      description: newDescription().trim(),
      allowedTools: newAllowedTools().trim()
        ? newAllowedTools()
            .trim()
            .split(',')
            .map((s) => s.trim())
            .filter(Boolean)
        : [],
      deniedTools: newDeniedTools().trim()
        ? newDeniedTools()
            .trim()
            .split(',')
            .map((s) => s.trim())
            .filter(Boolean)
        : [],
      systemPromptExtension:
        newSystemPrompt().trim() || t('subagent.systemPromptPlaceholder', { name }),
    };
    await saveCustomSubagentProfile(profile);
    // Reset form
    setShowCreateForm(false);
    setNewId('');
    setNewName('');
    setNewDescription('');
    setNewAllowedTools('');
    setNewDeniedTools('');
    setNewSystemPrompt('');
  };

  return (
    <div class="space-y-1">
      <div class="pb-3 border-b border-[var(--border-dim)] animate-row-in">
        <h2 class="text-lg text-white font-semibold m-0">{t('subagent.title')}</h2>
        <p class="text-xs text-white/35 mt-1.5 leading-relaxed">
          {t('subagent.settingsDescription')}
        </p>
      </div>

      <For each={allProfiles()}>
        {(profile, index) => {
          const current = () => overrideModel(profile.id);
          const isOpen = () => openDropdown() === profile.id;
          return (
            <div
              class="bg-white/[0.03] rounded-xl border border-[var(--border-dim)] overflow-hidden animate-row-in"
              style={{ 'animation-delay': `${(index() + 1) * 30}ms` }}
            >
              {/* Profile header */}
              <div class="p-4 flex items-center justify-between">
                <div class="flex-1 min-w-0">
                  <div class="flex items-center gap-1.5">
                    <span class="text-sm font-semibold text-white">{profile.name}</span>
                    <span class="text-[10px] px-1.5 py-0.5 rounded bg-pri-20 text-pri font-mono">
                      {profile.id}
                    </span>
                  </div>
                  <p class="text-xs text-white/40 mt-1">{profile.description}</p>
                </div>
                <div class="flex items-center gap-2 ml-3 shrink-0">
                  <button
                    class="px-3 py-1.5 rounded-lg text-xs cursor-pointer transition-all border font-medium flex items-center gap-1.5"
                    classList={{
                      'bg-pri-20 border-pri-30 text-pri': !!current(),
                      'bg-white/[0.04] border-[var(--border-dim)] text-white/40 hover:text-white hover:bg-white/[0.06]':
                        !current(),
                    }}
                    onClick={() => setOpenDropdown(isOpen() ? null : profile.id)}
                  >
                    {current() ? getModelLogo(current()!.model_id) : null}
                    <span class="max-w-[140px] truncate">
                      {current()?.model_id ?? t('subagent.followMain')}
                    </span>
                    <span class="text-white/30 text-[10px]">&#9662;</span>
                  </button>
                  <Show when={!isBuiltin(profile.id)}>
                    <button
                      class="w-6 h-6 rounded flex items-center justify-center cursor-pointer transition-colors bg-transparent border-none text-white/25 hover:text-red-400 hover:bg-red-400/10"
                      onClick={() => void handleDeleteCustom(profile.id)}
                      title={t('subagent.deleteCustom')}
                    >
                      <Icon name="x" size={12} />
                    </button>
                  </Show>
                </div>
              </div>

              {/* Dropdown model picker */}
              <div
                class="border-t border-[var(--border-dim)] bg-[rgba(0,0,0,0.15)] transition-all duration-200 ease-out origin-top overflow-hidden"
                classList={{
                  'invisible max-h-0 opacity-0': !isOpen(),
                  'visible max-h-[260px] opacity-100': isOpen(),
                }}
              >
                <div class="max-h-[220px] overflow-y-auto p-2 scrollbar-thin">
                  {/* "Use parent model" option */}
                  <div
                    class="flex items-center gap-2.5 p-2.5 rounded-lg cursor-pointer select-none transition-all text-sm"
                    classList={{
                      'text-pri bg-pri-10': !current(),
                      'text-white/50 hover:bg-white/[0.04] hover:text-white/80': !!current(),
                    }}
                    onClick={() => void handlePickModel(profile.id, null)}
                  >
                    <span>{t('subagent.followMainModel')}</span>
                    <Show when={!!current()}>
                      <Icon name="arrow-left" size={13} class="text-white/30 ml-auto" />
                    </Show>
                    <Show when={!current()}>
                      <Icon name="check" size={13} class="ml-auto" />
                    </Show>
                  </div>

                  <div class="my-1.5 mx-2 border-t border-[var(--border-dim)]" />

                  <For each={cloudModels()}>
                    {(model) => {
                      const selected = () => isModelSelectedForProfile(profile.id, model);
                      const meta = () => getMeta(model);
                      const noKey = () => {
                        const m = model as any;
                        return !m.api_key && !m.is_remote;
                      };
                      return (
                        <div
                          class="flex items-center gap-2.5 p-2.5 text-sm rounded-lg cursor-pointer select-none transition-all"
                          classList={{
                            '!bg-[rgba(var(--primary-rgb),0.12)] !border-l-[3px] !border-[rgba(var(--primary-rgb),0.2)]':
                              selected(),
                          }}
                          onClick={() => handlePickModel(profile.id, model)}
                          style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}
                          onMouseEnter={(e) => {
                            if (!selected()) {
                              e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.06)';
                              e.currentTarget.style.color = 'white';
                            }
                          }}
                          onMouseLeave={(e) => {
                            if (!selected()) {
                              e.currentTarget.style.background = 'transparent';
                              e.currentTarget.style.color = 'rgba(var(--text-base-rgb),0.5)';
                            }
                          }}
                        >
                          <div class="w-6 h-6 bg-white rounded-full flex items-center justify-center shrink-0 shadow-sm">
                            {getModelLogo(model.model_id)}
                          </div>
                          <div class="flex-1 flex flex-col items-start justify-center overflow-hidden min-w-0">
                            <div class="max-w-[180px] text-[13px] text-white font-medium truncate">
                              {model.model_id}
                            </div>
                            <div class="flex gap-1 mt-0.5 flex-wrap">
                              <Show when={meta()}>
                                <span class="text-[9px] px-1 py-0.5 rounded bg-pri-20 text-pri">
                                  {formatContextWindow(meta()!.contextWindow)}
                                </span>
                              </Show>
                              <Show when={noKey()}>
                                <span class="text-[9px] px-1 py-0.5 rounded bg-yellow-500/20 text-yellow-300 inline-flex items-center gap-0.5">
                                  <Icon name="alert-triangle" size={9} />{' '}
                                  {t('project.noKeyConfigured')}
                                </span>
                              </Show>
                            </div>
                          </div>
                        </div>
                      );
                    }}
                  </For>

                  <Show when={cloudModels().length === 0}>
                    <div
                      class="p-4 text-center text-[13px]"
                      style={{ color: 'rgba(var(--text-base-rgb),0.2)' }}
                    >
                      <div>{t('subagent.noCloudModel')}</div>
                      <div
                        class="text-[10px] mt-1.5 leading-relaxed"
                        style={{ color: 'rgba(var(--text-base-rgb),0.25)' }}
                      >
                        {t('subagent.goToProviderSettings.prefix')}{' '}
                        <span style={{ color: 'rgba(var(--primary-rgb),0.5)', 'font-weight': 500 }}>
                          {t('subagent.goToProviderSettings.linkText')}
                        </span>
                        <br />
                        {t('subagent.goToProviderSettings.suffix')}
                      </div>
                    </div>
                  </Show>
                </div>
              </div>
            </div>
          );
        }}
      </For>

      {/* Create custom profile section */}
      <div
        class="pt-2 animate-row-in"
        style={{ 'animation-delay': `${(allProfiles().length + 1) * 30}ms` }}
      >
        <Show
          when={showCreateForm()}
          fallback={
            <button
              class="w-full py-3 rounded-xl border border-[var(--border-dim)] text-sm text-white/45 hover:text-white/70 hover:border-[var(--border-dim)] transition-all cursor-pointer bg-transparent"
              onClick={() => setShowCreateForm(true)}
            >
              {t('subagent.createCustom')}
            </button>
          }
        >
          <div class="bg-[rgba(var(--surface-bg),0.35)] rounded-xl border border-[var(--border-dim)] overflow-hidden p-4 space-y-3">
            <h3 class="text-sm text-white font-semibold m-0">{t('subagent.createTitle')}</h3>

            <div class="grid grid-cols-2 gap-3">
              <div>
                <label class="block text-[11px] text-white/40 mb-1">
                  {t('subagent.profileId')}
                </label>
                <input
                  type="text"
                  value={newId()}
                  onInput={(e) => setNewId(e.currentTarget.value)}
                  placeholder={slugify(newName())}
                  class="w-full px-2.5 py-1.5 rounded-lg bg-[rgba(0,0,0,0.2)] border border-[var(--border-dim)] text-xs text-white placeholder:text-white/15 outline-none focus:border-pri-30 transition-colors"
                />
              </div>
              <div>
                <label class="block text-[11px] text-white/40 mb-1">
                  {t('subagent.displayName')}
                </label>
                <input
                  type="text"
                  value={newName()}
                  onInput={(e) => setNewName(e.currentTarget.value)}
                  placeholder={t('subagent.namePlaceholder')}
                  class="w-full px-2.5 py-1.5 rounded-lg bg-[rgba(0,0,0,0.2)] border border-[var(--border-dim)] text-xs text-white placeholder:text-white/15 outline-none focus:border-pri-30 transition-colors"
                />
              </div>
            </div>

            <div>
              <label class="block text-[11px] text-white/40 mb-1">
                {t('subagent.description')}
              </label>
              <input
                type="text"
                value={newDescription()}
                onInput={(e) => setNewDescription(e.currentTarget.value)}
                placeholder={t('subagent.descriptionPlaceholder')}
                class="w-full px-2.5 py-1.5 rounded-lg bg-[rgba(0,0,0,0.2)] border border-[var(--border-dim)] text-xs text-white placeholder:text-white/15 outline-none focus:border-pri-30 transition-colors"
              />
            </div>

            {/* Tool multi-select dropdowns */}
            <Show
              when={availableTools().length > 0}
              fallback={
                <div class="text-[11px] text-white/25 py-3 text-center">
                  {t('subagent.noToolsAvailable')}
                </div>
              }
            >
              <div class="grid grid-cols-2 gap-3">
                <div>
                  <label class="block text-[11px] text-white/40 mb-1">
                    {t('subagent.allowedTools')}
                  </label>
                  <div class="relative" data-tool-dropdown>
                    <button
                      type="button"
                      class="w-full flex items-center justify-between px-3 py-1.5 rounded-lg text-xs text-left outline-none border border-[var(--border-dim)] transition-all duration-150 cursor-pointer"
                      style={{ background: 'rgba(0, 0, 0, 0.25)' }}
                      onClick={() => {
                        setAllowedOpen(!allowedOpen());
                        setDeniedOpen(false);
                      }}
                    >
                      <span class={allowedToolsSet().size > 0 ? 'text-white/80' : 'text-white/35'}>
                        {allowedToolsSet().size > 0
                          ? t('subagent.selectedCount', { count: allowedToolsSet().size })
                          : t('subagent.selectTools')}
                      </span>
                    </button>
                    <div
                      class="absolute z-[101] left-0 right-0 mt-1 rounded-[10px] p-1 max-h-[200px] overflow-y-auto transition-all duration-150 ease-out origin-top"
                      style={{
                        background: 'rgba(var(--surface-bg), 0.92)',
                        'backdrop-filter': 'blur(24px) saturate(150%)',
                        border: '1px solid var(--border-dim)',
                        'box-shadow': '0 12px 40px rgba(0, 0, 0, 0.45)',
                      }}
                      classList={{
                        'invisible opacity-0 scale-95 translate-y-1 pointer-events-none':
                          !allowedOpen(),
                        'visible opacity-100 scale-100 translate-y-0': allowedOpen(),
                      }}
                      onClick={(e) => e.stopPropagation()}
                    >
                      <For each={availableTools()}>
                        {(toolName) => {
                          const sel = () => allowedToolsSet().has(toolName);
                          return (
                            <div
                              class="flex items-center gap-2 px-2.5 py-1.5 rounded-md text-[11px] cursor-pointer transition-[background,color] duration-[120ms] select-none"
                              classList={{
                                'text-pri bg-pri-10': sel(),
                                'text-white/55 hover:bg-white/[0.06] hover:text-white/80': !sel(),
                              }}
                              onClick={() => toggleTool(toolName, true)}
                            >
                              <Icon
                                name={sel() ? 'check' : 'plus'}
                                size={11}
                                class={sel() ? 'text-pri' : 'text-white/25'}
                              />
                              <span class="truncate">{toolName}</span>
                            </div>
                          );
                        }}
                      </For>
                    </div>
                  </div>
                </div>
                <div>
                  <label class="block text-[11px] text-white/40 mb-1">
                    {t('subagent.disallowedTools')}
                  </label>
                  <div class="relative" data-tool-dropdown>
                    <button
                      type="button"
                      class="w-full flex items-center justify-between px-3 py-1.5 rounded-lg text-xs text-left outline-none border border-[var(--border-dim)] transition-all duration-150 cursor-pointer"
                      style={{ background: 'rgba(0, 0, 0, 0.25)' }}
                      onClick={() => {
                        setDeniedOpen(!deniedOpen());
                        setAllowedOpen(false);
                      }}
                    >
                      <span class={deniedToolsSet().size > 0 ? 'text-white/80' : 'text-white/35'}>
                        {deniedToolsSet().size > 0
                          ? t('subagent.selectedCount', { count: deniedToolsSet().size })
                          : t('subagent.selectTools')}
                      </span>
                    </button>
                    <div
                      class="absolute z-[101] left-0 right-0 mt-1 rounded-[10px] p-1 max-h-[200px] overflow-y-auto transition-all duration-150 ease-out origin-top"
                      style={{
                        background: 'rgba(var(--surface-bg), 0.92)',
                        'backdrop-filter': 'blur(24px) saturate(150%)',
                        border: '1px solid var(--border-dim)',
                        'box-shadow': '0 12px 40px rgba(0, 0, 0, 0.45)',
                      }}
                      classList={{
                        'invisible opacity-0 scale-95 translate-y-1 pointer-events-none':
                          !deniedOpen(),
                        'visible opacity-100 scale-100 translate-y-0': deniedOpen(),
                      }}
                      onClick={(e) => e.stopPropagation()}
                    >
                      <For each={availableTools()}>
                        {(toolName) => {
                          const sel = () => deniedToolsSet().has(toolName);
                          return (
                            <div
                              class="flex items-center gap-2 px-2.5 py-1.5 rounded-md text-[11px] cursor-pointer transition-[background,color] duration-[120ms] select-none"
                              classList={{
                                'text-[#ff8a8a] bg-[rgba(255,107,107,0.12)]': sel(),
                                'text-white/55 hover:bg-white/[0.06] hover:text-white/80': !sel(),
                              }}
                              onClick={() => toggleTool(toolName, false)}
                            >
                              <Icon
                                name={sel() ? 'x-circle' : 'plus'}
                                size={11}
                                class={sel() ? 'text-[#ff8a8a]' : 'text-white/25'}
                              />
                              <span class="truncate">{toolName}</span>
                            </div>
                          );
                        }}
                      </For>
                    </div>
                  </div>
                </div>
              </div>
            </Show>

            <div>
              <label class="block text-[11px] text-white/40 mb-1">
                {t('subagent.systemPromptSuffix')}
              </label>
              <textarea
                value={newSystemPrompt()}
                onInput={(e) => setNewSystemPrompt(e.currentTarget.value)}
                placeholder={t('subagent.systemPromptPlaceholder', { name: newName() || '助手' })}
                rows={3}
                class="w-full px-2.5 py-1.5 rounded-lg bg-[rgba(0,0,0,0.2)] border border-[var(--border-dim)] text-xs text-white placeholder:text-white/15 outline-none focus:border-pri-30 transition-colors resize-none"
              />
            </div>

            <div class="flex gap-2 justify-end pt-1">
              <button
                class="px-3 py-1.5 rounded-lg text-xs cursor-pointer transition-all bg-white/[0.04] border border-[var(--border-dim)] text-white/40 hover:text-white hover:bg-white/[0.06]"
                onClick={() => {
                  setShowCreateForm(false);
                  setNewName('');
                }}
              >
                {t('common.cancel')}
              </button>
              <button
                class="px-3 py-1.5 rounded-lg text-xs font-medium cursor-pointer transition-all bg-pri-20 border border-pri-30 text-pri hover:bg-pri-30"
                onClick={() => void handleCreate()}
              >
                {t('subagent.create')}
              </button>
            </div>
          </div>
        </Show>
      </div>
    </div>
  );
};

export default SubagentModelSettings;
