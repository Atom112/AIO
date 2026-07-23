import { createStore,reconcile } from "solid-js/store";
import { createEffect, createSignal } from "solid-js";
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { readFile } from '@tauri-apps/plugin-fs';
import type { Catalog, CatalogSourceTag, ProviderConfig } from '../utils/models';
import type { McpServerConfig, McpServerStatusInfo, ToolSpec, LlmToolCallPayload } from '../types/mcp';
import type { SkillConfig } from '../types/skill';

// 接口定义
    /** Agent 工作过程时间线步骤 */
    export interface AgentStep {
        id: string;
        type: 'thinking' | 'tool_call' | 'content' | 'subagent';
        timestamp: number;
        status: 'running' | 'complete' | 'error';
        /** 步骤耗时（ms），步骤关闭时计算 */
        duration?: number;
        /** 思考步骤文本（仅 type=thinking） */
        thinkingText?: string;
        /** 工具调用详情（仅 type=tool_call） */
        toolCall?: ToolCallDisplay;
        /** 阶段性总结 / 回复文本（仅 type=content） */
        contentText?: string;
        // ===== 子智能体字段（仅 type=subagent） =====
        /** 子智能体唯一 ID */
        subagentId?: string;
        /** 子智能体类型标识（explorer | coder | general） */
        subagentProfile?: string;
        /** 子智能体显示名称 */
        subagentName?: string;
        /** 子智能体的任务描述摘要 */
        subagentTask?: string;
        /** 子智能体内部的步骤时间线 */
        subagentSteps?: SubagentStep[];
        /** 子智能体完成的最终结果 */
        subagentResult?: string;
    }

    /** 子智能体步骤（内部时间线，结构与 AgentStep 相同但不含嵌套子智能体） */
    export interface SubagentStep {
        id: string;
        type: 'thinking' | 'tool_call' | 'content';
        timestamp: number;
        status: 'running' | 'complete' | 'error';
        duration?: number;
        summary?: string;
        toolName?: string;
    }

	/* 消息项接口，定义聊天消息的数据结构 */
export interface Message {
    id?: string;                        // 消息唯一 ID（工具消息/assistant 消息持久化需要）
    role: 'user' | 'assistant' | 'tool' | 'system';  // 消息发送者角色：'tool' 为工具执行结果，'system' 为系统指令
    content: any;                       // 消息内容，支持文本或多模态内容（使用 any 类型以兼容不同格式）
    modelId?: string;                   // 生成回复的模型标识符，仅在 AI 助手回复时存在
    displayFiles?: AttachmentMeta[];    // 消息关联的附件元数据
    displayText?: string;               // 用于界面显示的纯文本内容（已脱敏或解析处理）
    reasoning?: string;                 // 模型原生思维链（reasoning_content），仅 assistant 消息可能携带
    toolCallId?: string;                // role="tool" 时对应触发的 tool_call id（OpenAI API 要求配对）
    name?: string;                      // role="tool" 时为被调用的函数名；role="assistant" 携带 tool_calls 时为 "assistant"
    toolCalls?: ToolCallDisplay[];      // role="assistant" 时携带模型发起的工具调用请求（含前端 UI 状态 state/result/error）
    agentStartTime?: number;             // Agent 轮次开始时间戳（ms），用于计算工作耗时
    interimContent?: string;             // 多轮 Agent 工作中，中间轮次的阶段性总结文本累积
    agentSteps?: AgentStep[];            // Agent 工作过程时间线（新），按时间顺序记录每个步骤
    inputTokens?: number;                // 本轮/本消息输入 tokens 用量（跨轮累计，用于成本统计）
    outputTokens?: number;               // 本轮/本消息输出 tokens 用量（跨轮累计，用于成本统计）
    contextTokens?: number;              // 上下文峰值 tokens（最后一轮 API 调用的 input_tokens），用于上下文窗口进度条
}

/** 前端展示用的工具调用（在 OpenAI tool_calls 基础上增加 UI 状态字段） */
export interface ToolCallDisplay {
    id: string;
    type: 'function';
    function: { name: string; arguments: string };
    /** UI 状态：calling / success / error */
    state?: 'calling' | 'success' | 'error';
    /** 工具返回的结构化结果 */
    result?: any;
    /** 错误信息（state=error 时） */
    error?: string;
    /** 文件变更详情（写/替换/删文件时由后端提供） */
    fileChanges?: FileChangeInfo[];
}

/** 单次文件修改记录 */
export interface FileChangeInfo {
    /** 文件路径（相对于项目根目录） */
    filePath: string;
    /** 操作类型: create | modify | delete */
    action: 'create' | 'modify' | 'delete';
    /** unified diff 文本 */
    diff: string;
    /** 增删行数概览 "+N -M" */
    summary: string;
}

export interface AttachmentMeta {
    id?: string;
    name: string;
    mimeType?: string;
    size?: number;
}

export interface StoredAttachment extends AttachmentMeta {
    id: string;
    mimeType: string;
    size: number;
    storagePath: string;
}

export interface PendingAttachment extends StoredAttachment {
    type: 'text' | 'image';
    previewUrl?: string;
}

 /* 话题接口，定义对话主题的数据结构 */
