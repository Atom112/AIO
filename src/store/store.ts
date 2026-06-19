import { createStore,reconcile } from "solid-js/store";
import { createEffect, createSignal } from "solid-js";
import { invoke } from '@tauri-apps/api/core';
import { readFile } from '@tauri-apps/plugin-fs';
import type { Catalog, CatalogSourceTag, ProviderConfig } from '../utils/models';

// 接口定义
 /* 消息项接口，定义聊天消息的数据结构 */
export interface Message {  
    role: 'user' | 'assistant';         // 消息发送者角色：'user' 表示用户，'assistant' 表示 AI 助手
    content: any;                       // 消息内容，支持文本或多模态内容（使用 any 类型以兼容不同格式）
    modelId?: string;                   // 生成回复的模型标识符，仅在 AI 助手回复时存在
    displayFiles?: { name: string }[];  // 消息关联的展示文件列表
    displayText?: string;               // 用于界面显示的纯文本内容（已脱敏或解析处理）
}  

 /* 话题接口，定义对话主题的数据结构 */
export interface Topic {  
    id: string;             // 话题唯一标识符
    name: string;           // 话题显示名称
    history: Message[];     // 该话题下的历史消息记录数组
    summary: string;        // 话题的长期记忆摘要，对应数据库存储的 summary 字段
}  

 /* 助手接口，定义 AI 助手的数据结构 */
export interface Assistant {  
    id: string;             // 助手唯一标识符
    name: string;           // 助手显示名称
    prompt: string;         // 助手的系统提示词（Prompt）
    topics: Topic[];        // 助手关联的话题列表
}

 /* 应用基础配置接口，存储 API 连接等全局设置 */
export interface AppConfig {
    apiUrl: string;         // API 服务提供商的基础 URL 地址
    apiKey: string;         // 用于身份验证的 API 密钥
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

 /* 用户接口，定义用户账户信息 */
export interface User {
    id: string;             // 用户唯一标识符
    username: string;       // 用户登录用户名
    nickname?: string;      // 用户昵称（可选）
    token: string;          // 用户身份验证令牌
}

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
 * @property user - 当前登录用户信息
 * @property isLoggedIn - 用户登录状态标志
 */
export const [datas, setDatas] = createStore({
    assistants: [] as any[],
    activatedModels: [] as ActivatedModel[],
    user: null as User | null,
    isLoggedIn: false
});


// 配置初始化与管理
/**
 * 从本地存储加载应用初始配置
 */
const initialConfig: AppConfig = JSON.parse(
    localStorage.getItem('app_config') || '{"apiUrl":"","apiKey":"","defaultModel":""}'
);

/** 应用配置信号，存储 API URL 和 Key 等全局设置 */
export const [config, setConfig] = createSignal<AppConfig>(initialConfig);

/**
 * 保存配置到本地存储并更新状态
 * @param newConfig - 新的应用配置对象
 */
export const saveConfig = (newConfig: AppConfig) => {
    setConfig(newConfig);
    localStorage.setItem('app_config', JSON.stringify(newConfig));
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

/**
 * 清除用户登录状态
 */
export const clearUserStatus = () => {
    setDatas('user', null);
    setDatas('isLoggedIn', false);
    localStorage.removeItem('auth-token');
};

/**
 * 执行用户登出操作
 */
export const logout = () => {
    setDatas('user', null);
    setDatas('isLoggedIn', false);
    localStorage.removeItem('auth-token');
    // 重置头像为默认
    setGlobalUserAvatar('/icons/app-logo/user.svg');
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