import { Component, createSignal, createEffect, For, Show } from 'solid-js';
import {
  datas,
  setDatas,
  saveSingleAssistantToBackend,
  setAssistantModel,
  allAvailableModels,
  isLocalModel,
  resolveAssistantModel,
  modelKey,
  ActivatedModel,
  modelsCatalog,
  mcpServers,
  mcpServerStatus,
  skills,
  startMcpServerAndRefresh,
  currentProjectId,
} from '../../../core/store/store';
import { getLogo as getLogoByIds } from '../../../core/utils/modelLogo';
import { findModel, formatContextWindow } from '../../../core/utils/models';
import { transportLabel, statusLabel, statusColor } from '../../../core/utils/mcp';
import { memoryGetStatus, memorySetEnabled } from '../../../core/api/memory';
import type { MemoryStatus } from '../../../core/types/memory';
import MemoryPanel from './MemoryPanel';
import Icon from '../../../shared/components/Icon';
import { btnClose, btnSecondary, btnSecondarySm } from '../../../shared/components/buttonStyles';
import Switch from '../../../shared/components/Switch';
import { t } from '../../../core/i18n';

interface ProjectSettingsModalProps {
  show: boolean;
  assistantId: string | null;
  onClose: () => void;
}

/**
 * Project settings modal
 * Central management for a single project's name, bound model, MCP servers, and skills.
 * - Name: saved immediately on blur/Enter
 * - Model: click takes effect immediately (setAssistantModel syncs selectedModel + persists + starts the local engine if needed)
 */