export interface Topic {
    id: string;             // 话题唯一标识符
    name: string;           // 话题显示名称
    history: Message[];     // 该话题下的历史消息记录数组
    summary: string;        // 话题的长期记忆摘要，对应数据库存储的 summary 字段
    /**
     * 是否已自动重命名。
     * - false：新建话题或迁移前的旧话题；首次对话结束后会被自动改名为 AI 生成的标题，并置为 true
     * - true：已自动重命名过，或用户手动改过名；后续不再触发自动重命名
     * - 缺省：兼容旧数据，等价于 false
     */
    renamed?: boolean;
}

/** Agent 执行模式 */
export type AgentMode = 'off' | 'normal' | 'auto' | 'plan' | 'workflow';

 /* 助手接口，定义 AI 助手的数据结构 */
export interface Assistant {
    id: string;             // 助手唯一标识符
    name: string;           // 助手显示名称
    prompt: string;         // 助手的系统提示词（Prompt）
    modelId?: string;       // 助手绑定的首选模型 ID；未设置时回退到全局默认模型
    mcpServerIds?: string[];// 助手启用的 MCP server id 列表；空/未设置 = 该助手不使用任何 MCP 工具（opt-in）
    skillIds?: string[];    // 助手启用的 Skill id 列表；空/未设置 = 不注入 Skill 指令
    projectId?: string;     // 助手所属的项目 ID；未设置 = 全局助手
    agentMode?: AgentMode;  // Agent 执行模式；'off' = 对话模式
    assistantType?: 'chat' | 'project'; // 助理类型：chat = 聊天模式，project = 项目助理
    topics: Topic[];        // 助手关联的话题列表
}

 /* 应用基础配置接口，存储 API 连接等全局设置 */
export interface AppConfig {
    apiUrl: string;         // API 服务提供商的基础 URL 地址
    apiKey: string;         // 用于身份验证的 API 密钥（H5：仅内存使用，不落盘）
    defaultModel?: string;  // 用户偏好的默认模型 ID
}

 /* 已激活模型配置接口，定义可用 AI 模型的连接信息 */
export interface ActivatedModel {
    api_url: string;        // 模型 API 端点地址
    api_key: string;        // 访问模型所需的 API 密钥
    model_id: string;       // 模型唯一标识符，例如 'gpt-4o', 'llama3'
    owned_by: string;       // 模型提供商或厂商名称
    local_path?: string;    // 本地模型的文件系统绝对路径，仅本地模型有效
    engine_type?: string;   // 本地推理引擎类型标识，如 "llama_cpp", "vllm"
}


 /** 项目接口 */
export interface Project {
    id: string;
    name: string;
    path: string;
    createdAt: string;
    updatedAt: string;
}

/** 当前活跃的项目 ID（null = 全局模式）。从 localStorage 持久化恢复。 */
const SAVED_PROJECT_KEY = 'aio-current-project-id';
const savedProjectId = (() => {
    try {
        return localStorage.getItem(SAVED_PROJECT_KEY);
    } catch { return null; }
})();
export const [currentProjectId, setCurrentProjectId] = createSignal<string | null>(savedProjectId);

/** 项目 ID 变更时自动持久化到 localStorage */
createEffect(() => {
    const id = currentProjectId();
    try {
        if (id) {
            localStorage.setItem(SAVED_PROJECT_KEY, id);
        } else {
            localStorage.removeItem(SAVED_PROJECT_KEY);
        }
    } catch { /* ignore */ }
});

/** 上次 Agent 模式下使用的工作目录 ID（用于从对话模式切换到 Agent 时自动恢复） */
const LAST_AGENT_PROJECT_KEY = 'aio-last-agent-project-id';
export const getLastAgentProjectId = (): string | null => {
    try {
        return localStorage.getItem(LAST_AGENT_PROJECT_KEY);
    } catch { return null; }
};
export const saveLastAgentProjectId = (id: string | null) => {
    try {
        if (id) {
            localStorage.setItem(LAST_AGENT_PROJECT_KEY, id);
        } else {
            localStorage.removeItem(LAST_AGENT_PROJECT_KEY);
        }
    } catch { /* ignore */ }
};

/** 所有项目列表 */
export const [projects, setProjects] = createSignal<Project[]>([]);

/** 当前活跃项目的完整对象（派生） */
export const currentProject = (): Project | null => {
    const id = currentProjectId();
    if (!id) return null;
    return projects().find(p => p.id === id) ?? null;
};

/** 当前项目的 Git 分支名称（null = 非 git 仓库或尚未加载） */
export const [gitBranch, setGitBranch] = createSignal<string | null>(null);

// 当前项目变更时自动获取 Git 分支
createEffect(() => {
    const project = currentProject();
    if (project?.path) {
        invoke<string | null>('get_git_branch', { projectPath: project.path })
            .then(branch => setGitBranch(branch ?? null))
            .catch(() => setGitBranch(null));
    } else {
        setGitBranch(null);
    }
});

/** 当前项目的所有 Git 本地分支列表（null = 非 git 仓库或尚未加载） */
export const [gitBranches, setGitBranches] = createSignal<string[] | null>(null);

// 当前项目变更时自动获取 Git 分支列表
createEffect(() => {
    const project = currentProject();
    if (project?.path) {
        invoke<string[]>('list_git_branches', { projectPath: project.path })
            .then(branches => setGitBranches(branches ?? null))
            .catch(() => setGitBranches(null));
    } else {
        setGitBranches(null);
    }
});

