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
import { memoryGetStatus, memorySetEnabled } from '../../../core/utils/memory';
import type { MemoryStatus } from '../../../core/types/memory';
import MemoryPanel from './MemoryPanel';
import Icon from '../../../shared/components/Icon';
import Switch from '../../../shared/components/Switch';
import { t } from '../../../core/i18n';

interface ProjectSettingsModalProps {
  show: boolean;
  assistantId: string | null;
  onClose: () => void;
}

/**
 * 项目设置弹窗
 * 集中管理单个项目的：名称、绑定模型、MCP 服务器、Skill。
 * - 名称：blur/Enter 即时存
 * - 模型：点击即时生效（setAssistantModel 立即同步 selectedModel + 持久化 + 必要时拉起本地引擎）
 */
const ProjectSettingsModal: Component<ProjectSettingsModalProps> = (props) => {
  const [nameText, setNameText] = createSignal<string>('');
  const [isExiting, setIsExiting] = createSignal(false);
  const [isEntering, setIsEntering] = createSignal(true);
  const [memoryStatus, setMemoryStatus] = createSignal<MemoryStatus | null>(null);
  const [showPanel, setShowPanel] = createSignal(false);

  /** 弹窗打开时刷新项目记忆状态 */
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
      console.warn('切换项目记忆失败:', err);
    }
  };

  /** 当前编辑的助手对象（响应式） */
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

  /** 弹窗打开时同步名称到本地编辑态，并触发入场动画 */
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

  /** 即时保存项目名称 */
  const saveName = async () => {
    const id = props.assistantId;
    const newName = nameText().trim();
    if (!id || !newName) return;
    setDatas('assistants', (a) => a.id === id, 'name', newName);
    await saveSingleAssistantToBackend(id);
  };

  /** 选中某模型：立即绑定 + 同步全局 + 持久化（本地模型顺带拉起引擎） */
  const handlePickModel = (model: ActivatedModel) => {
    const id = props.assistantId;
    if (!id) return;
    void setAssistantModel(id, model);
  };

  /** 为当前助手启用或停用一个 MCP server，并立即持久化。
   *  启用时若 server 未运行则自动启动。 */
  const handleToggleMcpServer = async (serverId: string, enabled: boolean) => {
    const id = props.assistantId;
    const current = asst()?.mcpServerIds ?? [];
    if (!id) return;

    const next = enabled
      ? Array.from(new Set([...current, serverId]))
      : current.filter((existingId) => existingId !== serverId);
    setDatas('assistants', (a) => a.id === id, 'mcpServerIds', next);
    await saveSingleAssistantToBackend(id);

    // 启用时若 server 未运行，自动启动它
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

  /** 为当前助手启用或停用一个 Skill，并立即持久化。 */
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

  // NOTE: 此函数与 SubagentModelSettings.tsx 中的 getProviderIdFor 存在分支副本；
  // 两者行为略有差异（此版本不含 .trim() 和 deepseek.ai 检查），
  // 合并前需确认行为差异是否有意。另见 core/utils/models.ts 中的 detectProviderByUrl。
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

  /** 当前生效模型（用于回显「当前使用」标识） */
  const activeModel = () => resolveAssistantModel(asst() as any);

  const cloudModels = () => allAvailableModels().filter((m) => !isLocalModel(m));
  const localModels = () => allAvailableModels().filter((m) => isLocalModel(m));

  /**
   * 判断某模型是否为「当前选中」。
   * 统一用复合键 modelKey 精确匹配：助手绑定 modelId 时按其匹配，
   * 未绑定时回退到当前生效模型（resolveAssistantModel）的键。
   */
  const isSelected = (model: ActivatedModel): boolean => {
    const bound = asst()?.modelId;
    const targetKey = bound ?? (activeModel() ? modelKey(activeModel()!) : null);
    if (!targetKey) return false;
    if (modelKey(model) === targetKey) return true;
    // 兼容旧数据：绑定的 modelId 为纯 id（无 @）时，按 model_id 兜底匹配本地模型
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
          class="fixed inset-0 z-[1000] flex items-center justify-center bg-black/60 backdrop-blur-[12px] rounded-lg transition-all duration-200 ease-out"
          onClick={(e) => e.target === e.currentTarget && handleClose()}
        >
          <div
            classList={{
              'scale-95 opacity-0': isExiting() || isEntering(),
              'scale-100 opacity-100': !isExiting() && !isEntering(),
            }}
            class="rounded-lg bg-dark-500 text-[#e0e0e0] p-6 w-[92%] max-w-[640px] max-h-[90vh] overflow-y-auto flex flex-col gap-4 transition-all duration-500 ease-out transform"
            style={{
              background: 'var(--acrylic-bg)',
              'backdrop-filter': 'blur(var(--acrylic-blur))',
              border: '1px solid var(--acrylic-border)',
            }}
          >
            <div class="flex justify-between items-center border-b border-[#444] pb-3">
              <h2 class="m-0 text-xl">{t('project.settings')}</h2>
              <button
                onClick={handleClose}
                class="w-8 h-8 rounded-lg bg-transparent border-none text-2xl cursor-pointer leading-none p-0 transition-all duration-200 text-white/40 hover:text-white hover:bg-danger/80"
              >
                &times;
              </button>
            </div>

            {/* 名称 */}
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
                class="w-full p-2.5 bg-dark-300 border border-dark-100 rounded-lg text-[#e0e0e0] text-sm focus:outline-none focus:border-pri-50"
              />
            </div>

            {/* 模型 */}
            <div class="flex flex-col gap-1.5">
              <label class="text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold">
                {t('project.bindModel')}
                <Show when={activeModel()}>
                  <span
                    class="ml-2 text-[11px] font-normal"
                    style={{ color: 'rgba(var(--primary-rgb),0.7)' }}
                  >
                    {t('project.currentModel', { model: activeModel()!.model_id })}
                  </span>
                </Show>
              </label>
              <div class="flex flex-row h-[240px] rounded-lg overflow-hidden border border-dark-100">
                {/* 线上模型 */}
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
                          去{' '}
                          <span
                            style={{
                              color: 'rgba(var(--primary-rgb),0.5)',
                              'font-weight': '500',
                            }}
                          >
                            设置中心 → 供应商设置
                          </span>
                          <br />
                          启用 provider 并填写 API Key
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
                {/* 本地模型 */}
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

            {/* 项目记忆 */}
            <div class="flex flex-col gap-1.5">
              <label class="text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold">
                {t('project.memory.title')}
                <span
                  class="ml-2 text-[11px] font-normal"
                  style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                >
                  {t('project.memory.description')}
                </span>
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
                    class="px-2.5 py-1.5 rounded-md text-[11px] font-semibold bg-pri-20 text-pri border border-pri/30 cursor-pointer hover:bg-pri-30 transition-colors"
                    onClick={() => setShowPanel(true)}
                  >
                    {t('project.memory.openPanel')}
                  </button>
                </div>
                <Show when={!memoryReady()}>
                  <div class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}>
                    {t('project.memory.configureHint')}
                  </div>
                </Show>
              </div>
            </div>

            {/* MCP 服务器 */}
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

            {/* Skill */}
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
              <button
                onClick={handleClose}
                class="px-5 py-2.5 border-0 cursor-pointer font-bold bg-dark-100 text-[#e0e0e0] rounded-lg transition-all duration-200 hover:bg-dark-50"
              >
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
