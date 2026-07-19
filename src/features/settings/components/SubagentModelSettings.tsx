/**
 * SubagentModelSettings — Per-profile model override configuration.
 *
 * Each subagent profile (9 built-in + custom) can optionally use a
 * different model/provider than the parent agent.  Only cloud models are
 * selectable because subagents run in the same process; a local model can
 * be used by leaving the override unset (falls back to parent model).
 */
import { Component, createMemo, createSignal, For, Show } from 'solid-js';
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
    setCustomSubagentProfiles,
    saveCustomSubagentProfile,
    deleteCustomSubagentProfile,
    type CustomSubagentProfile,
} from '../../../core/store/store';
import { getLogo as getLogoByIds } from '../../../core/utils/modelLogo';
import { findModel, formatContextWindow } from '../../../core/utils/models';
import Icon from '../../../shared/components/Icon';

// --- Profile descriptions (mirrors subagent::builtin_profiles) ---

interface ProfileInfo {
    id: string;
    name: string;
    description: string;
}

const BUILTIN_PROFILE_IDS = new Set([
    'explorer', 'coder', 'general', 'architect', 'debugger', 'reviewer', 'writer', 'tester', 'requirements',
]);

const BUILTIN_PROFILES: ProfileInfo[] = [
    {
        id: 'explorer',
        name: '代码探索者',
        description: '只读搜索和分析，用于大规模代码探索、多文件检索、架构分析。不能修改任何文件。',
    },
    {
        id: 'coder',
        name: '代码实现者',
        description: '代码编写和修改，用于实现具体功能模块。禁止执行 shell 命令。',
    },
    {
        id: 'general',
        name: '通用子智能体',
        description: '全能力子智能体，拥有和主 Agent 完全相同的工具集。',
    },
    {
        id: 'architect',
        name: '架构设计师',
        description: '只读架构分析、依赖映射、设计决策评估与技术选型建议。不能修改任何文件。',
    },
    {
        id: 'debugger',
        name: '问题诊断师',
        description: '错误调查与根因分析，可运行命令复现问题但不能修改任何文件。',
    },
    {
        id: 'reviewer',
        name: '代码审查员',
        description: '只读代码质量评估、安全审计与最佳实践检查。不能修改任何文件。',
    },
    {
        id: 'writer',
        name: '文档撰写员',
        description: '编写文档、注释、README、变更日志与技术规范。可读写文件。',
    },
    {
        id: 'tester',
        name: '测试工程师',
        description: '测试用例生成、覆盖率分析与测试执行。可运行测试并编写测试文件。',
    },

    {
        id: 'requirements',
        name: '需求分析员',
        description: '用户需求分析，将用户输入拆解为结构化工作流方案。不能修改文件。',
    },
];

/** Auto-generate a profile id from a display name */
const slugify = (name: string): string =>
    name
        .toLowerCase()
        .replace(/[^a-z0-9\u4e00-\u9fff\-]/g, '-')
        .replace(/-+/g, '-')
        .replace(/^-|-$/g, '')
    || 'custom';