/** 切换到指定 Git 分支并更新本地状态 */
export const switchBranch = async (branchName: string): Promise<void> => {
    const project = currentProject();
    if (!project?.path) return;
    try {
        await invoke<string>('switch_git_branch', { projectPath: project.path, branchName });
        // 切换成功后，立即更新当前分支名（不需要等待下一个 effect 周期）
        setGitBranch(branchName);
    } catch (e) {
        // 切换失败时，通过控制台或 UI 反馈错误
        console.error('切换分支失败:', e);
        throw e; // 让调用方处理显示
    }
};

/** 加载项目列表，恢复后验证持久化的项目 ID 仍然有效 */
export const initProjects = async () => {
    try {
        const list = await invoke<Project[]>('list_projects');
        setProjects(list);
        // 验证持久化的 projectId 是否仍存在，不存在则回退全局模式
        const saved = currentProjectId();
        if (saved && !list.find(p => p.id === saved)) {
            setCurrentProjectId(null);
        }
    } catch (e) {
        console.warn('加载项目列表失败:', e);
    }
};

/** 判断当前是否是聊天模式（纯对话，无工具调用） */
export const isChatMode = (): boolean => {
  const asst = datas.assistants.find(a => a.id === currentAssistantId());
  return asst?.agentMode === 'off' && !asst?.projectId;
};

/** 获取或创建项目对应的助理 ID */
export const ensureProjectAssistant = async (projectId: string): Promise<string> => {
  let asst = datas.assistants.find(a => a.projectId === projectId);
  if (!asst) {
    const project = projects().find(p => p.id === projectId);
    if (!project) throw new Error('Project not found');
    asst = {
      id: `asst-${projectId}`,
      name: project.name,
      prompt: '',
      agentMode: 'normal',
      assistantType: 'project',
      projectId,
      mcpServerIds: ['__aio-filesystem__'],
      skillIds: [],
      topics: [{ id: Date.now().toString(), name: '默认话题', history: [], summary: '' }],
    };
    setDatas('assistants', prev => [...prev, asst!]);
    await saveSingleAssistantToBackend(asst.id);
  }
  return asst.id;
};

/** 键盘快捷键触发的"新建项目"弹窗信号 */
export const [showProjectCreateModal, setShowProjectCreateModal] = createSignal(false);

/** 全局用户头像状态信号，默认使用系统默认头像 */
export const [globalUserAvatar, setGlobalUserAvatar] = createSignal('/icons/app-logo/user.svg');
/** 主题颜色信号，从本地存储读取或使用默认柔雾蓝 #7c9abf */
export const [themeColor, setThemeColor] = createSignal(localStorage.getItem('theme-color') || '#7c9abf');
/** 推理强度 (off=关闭 / low=轻度 / medium=中度 / high=深度) */
export type ReasoningLevel = 'off' | 'low' | 'medium' | 'high';
const REASONING_KEY = 'chat-reasoning-level';
export const [reasoningLevel, setReasoningLevel] = createSignal<ReasoningLevel>(
    (localStorage.getItem(REASONING_KEY) as ReasoningLevel) || 'off'
);
export const persistReasoningLevel = (lvl: ReasoningLevel) => {
    setReasoningLevel(lvl);
    localStorage.setItem(REASONING_KEY, lvl);
};
/** 网页搜索开关 */
const WEB_SEARCH_KEY = 'chat-web-search';
export const [webSearchEnabled, setWebSearchEnabled] = createSignal<boolean>(
    localStorage.getItem(WEB_SEARCH_KEY) === 'true'
);
export const persistWebSearch = (enabled: boolean) => {
    setWebSearchEnabled(enabled);
    localStorage.setItem(WEB_SEARCH_KEY, String(enabled));
};
/** 当前选中的模型信号，用于获取当前对话使用的 AI 配置 */
export const [selectedModel, setSelectedModel] = createSignal<ActivatedModel | null>(null);
/** 当前选中的助手 ID 信号，用于侧边栏助手切换 */
export const [currentAssistantId, setCurrentAssistantId] = createSignal<string | null>(null);
/** 当前选中的话题 ID 信号，用于 Chat 页面跟踪当前对话 */
export const [currentTopicId, setCurrentTopicId] = createSignal<string | null>(null);

/** 模型目录信号（来自 @aio/models-data 同步数据，启动时由 Layout 加载） */
export const [modelsCatalog, setModelsCatalog] = createSignal<Catalog | null>(null);
/** 模型目录加载状态：'idle' | 'loading' | 'ready' | 'failed' */
export const [modelsCatalogStatus, setModelsCatalogStatus] = createSignal<'idle' | 'loading' | 'ready' | 'failed'>('idle');
/** 当前 catalog 来源标签 */
export const [modelsCatalogSource, setModelsCatalogSource] = createSignal<CatalogSourceTag>('empty');
/** 当前 catalog 在磁盘上的路径（仅 AppData/Bundled 来源有值） */
export const [modelsCatalogPath, setModelsCatalogPath] = createSignal<string | null>(null);
/** 当前 catalog 版本号（来自 models.dev） */
export const [modelsCatalogVersion, setModelsCatalogVersion] = createSignal<string | null>(null);
/** 当前 catalog 生成时间（ISO 字符串） */
export const [modelsCatalogGeneratedAt, setModelsCatalogGeneratedAt] = createSignal<string | null>(null);

/** Provider 配置 (lobehub 形态)，key = provider id */
export const [providerConfigs, setProviderConfigs] = createSignal<Record<string, ProviderConfig>>({});

