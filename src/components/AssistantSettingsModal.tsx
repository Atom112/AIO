import { Component, createSignal, createEffect, For, Show } from 'solid-js';
import {
    datas, setDatas, saveSingleAssistantToBackend, setAssistantModel,
    allAvailableModels, isLocalModel, resolveAssistantModel, modelKey,
    ActivatedModel, modelsCatalog,
    mcpServers, mcpServerStatus, skills,
} from '../store/store';
import { getLogo as getLogoByIds } from '../utils/modelLogo';
import { findModel, formatContextWindow } from '../utils/models';
import { transportLabel, statusLabel, statusColor } from '../utils/mcp';
import Icon from './Icon';

interface AssistantSettingsModalProps {
    show: boolean;
    assistantId: string | null;
    onClose: () => void;
}

/**
 * 助手设置弹窗
 * 集中管理单个助手的：名称、绑定模型、系统提示词。
 * - 名称：blur/Enter 即时存（复用 AssistantSidebar 的 rename 模式）
 * - 模型：点击即时生效（setAssistantModel 立即同步 selectedModel + 持久化 + 必要时拉起本地引擎）
 * - 系统提示词：显式「保存」按钮持久化（沿用原 PromptModal 的交互习惯）
 */
const AssistantSettingsModal: Component<AssistantSettingsModalProps> = (props) => {

    const [nameText, setNameText] = createSignal<string>('');
    const [promptText, setPromptText] = createSignal<string>('');
    const [isExiting, setIsExiting] = createSignal(false);
    const [isEntering, setIsEntering] = createSignal(true);

    /** 当前编辑的助手对象（响应式） */
    const asst = () => datas.assistants.find((a: any) => a.id === props.assistantId) as
        | { id: string; name: string; prompt: string; modelId?: string; mcpServerIds?: string[]; skillIds?: string[] } | undefined;

    /** 弹窗打开时同步名称与提示词到本地编辑态，并触发入场动画 */
    createEffect(() => {
        if (props.show && props.assistantId) {
            const a = asst();
            setNameText(a?.name ?? '');
            setPromptText(a?.prompt ?? '');
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

    /** 即时保存助手名称 */
    const saveName = async () => {
        const id = props.assistantId;
        const newName = nameText().trim();
        if (!id || !newName) return;
        setDatas('assistants', a => a.id === id, 'name', newName);
        await saveSingleAssistantToBackend(id);
    };

    /** 保存系统提示词并关闭 */
    const handleSavePrompt = async () => {
        const id = props.assistantId;
        if (id) {
            setDatas('assistants', a => a.id === id, 'prompt', promptText());
            await saveSingleAssistantToBackend(id);
        }
        handleClose();
    };

    /** 选中某模型：立即绑定 + 同步全局 + 持久化（本地模型顺带拉起引擎） */
    const handlePickModel = (model: ActivatedModel) => {
        const id = props.assistantId;
        if (!id) return;
        void setAssistantModel(id, model);
    };

    /** 为当前助手启用或停用一个 MCP server，并立即持久化。 */
    const handleToggleMcpServer = async (serverId: string, enabled: boolean) => {
        const id = props.assistantId;
        const current = asst()?.mcpServerIds ?? [];
        if (!id) return;

        const next = enabled
            ? Array.from(new Set([...current, serverId]))
            : current.filter(existingId => existingId !== serverId);
        setDatas('assistants', a => a.id === id, 'mcpServerIds', next);
        await saveSingleAssistantToBackend(id);
    };

    const sortedMcpServers = () =>
        Object.values(mcpServers()).sort((a, b) => a.displayName.localeCompare(b.displayName));

    /** 为当前助手启用或停用一个 Skill，并立即持久化。 */
    const handleToggleSkill = async (skillId: string, enabled: boolean) => {
        const id = props.assistantId;
        const current = asst()?.skillIds ?? [];
        if (!id) return;

        const next = enabled
            ? Array.from(new Set([...current, skillId]))
            : current.filter(existingId => existingId !== skillId);
        setDatas('assistants', a => a.id === id, 'skillIds', next);
        await saveSingleAssistantToBackend(id);
    };

    const sortedSkills = () =>
        Object.values(skills()).sort((a, b) => a.name.localeCompare(b.name));

    /** 推导 model 的 provider id（用于查 catalog 元数据，与 ModelDropdown 一致） */
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

    const cloudModels = () => allAvailableModels().filter(m => !isLocalModel(m));
    const localModels = () => allAvailableModels().filter(m => isLocalModel(m));

    /**
     * 判断某模型是否为「当前选中」。
     * 统一用复合键 modelKey 精确匹配：助手绑定 modelId 时按其匹配，
     * 未绑定时回退到当前生效模型（resolveAssistantModel）的键。
     * 复合键为 `model_id@api_url`（本地模型为纯 model_id），可区分同名异源模型，
     * 彻底避免「选 B 后 A 仍高亮」的问题。
     */
    const isSelected = (model: ActivatedModel): boolean => {
        const bound = asst()?.modelId;
        const targetKey = bound ?? (activeModel() ? modelKey(activeModel()!) : null);
        if (!targetKey) return false;
        if (modelKey(model) === targetKey) return true;
        // 兼容旧数据：绑定的 modelId 为纯 id（无 @）时，按 model_id 兜底匹配本地模型
        if (bound && !bound.includes('@') && isLocalModel(model) && model.model_id === bound) return true;
        return false;
    };

    return (
        <Show when={props.show}>
            <div
                classList={{
                    "opacity-0 pointer-events-none": isExiting() || isEntering(),
                    "opacity-100": !isExiting() && !isEntering()
                }}
                class="modal-overlay bg-black/60 z-[1000] rounded-lg transition-all duration-200 ease-out"
                onClick={(e) => e.target === e.currentTarget && handleClose()}
            >
                <div
                    classList={{
                        "scale-95 opacity-0": isExiting() || isEntering(),
                        "scale-100 opacity-100": !isExiting() && !isEntering()
                    }}
                    class="modal-panel bg-dark-500 text-[#e0e0e0] p-6 rounded-lg w-[92%] max-w-[640px] max-h-[90vh] overflow-y-auto flex flex-col gap-4 transition-all duration-500 ease-out transform"
                >
                    <div class="flex justify-between items-center border-b border-[#444] pb-3">
                        <h2 class='m-0 text-xl'>助手设置</h2>
                        <button onClick={handleClose} class="close-btn">&times;</button>
                    </div>

                    {/* 名称 */}
                    <div class="flex flex-col gap-1.5">
                        <label class="section-label">助手名称</label>
                        <input
                            value={nameText()}
                            onInput={(e) => setNameText(e.currentTarget.value)}
                            onBlur={() => saveName()}
                            onKeyDown={(e) => e.key === 'Enter' && saveName()}
                            placeholder="例如：翻译助手"
                            class="w-full p-2.5 bg-dark-300 border border-dark-100 rounded-lg text-[#e0e0e0] text-sm focus:outline-none focus:border-pri-50"
                        />
                    </div>

                    {/* 模型 */}
                    <div class="flex flex-col gap-1.5">
                        <label class="section-label">
                            绑定模型
                            <Show when={activeModel()}>
                                <span class="ml-2 text-[11px] font-normal" style="color: rgba(124,154,191,0.7);">
                                    当前：{activeModel()!.model_id}
                                </span>
                            </Show>
                        </label>
                        <div class="flex flex-row h-[240px] rounded-lg overflow-hidden border border-dark-100">
                            {/* 线上模型 */}
                            <div class="flex-1 flex flex-col min-w-0">
                                <div class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest"
                                    style="color: rgba(255,255,255,0.35); background: rgba(255,255,255,0.04); border-bottom: 1px solid rgba(255,255,255,0.04);">
                                    线上模型
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
                                                    style="color: rgba(255,255,255,0.5);"
                                                    classList={{
                                                        '!bg-[rgba(124,154,191,0.12)] !border-l-[3px] !border-[rgba(124,154,191,0.2)]': selected()
                                                    }}
                                                    onClick={() => handlePickModel(model)}
                                                    onMouseEnter={(e) => { if (!selected()) { e.currentTarget.style.background = 'rgba(255,255,255,0.06)'; e.currentTarget.style.color = 'white'; } }}
                                                    onMouseLeave={(e) => { if (!selected()) { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = 'rgba(255,255,255,0.5)'; } }}>
                                                    <div class="w-7 h-7 bg-white rounded-full flex items-center justify-center shrink-0 shadow-sm">
                                                        <img src={getModelLogo(model.model_id)} alt="logo" class="w-[18px] h-[18px] object-contain" />
                                                    </div>
                                                    <div class="flex-1 flex flex-col items-start justify-center overflow-hidden text-left min-w-0">
                                                        <div class="max-w-[200px] text-[13px] text-white font-medium truncate">{model.model_id}</div>
                                                        <div style="color: rgba(124,154,191,0.5); font-size: 10px;">{model.owned_by}</div>
                                                        <div class="flex gap-1 mt-0.5 flex-wrap">
                                                            <Show when={meta()}>
                                                                <span class="text-[9px] px-1 py-0.5 rounded bg-pri-20 text-pri">{formatContextWindow(meta()!.contextWindow)}</span>
                                                                <Show when={meta()!.capabilities.tools}>
                                                                    <span class="text-[9px] px-1 py-0.5 rounded bg-green-500/20 text-green-300">工具</span>
                                                                </Show>
                                                                <Show when={meta()!.capabilities.vision}>
                                                                    <span class="text-[9px] px-1 py-0.5 rounded bg-blue-500/20 text-blue-300">视觉</span>
                                                                </Show>
                                                                <Show when={meta()!.capabilities.reasoning}>
                                                                    <span class="text-[9px] px-1 py-0.5 rounded bg-purple-500/20 text-purple-300">推理</span>
                                                                </Show>
                                                                <Show when={meta()!.status === 'deprecated'}>
                                                                    <span class="text-[9px] px-1 py-0.5 rounded bg-red-500/20 text-red-300">已弃用</span>
                                                                </Show>
                                                            </Show>
                                                            <Show when={noKey()}>
                                                                <span class="text-[9px] px-1 py-0.5 rounded bg-yellow-500/20 text-yellow-300 inline-flex items-center gap-0.5" title="该 provider 未配置 API Key">
                                                                    <Icon name="alert-triangle" size={9} /> 未配置 Key
                                                                </span>
                                                            </Show>
                                                        </div>
                                                    </div>
                                                </div>
                                            );
                                        }}
                                    </For>
                                    <Show when={cloudModels().length === 0}>
                                        <div class="p-5 text-center text-[13px]" style="color: rgba(255,255,255,0.2);">
                                            <div>无线上模型</div>
                                            <div class="text-[10px] mt-1.5 leading-relaxed" style="color: rgba(255,255,255,0.25);">
                                                去 <span style="color: rgba(124,154,191,0.5); font-medium;">设置中心 → 供应商设置</span><br />
                                                启用 provider 并填写 API Key
                                            </div>
                                        </div>
                                    </Show>
                                </div>
                            </div>
                            <div style="width: 1px; background: rgba(255,255,255,0.04); align-self: stretch;"></div>
                            {/* 本地模型 */}
                            <div class="flex-1 flex flex-col min-w-0">
                                <div class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest"
                                    style="color: rgba(255,255,255,0.35); background: rgba(255,255,255,0.04); border-bottom: 1px solid rgba(255,255,255,0.04);">
                                    本地模型
                                </div>
                                <div class="flex-1 overflow-y-auto p-1.5 scrollbar-thin">
                                    <For each={localModels()}>
                                        {(model) => {
                                            const selected = () => isSelected(model);
                                            return (
                                                <div
                                                    class="flex flex-row items-center gap-2.5 p-2 text-sm rounded-lg cursor-pointer select-none transition-all"
                                                    style="color: rgba(255,255,255,0.5);"
                                                    classList={{
                                                        '!bg-[rgba(124,154,191,0.12)] !border-l-[3px] !border-[rgba(124,154,191,0.2)]': selected()
                                                    }}
                                                    onClick={() => handlePickModel(model)}
                                                    onMouseEnter={(e) => { if (!selected()) { e.currentTarget.style.background = 'rgba(255,255,255,0.06)'; e.currentTarget.style.color = 'white'; } }}
                                                    onMouseLeave={(e) => { if (!selected()) { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = 'rgba(255,255,255,0.5)'; } }}>
                                                    <div class="w-7 h-7 bg-white rounded-full flex items-center justify-center shrink-0 shadow-sm">
                                                        <img src={getModelLogo(model.model_id)} alt="logo" class="w-[18px] h-[18px] object-contain" />
                                                    </div>
                                                    <div class="flex-1 flex flex-col items-start justify-center overflow-hidden text-left min-w-0">
                                                        <div class="max-w-[180px] text-[13px] text-white font-medium truncate">{model.model_id}</div>
                                                        <div style="color: rgba(124,154,191,0.5); font-size: 10px;">{model.owned_by}</div>
                                                    </div>
                                                </div>
                                            );
                                        }}
                                    </For>
                                    <Show when={localModels().length === 0}>
                                        <div class="p-5 text-center text-[13px]" style="color: rgba(255,255,255,0.2);">无本地模型</div>
                                    </Show>
                                </div>
                            </div>
                        </div>
                    </div>

                    {/* MCP 服务器 */}
                    <div class="flex flex-col gap-1.5">
                        <label class="section-label">
                            MCP 服务器
                            <span class="ml-2 text-[11px] font-normal" style="color: rgba(255,255,255,0.4);">
                                仅对当前助手生效
                            </span>
                        </label>
                        <div class="flex flex-col gap-1.5 max-h-[180px] overflow-y-auto rounded-lg border border-dark-100 p-1.5">
                            <For each={sortedMcpServers()}>
                                {(server) => {
                                    const checked = () => (asst()?.mcpServerIds ?? []).includes(server.id);
                                    const status = () => mcpServerStatus()[server.id]?.status ?? 'disconnected';
                                    return (
                                        <label
                                            class="flex items-center gap-3 rounded-md px-2.5 py-2 cursor-pointer transition-colors hover:bg-white/5"
                                        >
                                            <input
                                                type="checkbox"
                                                checked={checked()}
                                                onChange={(e) => void handleToggleMcpServer(server.id, e.currentTarget.checked)}
                                            />
                                            <div class="flex-1 min-w-0">
                                                <div class="text-sm text-white truncate">{server.displayName || server.id}</div>
                                                <div class="text-[11px] truncate" style="color: rgba(255,255,255,0.4);">
                                                    {transportLabel(server.transport)}
                                                </div>
                                            </div>
                                            <span
                                                class="px-1.5 py-0.5 rounded text-[10px] shrink-0"
                                                style={`background: ${statusColor(status())}22; color: ${statusColor(status())};`}
                                            >
                                                {statusLabel(status())}
                                            </span>
                                        </label>
                                    );
                                }}
                            </For>
                            <Show when={sortedMcpServers().length === 0}>
                                <div class="px-3 py-5 text-center text-xs" style="color: rgba(255,255,255,0.35);">
                                    尚未配置 MCP 服务器，请先前往设置中心添加。
                                </div>
                            </Show>
                        </div>
                        <div class="text-[11px]" style="color: rgba(255,255,255,0.35);">
                            勾选决定该助手可使用哪些服务器；连接状态仍在 MCP 服务器管理页统一控制。
                        </div>
                    </div>

                    {/* Skill */}
                    <div class="flex flex-col gap-1.5">
                        <label class="section-label">
                            Skill
                            <span class="ml-2 text-[11px] font-normal" style="color: rgba(255,255,255,0.4);">
                                仅对当前助手生效
                            </span>
                        </label>
                        <div class="flex flex-col gap-1.5 max-h-[180px] overflow-y-auto rounded-lg border border-dark-100 p-1.5">
                            <For each={sortedSkills()}>
                                {(skill) => (
                                    <label class="flex items-start gap-3 rounded-md px-2.5 py-2 cursor-pointer transition-colors hover:bg-white/5">
                                        <input
                                            type="checkbox"
                                            class="mt-1"
                                            checked={(asst()?.skillIds ?? []).includes(skill.id)}
                                            onChange={(e) => void handleToggleSkill(skill.id, e.currentTarget.checked)}
                                        />
                                        <div class="flex-1 min-w-0">
                                            <div class="text-sm text-white truncate">{skill.name}</div>
                                            <div class="text-[11px] line-clamp-2" style="color: rgba(255,255,255,0.4);">
                                                {skill.description || skill.content}
                                            </div>
                                        </div>
                                    </label>
                                )}
                            </For>
                            <Show when={sortedSkills().length === 0}>
                                <div class="px-3 py-5 text-center text-xs" style="color: rgba(255,255,255,0.35);">
                                    尚未配置 Skill，请先前往设置中心添加。
                                </div>
                            </Show>
                        </div>
                        <div class="text-[11px]" style="color: rgba(255,255,255,0.35);">
                            已启用 Skill 会作为额外系统指令注入当前助手的每次对话。
                        </div>
                    </div>

                    {/* 系统提示词 */}
                    <div class="flex flex-col gap-1.5">
                        <label class="section-label">系统提示词</label>
                        <textarea
                            rows={6}
                            value={promptText()}
                            onInput={(e) => setPromptText(e.currentTarget.value)}
                            placeholder="例如：你是一个乐于助人的 AI 助手。"
                            class="w-full p-2.5 bg-dark-300 border border-dark-100 rounded-lg text-[#e0e0e0] text-base font-mono resize-y box-border focus:outline-none focus:border-pri-50"
                            style="font-family: 'JetBrains Mono', Consolas, Monaco, 'Courier New', monospace !important;"
                        />
                    </div>

                    <div class="flex justify-end gap-3">
                        <button onClick={handleClose} class="px-5 py-2.5 border-0 cursor-pointer font-bold bg-dark-100 text-[#e0e0e0] rounded-lg transition-all duration-200 hover:bg-dark-50">
                            关闭
                        </button>
                        <button onClick={handleSavePrompt} class="px-5 py-2.5 border-0 cursor-pointer font-bold bg-pri text-black rounded-lg hover:scale-105 transition-all duration-200">
                            保存
                        </button>
                    </div>
                </div>
            </div>
        </Show>
    );
};

export default AssistantSettingsModal;