const ProjectSettingsModal: Component<ProjectSettingsModalProps> = (props) => {
  const [nameText, setNameText] = createSignal<string>('');
  const [isExiting, setIsExiting] = createSignal(false);
  const [isEntering, setIsEntering] = createSignal(true);
  const [memoryStatus, setMemoryStatus] = createSignal<MemoryStatus | null>(null);
  const [showPanel, setShowPanel] = createSignal(false);

  /** Refresh project memory status when the modal opens */
  createEffect(() => {
    if (props.show && currentProjectId()) {
      void memoryGetStatus(currentProjectId()!)
        .then(setMemoryStatus)
        .catch(() => setMemoryStatus(null));
    }
  });

  const memoryEnabled = () => memoryStatus()?.enabled ?? false;
  const memoryReady = () => memoryStatus()?.embedder.available ?? false;
  const memoryStatsText = () => {
    const s = memoryStatus()?.stats;
    if (!s) return '';
    return t('project.memory.stats', {
      total: String(s.totalFacts),
      active: String(s.activeFacts),
      embedded: String(s.embeddedFacts),
    });
  };
  const toggleMemory = async (enabled: boolean) => {
    const pid = currentProjectId();
    if (!pid) return;
    try {
      await memorySetEnabled(pid, enabled);
      const st = await memoryGetStatus(pid);
      setMemoryStatus(st);
    } catch (err) {
      console.warn('Failed to toggle project memory:', err);
    }
  };

  /** The assistant being edited (reactive) */
  const asst = () =>
    datas.assistants.find((a: any) => a.id === props.assistantId) as
      | {
          id: string;
          name: string;
          prompt: string;
          modelId?: string;
          mcpServerIds?: string[];
          skillIds?: string[];
          agentMode?: string;
        }
      | undefined;

  /** Sync the name into local edit state when the modal opens, and trigger the enter animation */
  createEffect(() => {
    if (props.show && props.assistantId) {
      const a = asst();
      setNameText(a?.name ?? '');
      setIsEntering(true);
      setTimeout(() => setIsEntering(false), 0);
    }
  });

  const handleClose = () => {
    setIsExiting(true);
    setTimeout(() => {
      setIsExiting(false);
      props.onClose();
    }, 300);
  };

  /** Save the project name immediately */
  const saveName = async () => {
    const id = props.assistantId;
    const newName = nameText().trim();
    if (!id || !newName) return;
    setDatas('assistants', (a) => a.id === id, 'name', newName);
    await saveSingleAssistantToBackend(id);
  };

  /** Pick a model: bind at once + sync global state + persist (local models also start the engine) */
  const handlePickModel = (model: ActivatedModel) => {
    const id = props.assistantId;
    if (!id) return;
    void setAssistantModel(id, model);
  };

  /** Enable or disable an MCP server for the current assistant and persist immediately.
   *  Auto-starts the server if it is not running when enabled. */
  const handleToggleMcpServer = async (serverId: string, enabled: boolean) => {
    const id = props.assistantId;
    const current = asst()?.mcpServerIds ?? [];
    if (!id) return;

    const next = enabled
      ? Array.from(new Set([...current, serverId]))
      : current.filter((existingId) => existingId !== serverId);
    setDatas('assistants', (a) => a.id === id, 'mcpServerIds', next);
    await saveSingleAssistantToBackend(id);

    // Auto-start the server if it is not running when enabled
    if (enabled) {
      const status = mcpServerStatus()[serverId]?.status;
      if (!status || status === 'disconnected' || status === 'error') {
        void startMcpServerAndRefresh(serverId, currentProjectId());
      }
    }
  };

  const sortedMcpServers = () =>
    Object.values(mcpServers())
      .filter((s) => s.id !== '__aio-filesystem__')
      .sort((a, b) => a.displayName.localeCompare(b.displayName));

  /** Enable or disable a skill for the current assistant and persist immediately. */
  const handleToggleSkill = async (skillId: string, enabled: boolean) => {
    const id = props.assistantId;
    const current = asst()?.skillIds ?? [];
    if (!id) return;

    const next = enabled
      ? Array.from(new Set([...current, skillId]))
      : current.filter((existingId) => existingId !== skillId);
    setDatas('assistants', (a) => a.id === id, 'skillIds', next);
    await saveSingleAssistantToBackend(id);
  };

  const sortedSkills = () => Object.values(skills()).sort((a, b) => a.name.localeCompare(b.name));

  // NOTE: branch copy of getProviderIdFor in SubagentModelSettings.tsx;
  // behavior differs slightly (this version lacks .trim() and the deepseek.ai check).
  // Confirm the difference is intentional before merging. See also detectProviderByUrl in core/utils/models.ts.
  const getProviderIdFor = (model: ActivatedModel): string => {
    if ((model as any).provider_id) return (model as any).provider_id;

    const getHostname = (rawUrl: string): string => {
      try {
        return new URL(rawUrl).hostname.toLowerCase();
      } catch {
        return '';
      }
    };

    const hostMatches = (host: string, allowedHost: string): boolean =>
      host === allowedHost || host.endsWith(`.${allowedHost}`);

    const host = getHostname(model.api_url || '');
    if (hostMatches(host, 'api.openai.com')) return 'openai';
    if (hostMatches(host, 'api.anthropic.com')) return 'anthropic';
    if (hostMatches(host, 'generativelanguage.googleapis.com')) return 'google';
    if (hostMatches(host, 'api.deepseek.com')) return 'deepseek';
    if (hostMatches(host, 'api.groq.com')) return 'groq';
    if (hostMatches(host, 'api.mistral.ai')) return 'mistral';
    if (hostMatches(host, 'api.x.ai')) return 'xai';
    if (hostMatches(host, 'api.cohere.ai')) return 'cohere';
    if (hostMatches(host, 'openrouter.ai')) return 'openrouter';
    return 'openai';
  };

  const getMeta = (model: ActivatedModel) => {
    const cat = modelsCatalog();
    if (!cat) return null;
    const pid = getProviderIdFor(model);
    return findModel(cat, pid, model.model_id);
  };

  const getModelLogo = (modelName: string) => getLogoByIds(null, modelName);

  /** Currently effective model (used to echo the "in use" marker) */
  const activeModel = () => resolveAssistantModel(asst() as any);

  const cloudModels = () => allAvailableModels().filter((m) => !isLocalModel(m));
  const localModels = () => allAvailableModels().filter((m) => isLocalModel(m));

  /**
   * Whether a model is the "currently selected" one.
   * Matches exactly by the composite modelKey: when the assistant has a bound modelId, match against it;
   * otherwise fall back to the key of the currently effective model (resolveAssistantModel).
   */
  const isSelected = (model: ActivatedModel): boolean => {
    const bound = asst()?.modelId;
    const targetKey = bound ?? (activeModel() ? modelKey(activeModel()!) : null);
    if (!targetKey) return false;
    if (modelKey(model) === targetKey) return true;
    // Legacy data compat: when the bound modelId is a bare id (no @), fall back to matching local models by model_id
    if (bound && !bound.includes('@') && isLocalModel(model) && model.model_id === bound)
      return true;
    return false;
  };

  return (
    <>
      <Show when={props.show}>
        <div
          classList={{
            'opacity-0 pointer-events-none': isExiting() || isEntering(),
            'opacity-100': !isExiting() && !isEntering(),
          }}
          class="fixed inset-0 z-[1000] flex items-center justify-center bg-black/25 rounded-lg transition-all duration-200 ease-out"
          onClick={(e) => e.target === e.currentTarget && handleClose()}
        >
          <div
            classList={{
              'scale-95 opacity-0': isExiting() || isEntering(),
              'scale-100 opacity-100': !isExiting() && !isEntering(),
            }}
            class="rounded-lg text-[rgba(var(--text-base-rgb),0.88)] p-6 w-[92%] max-w-[640px] max-h-[90vh] overflow-y-auto flex flex-col gap-4 transition-all duration-500 ease-out transform"
            style={{
              background: 'var(--dialog-bg)',
              'backdrop-filter': 'var(--popup-blur)',
              border: '1px solid var(--border-dim)',
            }}
          >
            <div class="flex justify-between items-center border-b border-[rgba(var(--text-base-rgb),0.27)] pb-3">
              <h2 class="m-0 text-xl">{t('project.settings')}</h2>
              <button onClick={handleClose} class={btnClose} aria-label={t('common.close')}>
                <Icon name="x" size={18} class="stroke-[2.5]" />
              </button>
            </div>

            {/* Name */}
            <div class="flex flex-col gap-1.5">
              <label class="text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold">
                {t('project.name')}
              </label>
              <input
                value={nameText()}
                onInput={(e) => setNameText(e.currentTarget.value)}
                onBlur={() => saveName()}
                onKeyDown={(e) => e.key === 'Enter' && saveName()}
                placeholder={t('project.namePlaceholder')}
                class="w-full p-2.5 bg-dark-300 border border-dark-100 rounded-lg text-[rgba(var(--text-base-rgb),0.88)] text-sm focus:outline-none focus:border-pri-50"
              />
            </div>

            {/* Model */}
            <div class="flex flex-col gap-1.5">
              <label class="text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold">
                {t('project.bindModel')}
              </label>
              <div class="flex flex-row h-[240px] rounded-lg overflow-hidden border border-dark-100">
                {/* Cloud models */}
                <div class="flex-1 flex flex-col min-w-0">
                  <div
                    class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest"
                    style={{
                      color: 'rgba(var(--text-base-rgb),0.35)',
                      background: 'rgba(var(--text-base-rgb),0.04)',
                      'border-bottom': '1px solid var(--border-dim)',
                    }}
                  >
                    {t('chat.model.cloud')}
                  </div>
                  <div class="flex-1 overflow-y-auto p-1.5 scrollbar-thin">
                    <For each={cloudModels()}>
                      {(model) => {
                        const meta = () => getMeta(model);
                        const noKey = () => !model.api_key;
                        const selected = () => isSelected(model);
                        return (
                          <div
                            class="flex flex-row items-center gap-2.5 p-2 text-sm rounded-lg cursor-pointer select-none transition-all"
                            style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}
                            classList={{
                              '!bg-[rgba(var(--primary-rgb),0.12)] !border-l-[3px] !border-[rgba(var(--primary-rgb),0.2)]':
                                selected(),
                            }}
                            onClick={() => handlePickModel(model)}
                            onMouseEnter={(e) => {
                              if (!selected()) {
                                e.currentTarget.style.background =
                                  'rgba(var(--text-base-rgb),0.06)';
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
                            <div class="w-7 h-7 bg-white rounded-full flex items-center justify-center shrink-0 shadow-sm">
                              <img
                                src={getModelLogo(model.model_id)}
                                alt="logo"
                                class="w-[18px] h-[18px] object-contain"
                              />
                            </div>
                            <div class="flex-1 flex flex-col items-start justify-center overflow-hidden text-left min-w-0">
                              <div class="max-w-[200px] text-[13px] text-white font-medium truncate">
                                {model.model_id}
                              </div>
                              <div
                                style={{
                                  color: 'rgba(var(--primary-rgb),0.5)',
                                  'font-size': '10px',
                                }}
                              >
                                {model.owned_by}
                              </div>
                              <div class="flex gap-1 mt-0.5 flex-wrap">
                                <Show when={meta()}>
                                  <span class="text-[9px] px-1 py-0.5 rounded bg-pri-20 text-pri">
                                    {formatContextWindow(meta()!.contextWindow)}
                                  </span>
                                  <Show when={meta()!.capabilities.tools}>
                                    <span class="text-[9px] px-1 py-0.5 rounded bg-green-500/20 text-green-300">
                                      {t('project.capability.tools')}
                                    </span>
                                  </Show>
                                  <Show when={meta()!.capabilities.vision}>
                                    <span class="text-[9px] px-1 py-0.5 rounded bg-blue-500/20 text-blue-300">
                                      {t('project.capability.vision')}
                                    </span>
                                  </Show>
                                  <Show when={meta()!.capabilities.reasoning}>
                                    <span class="text-[9px] px-1 py-0.5 rounded bg-purple-500/20 text-purple-300">
                                      {t('project.capability.reasoning')}
                                    </span>
                                  </Show>
                                  <Show when={meta()!.status === 'deprecated'}>
                                    <span class="text-[9px] px-1 py-0.5 rounded bg-red-500/20 text-red-300">
                                      {t('project.capability.deprecated')}
                                    </span>
                                  </Show>
                                </Show>
                                <Show when={noKey()}>
                                  <span
                                    class="text-[9px] px-1 py-0.5 rounded bg-yellow-500/20 text-yellow-300 inline-flex items-center gap-0.5"
                                    title={t('project.noKeyTitle') as string}
                                  >
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
                        class="p-5 text-center text-[13px]"
                        style={{ color: 'rgba(var(--text-base-rgb),0.2)' }}
                      >
                        <div>{t('chat.model.noCloud')}</div>
                        <div
                          class="text-[10px] mt-1.5 leading-relaxed"
                          style={{ color: 'rgba(var(--text-base-rgb),0.25)' }}
                        >
                          {t('chat.model.noCloudGoTo')}{' '}
                          <span
                            style={{
                              color: 'rgba(var(--primary-rgb),0.5)',
                              'font-weight': '500',
                            }}
                          >
                            {t('chat.model.noCloudSettingsPath')}
                          </span>
                          <br />
                          {t('chat.model.noCloudEnableProvider')}
                        </div>
                      </div>
                    </Show>
                  </div>
                </div>
                <div
                  style={{
                    width: '1px',
                    background: 'rgba(var(--text-base-rgb),0.04)',
                    'align-self': 'stretch',
                  }}
                />
                {/* Local models */}
                <div class="flex-1 flex flex-col min-w-0">
                  <div
                    class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest"
                    style={{
                      color: 'rgba(var(--text-base-rgb),0.35)',
                      background: 'rgba(var(--text-base-rgb),0.04)',
                      'border-bottom': '1px solid var(--border-dim)',
                    }}
                  >
                    {t('chat.model.local')}
                  </div>
                  <div class="flex-1 overflow-y-auto p-1.5 scrollbar-thin">
                    <For each={localModels()}>
                      {(model) => {
                        const selected = () => isSelected(model);
                        return (
                          <div
                            class="flex flex-row items-center gap-2.5 p-2 text-sm rounded-lg cursor-pointer select-none transition-all"
                            style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}
                            classList={{
                              '!bg-[rgba(var(--primary-rgb),0.12)] !border-l-[3px] !border-[rgba(var(--primary-rgb),0.2)]':
                                selected(),
                            }}
                            onClick={() => handlePickModel(model)}
                            onMouseEnter={(e) => {
                              if (!selected()) {
                                e.currentTarget.style.background =
                                  'rgba(var(--text-base-rgb),0.06)';
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
                            <div class="w-7 h-7 bg-white rounded-full flex items-center justify-center shrink-0 shadow-sm">
                              <img
                                src={getModelLogo(model.model_id)}
                                alt="logo"
                                class="w-[18px] h-[18px] object-contain"
                              />
                            </div>
                            <div class="flex-1 flex flex-col items-start justify-center overflow-hidden text-left min-w-0">
                              <div class="max-w-[180px] text-[13px] text-white font-medium truncate">
                                {model.model_id}
                              </div>
                              <div
                                style={{
                                  color: 'rgba(var(--primary-rgb),0.5)',
                                  'font-size': '10px',
                                }}
                              >
                                {model.owned_by}
                              </div>
                            </div>
                          </div>
                        );
                      }}
                    </For>
                    <Show when={localModels().length === 0}>
                      <div
                        class="p-5 text-center text-[13px]"
                        style={{ color: 'rgba(var(--text-base-rgb),0.2)' }}
                      >
                        {t('chat.model.noLocal')}
                      </div>
                    </Show>
                  </div>
                </div>
              </div>
            </div>

            {/* Project memory */}
            <div class="flex flex-col gap-1.5">
              <label class="text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold">
                {t('project.memory.title')}
              </label>
              <div class="flex flex-col gap-2 rounded-lg border border-dark-100 p-2.5">
                <div class="flex items-center justify-between gap-3">
                  <div class="flex-1 min-w-0">
                    <div class="text-sm text-white truncate">{t('project.memory.enabled')}</div>
                    <div class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}>
                      {memoryReady()
                        ? t('project.memory.statusReady')
                        : t('project.memory.statusNotReady')}
                    </div>
                    <Show when={memoryStatsText()}>
                      <div class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}>
                        {memoryStatsText()}
                      </div>
                    </Show>
                  </div>
                  <Switch
                    checked={memoryEnabled()}
                    label={t('project.memory.enabled')}
                    onChange={(enabled) => void toggleMemory(enabled)}
                  />
                </div>
                <div class="flex justify-end">
                  <button
                    class={btnSecondarySm}
                    onClick={() => {
                      // Exit the project settings modal first, then open the memory panel (avoids the panel being covered by the modal)
                      setShowPanel(true);
                      handleClose();
                    }}
                  >
                    {t('project.memory.openPanel')}
                  </button>
                </div>
              </div>
            </div>

            {/* MCP servers */}
            <div class="flex flex-col gap-1.5">
              <label class="text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold">
                {t('mcp.title')}
                <span
                  class="ml-2 text-[11px] font-normal"
                  style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                >
                  {t('project.skillHint')}
                </span>
              </label>
              <div class="flex flex-col gap-1.5 max-h-[180px] overflow-y-auto rounded-lg border border-dark-100 p-1.5">
                <For each={sortedMcpServers()}>
                  {(server) => {
                    const checked = () => (asst()?.mcpServerIds ?? []).includes(server.id);
                    const status = () => mcpServerStatus()[server.id]?.status ?? 'disconnected';
                    return (
                      <div class="flex items-center gap-3 rounded-md px-2.5 py-2 transition-colors hover:bg-white/5">
                        <div class="flex-1 min-w-0">
                          <div class="text-sm text-white truncate">
                            {server.displayName || server.id}
                          </div>
                          <div
                            class="text-[11px] truncate"
                            style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                          >
                            {transportLabel(server.transport)}
                          </div>
                        </div>
                        <span
                          class="px-1.5 py-0.5 rounded text-[10px] shrink-0"
                          style={{
                            background: `${statusColor(status())}22`,
                            color: statusColor(status()),
                          }}
                        >
                          {statusLabel(status())}
                        </span>
                        <Switch
                          checked={checked()}
                          label={t('project.mcpEnable', { name: server.displayName || server.id })}
                          onChange={(enabled) => void handleToggleMcpServer(server.id, enabled)}
                        />
                      </div>
                    );
                  }}
                </For>
                <Show when={sortedMcpServers().length === 0}>
                  <div
                    class="px-3 py-5 text-center text-xs"
                    style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}
                  >
                    {t('project.mcpEmpty')}
                  </div>
                </Show>
              </div>
            </div>

            {/* Skills */}
            <div class="flex flex-col gap-1.5">
              <label class="text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold">
                {t('project.skill')}
                <span
                  class="ml-2 text-[11px] font-normal"
                  style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                >
                  {t('project.skillHint')}
                </span>
              </label>
              <div class="flex flex-col gap-1.5 max-h-[180px] overflow-y-auto rounded-lg border border-dark-100 p-1.5">
                <For each={sortedSkills()}>
                  {(skill) => {
                    const checked = () => (asst()?.skillIds ?? []).includes(skill.id);
                    return (
                      <div class="flex items-center gap-3 rounded-md px-2.5 py-2 transition-colors hover:bg-white/5">
                        <div class="flex-1 min-w-0">
                          <div class="text-sm text-white truncate">{skill.name}</div>
                          <div
                            class="text-[11px] line-clamp-2"
                            style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                          >
                            {skill.description || skill.content}
                          </div>
                        </div>
                        <Switch
                          checked={checked()}
                          label={t('project.skillEnable', { name: skill.name })}
                          onChange={(enabled) => void handleToggleSkill(skill.id, enabled)}
                        />
                      </div>
                    );
                  }}
                </For>
                <Show when={sortedSkills().length === 0}>
                  <div
                    class="px-3 py-5 text-center text-xs"
                    style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}
                  >
                    {t('project.skillEmpty')}
                  </div>
                </Show>
              </div>
            </div>

            <div class="flex justify-end gap-3">
              <button onClick={handleClose} class={btnSecondary}>
                {t('common.close')}
              </button>
            </div>
          </div>
        </div>
      </Show>
      <MemoryPanel
        show={showPanel()}
        projectId={currentProjectId() ?? ''}
        onClose={() => setShowPanel(false)}
      />
    </>
  );
};

export default ProjectSettingsModal;