/** 从 providerConfigs 派生的可用模型列表（启用 provider 的启用模型） */
export interface ActiveModelEntry {
    provider: string;
    providerName: string;
    modelId: string;
    apiUrl: string;
    apiKey: string;
    isCustom: boolean;
}

export const activeProviderModels = (): ActiveModelEntry[] => {
    const cfgs = providerConfigs();
    const out: ActiveModelEntry[] = [];
    for (const cfg of Object.values(cfgs)) {
        if (!cfg.enabled) continue;
        for (const mid of cfg.enabledModels) {
            out.push({
                provider: cfg.id,
                providerName: cfg.displayName,
                modelId: mid,
                apiUrl: cfg.apiUrl,
                apiKey: cfg.apiKey,
                isCustom: cfg.isCustom,
            });
        }
    }
    return out;
};

/** 判断是否为本地模型（有 local_path 或 engine_type） */
export const isLocalModel = (m: ActivatedModel): boolean => !!(m.local_path || m.engine_type);

/**
 * 生成模型在助手绑定中的唯一标识键。
 * 云端模型用 `model_id@api_url` 复合键，以区分「同名模型来自不同 provider」的情况
 * （例如 OpenAI 和 OpenRouter 都提供 gpt-4o，仅靠 model_id 无法区分，会导致高亮错乱）。
 * 本地模型没有稳定的 api_url，退化为纯 model_id。
 */
export const modelKey = (m: ActivatedModel): string => {
    if (isLocalModel(m) || !m.api_url) return m.model_id;
    return `${m.model_id}@${m.api_url}`;
};

/**
 * 当前可用的全部模型列表（云端 + 本地合并）。
 * 云端模型来自 providerConfigs，本地模型来自 datas.activatedModels。
 * 供助手设置弹窗的模型选择器与 resolveAssistantModel 解析使用。
 */
export const allAvailableModels = (): ActivatedModel[] => {
    // 云端模型：派生自 providerConfigs (lobehub 形态)
    const cloud = activeProviderModels().map(m => ({
        model_id: m.modelId,
        owned_by: m.providerName,
        api_url: m.apiUrl,
        api_key: m.apiKey,
        provider_id: m.provider,
    } as ActivatedModel & { provider_id: string }));
    // 本地模型：activatedModels 中带 local_path / engine_type 的项
    const local = datas.activatedModels.filter(m => isLocalModel(m));
    return [...cloud, ...local];
};

/**
 * 解析某助手实际应使用的模型。
 * 优先按复合键 modelKey 匹配助手绑定的 modelId；若绑定的是旧式纯 model_id（无 @），
 * 则退化为按 model_id 匹配首个命中项，保证向后兼容。
 * 未绑定或匹配不到时回退到当前全局 selectedModel，再不行则取首个可用模型。
 * 返回 null 表示当前没有任何可用模型。
 */
export const resolveAssistantModel = (asst: Assistant | undefined | null): ActivatedModel | null => {
    if (!asst) return null;
    const all = allAvailableModels();
    if (asst.modelId) {
        // 1. 优先按复合键精确匹配
        const exact = all.find(m => modelKey(m) === asst.modelId);
        if (exact) return exact;
        // 2. 兼容旧数据：绑定的 modelId 不含 @ 时，按 model_id 兜底匹配
        if (!asst.modelId.includes('@')) {
            const legacy = all.find(m => m.model_id === asst.modelId);
            if (legacy) return legacy;
        }
    }
    const cur = selectedModel();
    if (cur) return cur;
    return all[0] ?? null;
};

// ====== Per-Profile Model Overrides ======

/** 从后端加载的 profile 模型覆盖信息 */
export interface ProfileModelOverrideBackend {
    profileId: string;
    modelId: string;
    apiUrl: string;
    apiKey: string;
}

/**
 * Per-profile 模型覆盖映射。
 * Key = profile id ("explorer" | "coder" | "general")，
 * Value = 复合键 modelKey (model_id@api_url)。空字符串 = 无覆盖（使用父模型）。
 */
export const [profileModelOverrides, setProfileModelOverrides] = createSignal<Record<string, string>>({});

/** 应用启动时从后端加载 per-profile 模型覆盖配置 */
export const initProfileModelOverrides = async () => {
    try {
        const overrides = await invoke<ProfileModelOverrideBackend[]>('load_profile_model_overrides');
        const map: Record<string, string> = {};
        for (const o of overrides) {
            if (o.profileId && o.modelId) {
                map[o.profileId] = o.modelId.includes('@') ? o.modelId : `${o.modelId}@${o.apiUrl}`;
            }
        }
        setProfileModelOverrides(map);
    } catch (e) {
        console.warn('Failed to load profile model overrides:', e);
    }
};

/** 持久化当前 per-profile 模型覆盖到后端 */
export const saveProfileModelOverrides = async () => {
    const map = profileModelOverrides();
    const overrides: ProfileModelOverrideBackend[] = [];
    for (const [profileId, mKey] of Object.entries(map)) {
        if (!mKey) continue;
        const model = allAvailableModels().find(m => modelKey(m) === mKey);
        if (model) {
            overrides.push({
                profileId,
                modelId: mKey,
                apiUrl: model.api_url,
                apiKey: model.api_key,
            });
        }
    }
    await invoke('save_profile_model_overrides', { overrides });
};

/** 解析某个 profile 的覆盖模型（从 allAvailableModels 中查找）。返回 null 表示无覆盖。 */
export const resolveProfileModel = (profileId: string): ActivatedModel | null => {
    const key = profileModelOverrides()[profileId];
    if (!key) return null;
    return allAvailableModels().find(m => modelKey(m) === key) ?? null;
};