const SubagentModelSettings: Component = () => {
    /** Cloud models only (exclude local). */
    const cloudModels = createMemo(() => allAvailableModels().filter(m => !isLocalModel(m)));

    /** Which profile has its dropdown open (null = none). */
    const [openDropdown, setOpenDropdown] = createSignal<string | null>(null);

    /** Create custom profile form state */
    const [showCreateForm, setShowCreateForm] = createSignal(false);
    const [newId, setNewId] = createSignal('');
    const [newName, setNewName] = createSignal('');
    const [newDescription, setNewDescription] = createSignal('');
    const [newAllowedTools, setNewAllowedTools] = createSignal('');
    const [newDeniedTools, setNewDeniedTools] = createSignal('delegate_task');
    const [newSystemPrompt, setNewSystemPrompt] = createSignal('');

    /** Combined profiles: built-in first, then custom */
    const allProfiles = createMemo<ProfileInfo[]>(() => [
        ...BUILTIN_PROFILES,
        ...customSubagentProfiles().map(c => ({ id: c.id, name: c.name, description: c.description })),
    ]);

    const isBuiltin = (id: string) => BUILTIN_PROFILE_IDS.has(id);

    const getProviderIdFor = (model: ActivatedModel): string => {
        if ((model as any).provider_id) return (model as any).provider_id;
        const url = (model.api_url || '').toLowerCase();
        if (url.includes('api.openai.com')) return 'openai';
        if (url.includes('api.anthropic.com')) return 'anthropic';
        if (url.includes('generativelanguage')) return 'google';
        if (url.includes('api.deepseek')) return 'deepseek';
        if (url.includes('api.groq')) return 'groq';
        if (url.includes('api.mistral')) return 'mistral';
        if (url.includes('api.x.ai')) return 'xai';
        if (url.includes('api.cohere')) return 'cohere';
        if (url.includes('openrouter')) return 'openrouter';
        return 'openai';
    };

    const getMeta = (model: ActivatedModel) => {
        const cat = modelsCatalog();
        if (!cat) return null;
        return findModel(cat, getProviderIdFor(model), model.model_id);
    };

    const getModelLogo = (modelName: string) => getLogoByIds(null, modelName);

    /** Current override model for a profile (null = no override). */
    const overrideModel = (profileId: string): ActivatedModel | null => {
        const key = profileModelOverrides()[profileId];
        if (!key) return null;
        return cloudModels().find(m => modelKey(m) === key) ?? null;
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
        if (BUILTIN_PROFILE_IDS.has(id) || customSubagentProfiles().some(c => c.id === id)) {
            alert(`角色 ID "${id}" 已存在，请使用其他 ID`);
            return;
        }
        const profile: CustomSubagentProfile = {
            id,
            name,
            description: newDescription().trim(),
            allowedTools: newAllowedTools().trim() ? newAllowedTools().trim().split(',').map(s => s.trim()).filter(Boolean) : [],
            deniedTools: newDeniedTools().trim() ? newDeniedTools().trim().split(',').map(s => s.trim()).filter(Boolean) : [],
            systemPromptExtension: newSystemPrompt().trim() || `你是${name}，请完成任务。`,
        };
        await saveCustomSubagentProfile(profile);
        // Reset form
        setShowCreateForm(false);
        setNewId('');
        setNewName('');
        setNewDescription('');
        setNewAllowedTools('');
        setNewDeniedTools('delegate_task');
        setNewSystemPrompt('');
    };

    return (
        <div class="space-y-5">
            <div class="pb-3 border-b border-[rgba(255,255,255,0.06)]">
                <h2 class="text-lg text-white font-semibold m-0">子智能体模型</h2>
                <p class="text-xs text-white/35 mt-1.5 leading-relaxed">
                    为每个子智能体类型独立指定使用的模型和 API Provider。
                    未设置的子智能体将跟随主 Agent 使用同一模型。
                    你也可以创建自定义子智能体角色。
                </p>
            </div>

            <For each={allProfiles()}>
                {(profile) => {
                    const current = () => overrideModel(profile.id);
                    const isOpen = () => openDropdown() === profile.id;
                    const builtin = isBuiltin(profile.id);

                    return (
                        <div class="bg-[rgba(255,255,255,0.035)] rounded-xl border border-[rgba(255,255,255,0.06)] overflow-hidden">
                            {/* Profile header */}
                            <div class="p-4 flex items-center justify-between">
                                <div class="flex-1 min-w-0" >
                                    <div class="flex items-center gap-2">
                                        <Show when={builtin} fallback={
                                            <Icon name="user" size={14} class="text-white/40" />
                                        }>
                                            <Icon name="sparkles" size={14} class="text-pri" />
                                        </Show>
                                        <span class="text-sm font-semibold text-white">{profile.name}</span>
                                        <span class="text-[10px] px-1.5 py-0.5 rounded bg-pri-20 text-pri font-mono">{profile.id}</span>
                                    </div>
                                    <p class="text-xs text-white/40 mt-1">{profile.description}</p>
                                </div>
                                <div class="flex items-center gap-2">
                                    <Show when={!builtin}>
                                        <button
                                            class="p-1.5 rounded-lg text-xs cursor-pointer transition-all text-white/25 hover:text-red-400 hover:bg-red-400/10"
                                            title="删除自定义角色"
                                            onClick={() => handleDeleteCustom(profile.id)}
                                        >
                                            <Icon name="x" size={14} />
                                        </button>
                                    </Show>
                                    <button
                                        class="flex-shrink-0 px-3 py-1.5 rounded-lg text-xs font-medium cursor-pointer transition-all border select-none"
                                        classList={{
                                            'bg-pri-20 border-pri-30 text-pri': !!current(),
                                            'bg-[rgba(255,255,255,0.04)] border-[rgba(255,255,255,0.08)] text-white/40 hover:text-white hover:bg-[rgba(255,255,255,0.06)]': !current(),
                                        }}
                                        onClick={() => setOpenDropdown(isOpen() ? null : profile.id)}
                                    >
                                        <Show when={current()} fallback="使用主模型">
                                            <span class="flex items-center gap-1.5">
                                                <img src={getModelLogo(current()!.model_id)} alt="" class="w-3.5 h-3.5 rounded-full object-contain" />
                                                {current()!.model_id}
                                            </span>
                                        </Show>
                                    </button>
                                </div>
                            </div>

                            {/* Dropdown model picker */}
                            <Show when={isOpen()}>
                                <div class="border-t border-[rgba(255,255,255,0.04)] bg-[rgba(0,0,0,0.15)]">
                                    <div class="max-h-[220px] overflow-y-auto p-2 scrollbar-thin">
                                        {/* "Use parent model" option */}
                                        <div
                                            class="flex items-center gap-2.5 p-2.5 rounded-lg cursor-pointer select-none transition-all text-sm"
                                            classList={{
                                                'bg-[rgba(124,154,191,0.1)] text-pri': !current(),
                                                'text-white/40 hover:text-white': !!current(),
                                            }}
                                            onClick={() => handlePickModel(profile.id, null)}
                                        >
                                            <Icon name="refresh" size={16} class="text-white/30" />
                                            <span>使用主模型（不覆盖）</span>
                                        </div>

                                        <div class="my-1.5 mx-2 border-t border-[rgba(255,255,255,0.04)]" />

                                        <For each={cloudModels()}>
                                            {(model) => {
                                                const meta = () => getMeta(model);
                                                const selected = () => isModelSelectedForProfile(profile.id, model);
                                                const noKey = () => !model.api_key;
                                                return (
                                                    <div
                                                        class="flex items-center gap-2.5 p-2.5 text-sm rounded-lg cursor-pointer select-none transition-all"
                                                        classList={{
                                                            '!bg-[rgba(124,154,191,0.12)] !border-l-[3px] !border-[rgba(124,154,191,0.2)]': selected(),
                                                        }}
                                                        onClick={() => handlePickModel(profile.id, model)}
                                                        style={{ color: 'rgba(255,255,255,0.5)' }}
                                                        onMouseEnter={(e) => {
                                                            if (!selected()) {
                                                                e.currentTarget.style.background = 'rgba(255,255,255,0.06)';
                                                                e.currentTarget.style.color = 'white';
                                                            }
                                                        }}
                                                        onMouseLeave={(e) => {
                                                            if (!selected()) {
                                                                e.currentTarget.style.background = 'transparent';
                                                                e.currentTarget.style.color = 'rgba(255,255,255,0.5)';
                                                            }
                                                        }}
                                                    >
                                                        <div class="w-6 h-6 bg-white rounded-full flex items-center justify-center shrink-0 shadow-sm">
                                                            <img src={getModelLogo(model.model_id)} alt="" class="w-4 h-4 object-contain" />
                                                        </div>
                                                        <div class="flex-1 flex flex-col items-start justify-center overflow-hidden min-w-0">
                                                            <div class="max-w-[180px] text-[13px] text-white font-medium truncate">{model.model_id}</div>
                                                            <div class="flex gap-1 mt-0.5 flex-wrap">
                                                                <Show when={meta()}>
                                                                    <span class="text-[9px] px-1 py-0.5 rounded bg-pri-20 text-pri">{formatContextWindow(meta()!.contextWindow)}</span>
                                                                </Show>
                                                                <Show when={noKey()}>
                                                                    <span class="text-[9px] px-1 py-0.5 rounded bg-yellow-500/20 text-yellow-300 inline-flex items-center gap-0.5">
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
                                            <div class="p-4 text-center text-[13px]" style={{ color: 'rgba(255,255,255,0.2)' }}>
                                                <div>无云端模型</div>
                                                <div class="text-[10px] mt-1.5 leading-relaxed" style={{ color: 'rgba(255,255,255,0.25)' }}>
                                                    去 <span style={{ color: 'rgba(124,154,191,0.5)', 'font-weight': 500 }}>设置中心 → 供应商设置</span><br />
                                                    启用 provider 并填写 API Key
                                                </div>
                                            </div>
                                        </Show>
                                    </div>
                                </div>
                            </Show>
                        </div>
                    );
                }}
            </For>

            {/* Create custom profile section */}
            <div class="pt-2">
                <Show
                    when={showCreateForm()}
                    fallback={
                        <button
                            class="w-full py-3 rounded-xl border border-dashed border-[rgba(255,255,255,0.08)] text-xs text-white/30 hover:text-white/60 hover:border-[rgba(255,255,255,0.15)] transition-all cursor-pointer bg-transparent"
                            onClick={() => setShowCreateForm(true)}
                        >
                            + 创建自定义角色
                        </button>
                    }
                >
                    <div class="bg-[rgba(18,22,35,0.35)] rounded-xl border border-[rgba(255,255,255,0.06)] overflow-hidden p-4 space-y-3">
                        <h3 class="text-sm text-white font-semibold m-0">创建自定义子智能体角色</h3>

                        <div class="grid grid-cols-2 gap-3">
                            <div>
                                <label class="block text-[11px] text-white/40 mb-1">角色 ID</label>
                                <input
                                    type="text"
                                    value={newId()}
                                    onInput={(e) => setNewId(e.currentTarget.value)}
                                    placeholder={slugify(newName())}
                                    class="w-full px-2.5 py-1.5 rounded-lg bg-[rgba(0,0,0,0.2)] border border-[rgba(255,255,255,0.06)] text-xs text-white placeholder:text-white/15 outline-none focus:border-pri-30 transition-colors"
                                />
                            </div>
                            <div>
                                <label class="block text-[11px] text-white/40 mb-1">显示名称</label>
                                <input
                                    type="text"
                                    value={newName()}
                                    onInput={(e) => setNewName(e.currentTarget.value)}
                                    placeholder="我的助手"
                                    class="w-full px-2.5 py-1.5 rounded-lg bg-[rgba(0,0,0,0.2)] border border-[rgba(255,255,255,0.06)] text-xs text-white placeholder:text-white/15 outline-none focus:border-pri-30 transition-colors"
                                />
                            </div>
                        </div>

                        <div>
                            <label class="block text-[11px] text-white/40 mb-1">用途说明</label>
                            <input
                                type="text"
                                value={newDescription()}
                                onInput={(e) => setNewDescription(e.currentTarget.value)}
                                placeholder="简短描述角色的用途"
                                class="w-full px-2.5 py-1.5 rounded-lg bg-[rgba(0,0,0,0.2)] border border-[rgba(255,255,255,0.06)] text-xs text-white placeholder:text-white/15 outline-none focus:border-pri-30 transition-colors"
                            />
                        </div>

                        <div class="grid grid-cols-2 gap-3">
                            <div>
                                <label class="block text-[11px] text-white/40 mb-1">允许的工具（逗号分隔，空=全部）</label>
                                <input
                                    type="text"
                                    value={newAllowedTools()}
                                    onInput={(e) => setNewAllowedTools(e.currentTarget.value)}
                                    placeholder="read_file, search_files"
                                    class="w-full px-2.5 py-1.5 rounded-lg bg-[rgba(0,0,0,0.2)] border border-[rgba(255,255,255,0.06)] text-xs text-white placeholder:text-white/15 outline-none focus:border-pri-30 transition-colors"
                                />
                            </div>
                            <div>
                                <label class="block text-[11px] text-white/40 mb-1">禁止的工具（逗号分隔）</label>
                                <input
                                    type="text"
                                    value={newDeniedTools()}
                                    onInput={(e) => setNewDeniedTools(e.currentTarget.value)}
                                    placeholder="delegate_task"
                                    class="w-full px-2.5 py-1.5 rounded-lg bg-[rgba(0,0,0,0.2)] border border-[rgba(255,255,255,0.06)] text-xs text-white placeholder:text-white/15 outline-none focus:border-pri-30 transition-colors"
                                />
                            </div>
                        </div>

                        <div>
                            <label class="block text-[11px] text-white/40 mb-1">系统提示词后缀</label>
                            <textarea
                                value={newSystemPrompt()}
                                onInput={(e) => setNewSystemPrompt(e.currentTarget.value)}
                                placeholder={`你是${newName() || '助手'}，请完成任务。`}
                                rows={3}
                                class="w-full px-2.5 py-1.5 rounded-lg bg-[rgba(0,0,0,0.2)] border border-[rgba(255,255,255,0.06)] text-xs text-white placeholder:text-white/15 outline-none focus:border-pri-30 transition-colors resize-none"
                            />
                        </div>

                        <div class="flex gap-2 justify-end pt-1">
                            <button
                                class="px-3 py-1.5 rounded-lg text-xs cursor-pointer transition-all bg-[rgba(255,255,255,0.04)] border border-[rgba(255,255,255,0.08)] text-white/40 hover:text-white hover:bg-[rgba(255,255,255,0.06)]"
                                onClick={() => { setShowCreateForm(false); setNewName(''); }}
                            >
                                取消
                            </button>
                            <button
                                class="px-3 py-1.5 rounded-lg text-xs font-medium cursor-pointer transition-all bg-pri-20 border border-pri-30 text-pri hover:bg-pri-30"
                                onClick={handleCreate}
                            >
                                创建
                            </button>
                        </div>
                    </div>
                </Show>
            </div>
        </div>
    );
};

export default SubagentModelSettings;