// ====== Custom Subagent Profiles ======

/** 用户自定义子智能体配置文件 */
export interface CustomSubagentProfile {
    id: string;
    name: string;
    description: string;
    allowedTools: string[];
    deniedTools: string[];
    systemPromptExtension: string;
}

export const [customSubagentProfiles, setCustomSubagentProfiles] = createSignal<CustomSubagentProfile[]>([]);

/** 应用启动时从后端加载自定义子智能体配置文件 */
export const initCustomSubagentProfiles = async () => {
    try {
        const list = await invoke<CustomSubagentProfile[]>('list_custom_subagent_profiles');
        setCustomSubagentProfiles(list);
    } catch (e) {
        console.warn('Failed to load custom subagent profiles:', e);
    }
};

/** 保存（插入或更新）一个自定义子智能体配置文件 */
export const saveCustomSubagentProfile = async (profile: CustomSubagentProfile) => {
    await invoke('save_custom_subagent_profile', { profile });
    const list = await invoke<CustomSubagentProfile[]>('list_custom_subagent_profiles');
    setCustomSubagentProfiles(list);
};

/** 按 id 删除一个自定义子智能体配置文件 */
export const deleteCustomSubagentProfile = async (profileId: string) => {
    await invoke('delete_custom_subagent_profile', { profileId });
    setCustomSubagentProfiles(prev => prev.filter(p => p.id !== profileId));
};

/**
 * 检查本地推理引擎服务是否就绪（2s 超时）
 */
const checkServerHealth = async (baseUrl: string): Promise<boolean> => {
    try {
        const controller = new AbortController();
        const timeoutId = setTimeout(() => controller.abort(), 2000);
        const rootUrl = baseUrl.replace('/v1', '');
        const resp = await fetch(`${rootUrl}/health`, { signal: controller.signal });
        clearTimeout(timeoutId);
        return resp.ok;
    } catch {
        return false;
    }
};

/**
 * 为指定助手启动本地推理引擎，并把启动进度以占位消息写入该助手首个话题。
 * 从 NavBar 的 startLocalModel 逻辑提取，供助手设置弹窗选本地模型时复用。
 * @param model - 本地模型（需带 local_path）
 * @param asstId - 目标助手 ID（loading 消息落点）
 */
export const startLocalEngineForAssistant = async (model: ActivatedModel, asstId: string): Promise<void> => {
    if (!model.local_path) return;
    // M13 防护：未确认时不静默启动
    if (!isLocalAutoStartConfirmed()) {
        const ok = confirm(
            `检测到本地模型 "${model.model_id}"\n` +
            `路径: ${model.local_path}\n\n` +
            `是否允许 AIO 在应用启动时自动拉起该本地推理引擎？\n` +
            `（点击"取消"后，可随时在设置页手动启动）`
        );
        if (!ok) return;
        setLocalAutoStartConfirmed();
    }
    const isRunning = await invoke<boolean>('is_local_server_running');
    if (isRunning) return;

    const assistant = datas.assistants.find((a: any) => a.id === asstId);
    if (!assistant) return;
    const topicId = assistant.topics?.[0]?.id;
    const loadingText = "**正在启动本地推理引擎...**";

    if (topicId) {
        setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
            'history', h => [...h, { role: 'assistant', content: loadingText }]
        );
    }

    try {
        setIsStartingLocalModel(true);
        setLocalModelStartProgress(0);
        await invoke('start_local_server', {
            modelPath: model.local_path,
            port: 8080,
            gpuLayers: 99,
            engineType: model.engine_type || 'llama_cpp',
            trustRemoteCode: model.engine_type === 'vllm'
                ? window.confirm('[!] vLLM 安全警告：是否启用 --trust-remote-code？\n\n该选项允许模型执行自定义 Python 代码。')
                : false,
        });

        let attempts = 0;
        const maxAttempts = 60;
        const poll = setInterval(async () => {
            attempts++;
            const isReady = await checkServerHealth("http://127.0.0.1:8080/v1");
            if (isReady) {
                clearInterval(poll);
                setLocalModelStartProgress(100);
                setTimeout(() => {
                    setIsStartingLocalModel(false);
                    setLocalModelStartProgress(0);
                }, 1000);
                if (topicId) {
                    setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
                        'history', h => h.map((msg: any) =>
                            msg.content === loadingText
                                ? { ...msg, content: "**本地服务器启动成功，可以开始对话了！**" }
                                : msg
                        )
                    );
                }
            } else if (attempts >= maxAttempts) {
                clearInterval(poll);
                setLocalModelStartProgress(100);
                setTimeout(() => {
                    setIsStartingLocalModel(false);
                    setLocalModelStartProgress(0);
                }, 1000);
                if (topicId) {
                    setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
                        'history', h => [...h, { role: 'assistant', content: "**服务器启动超时，请检查显存空间或模型文件。**" }]
                    );
                }
            }
        }, 500);
    } catch (err) {
        setLocalModelStartProgress(100);
        setTimeout(() => {
            setIsStartingLocalModel(false);
            setLocalModelStartProgress(0);
        }, 1000);
        if (topicId) {
            setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
                'history', h => [...h, { role: 'assistant', content: `**启动失败: ${err}**` }]
            );
        }
    }
};

/**
 * 为某助手设置绑定模型并立即同步全局 selectedModel + 持久化。
 * 若为本地模型则顺带拉起推理引擎。供助手设置弹窗复用。
 */
export const setAssistantModel = async (asstId: string, model: ActivatedModel): Promise<void> => {
    // 存复合键 modelKey（model_id@api_url 或纯 model_id），确保同名异源模型可区分
    setDatas('assistants', (a: any) => a.id === asstId, 'modelId', modelKey(model));
    setSelectedModel(model);
    await saveSingleAssistantToBackend(asstId);
    if (isLocalModel(model) && model.local_path) {
        void startLocalEngineForAssistant(model, asstId);
    }
};

/** 是否正在启动本地模型 */
export const [isStartingLocalModel, setIsStartingLocalModel] = createSignal(false);
/** 本地模型启动进度百分比 */
export const [localModelStartProgress, setLocalModelStartProgress] = createSignal(0);

// ====== 应用更新状态 ======

/** 是否有可用的应用更新 */
export const [appUpdateAvailable, setAppUpdateAvailable] = createSignal(false);

/** 最新版本元信息（版本号、Release notes、发布时间） */
export interface AppUpdateInfoData {
    version: string;
    currentVersion?: string;
    notes?: string;
    pubDate?: string;
}

/** 最新版本元信息 */
export const [appUpdateInfo, setAppUpdateInfo] = createSignal<AppUpdateInfoData | null>(null);

/** 用户在当前会话中是否已经手动关闭过更新提示（避免短暂显示后又跳出来） */
export const [appUpdateDismissed, setAppUpdateDismissed] = createSignal(false);

/** 是否正在下载/安装更新 */
export const [appUpdateDownloading, setAppUpdateDownloading] = createSignal(false);
/** 下载进度 (0.0 ~ 1.0) */
export const [appUpdateProgress, setAppUpdateProgress] = createSignal(0);
/** 更新已下载完毕，等待用户重启 */
export const [appUpdateReady, setAppUpdateReady] = createSignal(false);

const IGNORED_UPDATE_VERSION_KEY = 'aio_ignored_update_version';

/**
 * 读取用户"忽略此版本"的记录
 * @returns 被忽略的最新版本号（带 v 前缀已剥离），没有则返回 null
 */
export const getIgnoredUpdateVersion = (): string | null => {
    try {
        return localStorage.getItem(IGNORED_UPDATE_VERSION_KEY);
    } catch {
        return null;
    }
};

/**
 * 记录用户"忽略此版本"或清除记录
 * @param version - 要忽略的版本号；传 null 则清除
 */
export const setIgnoredUpdateVersion = (version: string | null): void => {
    try {
        if (version) {
            localStorage.setItem(IGNORED_UPDATE_VERSION_KEY, version);
        } else {
            localStorage.removeItem(IGNORED_UPDATE_VERSION_KEY);
        }
    } catch (e) {
        console.warn('保存更新忽略记录失败:', e);
    }
};

/** 缓存上一次生成的 Blob URL，用于内存管理 */
let lastBlobUrl: string | null = null;

/** M13: 用户是否已确认允许自动启动本地推理引擎（首次启动需用户确认） */
const AUTO_START_KEY = 'aio-local-auto-start-confirmed';
export const isLocalAutoStartConfirmed = (): boolean => {
    try {
        return localStorage.getItem(AUTO_START_KEY) === '1';
    } catch {
        return false;
    }
};
export const setLocalAutoStartConfirmed = (): void => {
    try {
        localStorage.setItem(AUTO_START_KEY, '1');
    } catch { /* ignore */ }
};
export const clearLocalAutoStartConfirmed = (): void => {
    try {
        localStorage.removeItem(AUTO_START_KEY);
    } catch { /* ignore */ }
};

/**
 * 从指定路径加载用户头像
 * @param input - 头像数据源，可以是 Base64 字符串或文件系统路径
 * @returns 头像的可访问 URL 字符串
 */
export const loadAvatarFromPath = async (input: string): Promise<string> => {
    if (!input) return '/icons/app-logo/user.svg';

    // 直接返回已可访问的 URL（Base64 / 网络地址）
    if (input.startsWith('data:image') || input.startsWith('http://') || input.startsWith('https://')) {
        return input;
    }

    try {
        const contents = await readFile(input);
        const ext = input.split('.').pop()?.toLowerCase();
        const mime = ext === 'svg' ? 'image/svg+xml' : `image/${ext === 'jpg' ? 'jpeg' : ext}`;
        const blob = new Blob([contents], { type: mime });

        // 释放之前的 Blob URL 以避免内存泄漏
        if (lastBlobUrl) {
            URL.revokeObjectURL(lastBlobUrl);
        }

        lastBlobUrl = URL.createObjectURL(blob);
        return lastBlobUrl;
    } catch (e) {
        console.error("从物理路径加载头像失败:", e, "路径:", input);
        return '/icons/app-logo/user.svg';
    }
};

/**
 * 全局核心数据存储
 * @property assistants - 所有助手的数组
 * @property activatedModels - 当前激活的模型列表，用于模型选择器
 */
export const [datas, setDatas] = createStore({
    assistants: [] as any[],
    activatedModels: [] as ActivatedModel[],
});

// ====== MCP 状态 ======

/** MCP 服务器配置表（id → config） */
export const [mcpServers, setMcpServers] = createSignal<Record<string, McpServerConfig>>({});
/** MCP 服务器运行时状态（id → status） */
export const [mcpServerStatus, setMcpServerStatus] = createSignal<Record<string, McpServerStatusInfo>>({});
/** 当前对话可用的工具列表（合并所有已连接 server 的工具，供全局状态展示） */
export const [mcpToolsCache, setMcpToolsCache] = createSignal<ToolSpec[]>([]);
/** 全局 Skill 配置表（id → config）。 */
export const [skills, setSkills] = createSignal<Record<string, SkillConfig>>({});

/** LLM 工具调用事件总线（ChatPage 监听） */
export const [pendingToolCall, setPendingToolCall] = createSignal<LlmToolCallPayload | null>(null);

/**
 * 加载并初始化 MCP 服务器列表 + 同步后端已连接状态 + 自动启动标记为 autoStart 的 server
 */
export const initMcpServers = async (projectId?: string | null) => {
    try {
        const list = await invoke<McpServerConfig[]>('list_mcp_servers', { projectId: projectId ?? null });
        const map: Record<string, McpServerConfig> = {};
        for (const cfg of list) map[cfg.id] = cfg;
        setMcpServers(map);

        // 同步后端已连接状态
        const statusMap = await invoke<Record<string, McpServerStatusInfo>>('list_mcp_server_status');
        setMcpServerStatus(statusMap);

        // 监听后台状态推送
        listen<McpServerStatusInfo>('mcp-server-status', (event) => {
            const info = event.payload;
            setMcpServerStatus(prev => ({ ...prev, [info.id]: info }));
        });

        // 监听 MCP server stderr 日志（仅 dev 模式，避免生产环境泄露密钥）
        listen<{ id: string; line: string }>('mcp-server-stderr', (event) => {
            const { id, line } = event.payload;
            if (import.meta.env.DEV) {
                console.debug(`[mcp:${id}] ${line}`);
            }
        });

        // 自动启动标记为 autoStart 的 server
        // （是否被某助手使用由 Assistant.mcpServerIds 在 list_mcp_tools_for_assistant 时过滤）
        const autoStartIds = Object.values(map)
            .filter(cfg => cfg.autoStart)
            .map(cfg => cfg.id);
        if (autoStartIds.length > 0) {
            const pid = projectId ?? null;
            await Promise.allSettled(autoStartIds.map(id => startMcpServerAndRefresh(id, pid)));
        }
    } catch (e) {
        console.warn('加载 MCP server 列表失败:', e);
    }
};

/**
 * 启动一个 MCP server 并刷新工具缓存
 */
export const startMcpServerAndRefresh = async (id: string, projectId?: string | null): Promise<ToolSpec[]> => {
    setMcpServerStatus(prev => ({
        ...prev,
        [id]: { id, status: 'connecting', toolCount: 0 },
    }));
    try {
        const tools = await invoke<ToolSpec[]>('start_mcp_server', { id, projectId: projectId ?? null });
        setMcpServerStatus(prev => ({
            ...prev,
            [id]: { id, status: 'connected', toolCount: tools.length },
        }));
        await refreshMcpToolsCache();
        return tools;
    } catch (e) {
        setMcpServerStatus(prev => ({
            ...prev,
            [id]: { id, status: 'error', message: String(e), toolCount: 0 },
        }));
        throw e;
    }
};

/** 重新拉取所有已连接 server 的工具，更新 mcpToolsCache（全局状态展示用） */
export const refreshMcpToolsCache = async () => {
    try {
        const tools = await invoke<ToolSpec[]>('list_mcp_tools', { projectId: currentProjectId() ?? null });
        setMcpToolsCache(tools);
    } catch (e) {
        console.warn('刷新 MCP 工具缓存失败:', e);
    }
};


// 配置初始化与管理
// H5 加固：API Key 改存 Rust 侧 keyring；前端 localStorage 仅保留非敏感字段
const initialConfig: AppConfig = (() => {
    try {
        const raw = localStorage.getItem('app_config');
        if (!raw) return { apiUrl: '', apiKey: '', defaultModel: '' };
        const parsed = JSON.parse(raw);
        // 兼容旧数据：保留非敏感字段，清空 apiKey（由 Rust 重新加载）
        return {
            apiUrl: parsed.apiUrl || '',
            apiKey: '',
            defaultModel: parsed.defaultModel || '',
        };
    } catch {
        return { apiUrl: '', apiKey: '', defaultModel: '' };
    }
})();

/** 应用配置信号，存储 API URL 和 Key 等全局设置 */
export const [config, setConfig] = createSignal<AppConfig>(initialConfig);

/**
 * 保存配置到本地存储并更新状态
 * H5: apiKey 不再写入 localStorage，由后端存到 keyring
 * @param newConfig - 新的应用配置对象
 */
export const saveConfig = (newConfig: AppConfig) => {
    // 内存中保留 key 以便使用，但落盘前剥离
    setConfig(newConfig);
    const persisted: { apiUrl: string; defaultModel?: string } = {
        apiUrl: newConfig.apiUrl,
    };
    if (newConfig.defaultModel) {
        persisted.defaultModel = newConfig.defaultModel;
    }
    localStorage.setItem('app_config', JSON.stringify(persisted));
};

// 后端持久化逻辑
/**
 * 将指定助手数据持久化保存到后端
 * @param id - 要保存的助手唯一标识符
 */
export const saveSingleAssistantToBackend = async (id: string) => {
    const asst = datas.assistants.find(a => a.id === id);
    if (!asst) return;

    try {
        // 深度克隆对象以解除 SolidJS 响应式代理，确保可序列化
        await invoke('save_assistant', {
            assistant: JSON.parse(JSON.stringify(asst))
        });
    } catch (err) {
        console.error('保存助手失败:', err);
    }
};

// ====== 话题标题自动重命名 ======

/**
 * 待处理的重命名请求信号。
 * ChatPage 组件通过 createEffect 监听此信号，触发实际的 LLM 调用。
 * 消费后立即清空，避免重复触发。
 */
export const [pendingRenameRequest, setPendingRenameRequest] = createSignal<{ asstId: string; topicId: string } | null>(null);

/**
 * 触发话题标题的重新生成（手动入口）。
 * 用于右键菜单的"重新生成标题"按钮。
 * 行为：
 *   1. 校验话题存在且不是默认话题（默认话题永远不重命名）
 *   2. 重置 `renamed = false`，让后续的 checkAndRename 能正常工作
 *   3. 持久化到后端
 *   4. 通过 pendingRenameRequest 信号通知 ChatPage 执行实际的 LLM 调用
 * @returns 是否成功发起请求（默认话题 / 不存在则返回 false）
 */
export const requestRenameTopic = (asstId: string, topicId: string): boolean => {
    const asst = datas.assistants.find((a: any) => a.id === asstId);
    const topic = asst?.topics.find((t: Topic) => t.id === topicId);
    if (!asst || !topic) return false;
    // 默认话题（每个助手的话题列表第一项）永远不参与自动重命名
    if (asst.topics[0]?.id === topic.id) return false;

    // 重置标志位，使 checkAndRename 能继续工作
    setDatas(
        'assistants',
        (a: any) => a.id === asstId,
        'topics',
        (t: Topic) => t.id === topicId,
        'renamed',
        false
    );
    // 异步持久化（不阻塞 UI）
    void saveSingleAssistantToBackend(asstId);

    // 通过信号通知 ChatPage
    setPendingRenameRequest({ asstId, topicId });
    return true;
};

/**
 * 从后端物理删除助手配置文件
 * @param id - 要删除的助手唯一标识符
 */
export const deleteAssistantFile = async (id: string) => {
    try {
        await invoke('delete_assistant', { id });
    } catch (err) {
        console.error('物理删除失败:', err);
    }
};


// 监听主题颜色变化并同步到 CSS 变量和本地存储
createEffect(() => {
    const color = themeColor();
    // 打标记全局禁用过渡, 确保主题色切换即时无动画
    document.documentElement.setAttribute('data-theme-changing', '');
    document.documentElement.style.setProperty('--primary-color', color);
    // 同步 RGB 分量供 rgba(var(--primary-rgb), 0.xx) 使用
    const r = parseInt(color.slice(1, 3), 16);
    const g = parseInt(color.slice(3, 5), 16);
    const b = parseInt(color.slice(5, 7), 16);
    if (!isNaN(r) && !isNaN(g) && !isNaN(b)) {
        document.documentElement.style.setProperty('--primary-rgb', `${r}, ${g}, ${b}`);
    }
    localStorage.setItem('theme-color', color);
    // 下一帧移除标记, 恢复正常的交互过渡动画
    requestAnimationFrame(() => {
        requestAnimationFrame(() => {
            document.documentElement.removeAttribute('data-theme-changing');
        });
    });
});


/**
 * 平滑更新指定话题的消息历史（解决动画闪烁/DOM全量重建问题）
 * @param assistantId - 助手 ID
 * @param topicId - 话题 ID
 * @param newHistory - 最新的完整消息历史数组
 */
export const updateTopicHistorySmoothly = (
    assistantId: string, 
    topicId: string, 
    newHistory: Message[]
) => {
    setDatas(
        'assistants', 
        (asst) => asst.id === assistantId, // 定位助手
        'topics', 
        (topic) => topic.id === topicId,   // 定位话题
        'history', 
        reconcile(newHistory)              // 核心：智能 Diff 替换
    );
};

/** 从后端加载 Skill 配置（支持项目级合并）。 */
export const initSkills = async (projectId?: string | null) => {
    try {
        const list = await invoke<SkillConfig[]>('list_skills', { projectId: projectId ?? null });
        setSkills(Object.fromEntries(list.map(skill => [skill.id, skill])));
    } catch (e) {
        console.warn('加载 Skill 列表失败:', e);
    }
};

/** 返回某个助手启用且仍存在的 Skill，顺序与助手配置一致。 */
export const resolveAssistantSkills = (assistant: Assistant | undefined | null): SkillConfig[] => {
    if (!assistant) return [];
    const skillMap = skills();
    return (assistant.skillIds ?? [])
        .map(id => skillMap[id])
        .filter((skill): skill is SkillConfig => Boolean(skill));
};


// ====== 工作流状态（Workflow Visualization） ======

/** 工作流步骤状态 */
export interface WorkflowStepState {
    stepId: string;
    profileId: string;
    name: string;
    status: 'pending' | 'running' | 'completed' | 'failed';
    startedAt?: number;
    duration?: number;
}

/** 工作流整体状态 */
export interface WorkflowState {
    workflowId: string;
    title: string;
    steps: WorkflowStepState[];
    active: boolean;
}

/** 当前活跃的工作流状态，null 表示无活跃工作流 */
export const [workflowState, setWorkflowState] = createSignal<WorkflowState | null>(null);
