// src/store.ts  
import { createStore } from "solid-js/store";
import { createEffect, createSignal } from "solid-js";
import { invoke } from '@tauri-apps/api/core';
import { readFile } from '@tauri-apps/plugin-fs';

// =============================================================================
// I. 接口定义 (Interfaces)
// =============================================================================

/**
 * 消息项接口，定义聊天消息的数据结构
 */
export interface Message {  
    /** 消息发送者角色：'user' 表示用户，'assistant' 表示 AI 助手 */  
    role: 'user' | 'assistant';  
    /** 消息内容，支持文本或多模态内容（使用 any 类型以兼容不同格式） */  
    content: any;  
    /** 生成回复的模型标识符，仅在 AI 助手回复时存在 */
    modelId?: string;
    /** 消息关联的展示文件列表 */
    displayFiles?: { name: string }[];
    /** 用于界面显示的纯文本内容（已脱敏或解析处理） */
    displayText?: string;
}  

/**  
 * 话题接口，定义对话主题的数据结构  
 */  
export interface Topic {  
    /** 话题唯一标识符 */  
    id: string;  
    /** 话题显示名称 */  
    name: string;  
    /** 该话题下的历史消息记录数组 */  
    history: Message[];  
    /** 话题的长期记忆摘要，对应数据库存储的 summary 字段 */
    summary: string; 
}  

/**
 * 助手接口，定义 AI 助手的数据结构
 * 引用 Topic 接口作为话题类型
 */
export interface Assistant {  
    /** 助手唯一标识符 */  
    id: string;  
    /** 助手显示名称 */  
    name: string;  
    /** 助手的系统提示词（Prompt） */  
    prompt: string;  
    /** 助手关联的话题列表 */  
    topics: Topic[];  
}

/**
 * 应用基础配置接口，存储 API 连接等全局设置
 */
export interface AppConfig {
    /** API 服务提供商的基础 URL 地址 */
    apiUrl: string;
    /** 用于身份验证的 API 密钥 */
    apiKey: string;
}

/**
 * 已激活模型配置接口，定义可用 AI 模型的连接信息
 */
export interface ActivatedModel {
    /** 模型 API 端点地址 */
    api_url: string;
    /** 访问模型所需的 API 密钥 */
    api_key: string;
    /** 模型唯一标识符，例如 'gpt-4o', 'llama3' */
    model_id: string;
    /** 模型提供商或厂商名称 */
    owned_by: string;
    /** 本地模型的文件系统绝对路径，仅本地模型有效 */
    local_path?: string;
}

/**
 * 用户接口，定义用户账户信息
 */
export interface User {
    /** 用户唯一标识符 */
    id: string;
    /** 用户登录用户名 */
    username: string;
    /** 用户昵称（可选） */
    nickname?: string;
    /** 用户身份验证令牌 */
    token: string;
}

// =============================================================================
// II. 全局响应式状态 (Global State)
// =============================================================================

/** 全局用户头像状态信号，默认使用系统默认头像 */
export const [globalUserAvatar, setGlobalUserAvatar] = createSignal('/icons/user.svg');

/** 缓存上一次生成的 Blob URL，用于内存管理 */
let lastBlobUrl: string | null = null;

/**
 * 从指定路径加载用户头像
 * 支持 Base64 编码字符串或本地文件路径
 * 
 * @param input - 头像数据源，可以是 Base64 字符串或文件系统路径
 * @returns 头像的可访问 URL 字符串
 */
export const loadAvatarFromPath = async (input: string): Promise<string> => {
    if (!input) return '/icons/user.svg';

    if (input.startsWith('data:image')) {
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
        return '/icons/user.svg';
    }
};

/**
 * 全局核心数据存储
 * 
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

/** 当前选中的模型信号，用于获取当前对话使用的 AI 配置 */
export const [selectedModel, setSelectedModel] = createSignal<ActivatedModel | null>(null);

/** 当前选中的助手 ID 信号，用于侧边栏助手切换 */
export const [currentAssistantId, setCurrentAssistantId] = createSignal<string | null>(null);

/** 当前选中的话题 ID 信号，用于 Chat 页面跟踪当前对话 */
export const [currentTopicId, setCurrentTopicId] = createSignal<string | null>(null);

// =============================================================================
// III. 配置初始化与管理 (Configuration)
// =============================================================================

/**
 * 从本地存储加载应用初始配置
 * 默认配置：空 API URL、空 API Key、默认模型 'gpt-4o'
 */
const initialConfig: AppConfig = JSON.parse(
    localStorage.getItem('app_config') || '{"apiUrl":"","apiKey":"","defaultModel":"gpt-4o"}'
);

/** 应用配置信号，存储 API URL 和 Key 等全局设置 */
export const [config, setConfig] = createSignal<AppConfig>(initialConfig);

/**
 * 保存配置到本地存储并更新状态
 * 
 * @param newConfig - 新的应用配置对象
 */
export const saveConfig = (newConfig: AppConfig) => {
    setConfig(newConfig);
    localStorage.setItem('app_config', JSON.stringify(newConfig));
};

// =============================================================================
// IV. 后端持久化逻辑 (Backend Persistence)
// =============================================================================

/**
 * 将指定助手数据持久化保存到后端
 * 通过 Tauri 调用 Rust 后端进行文件存储
 * 
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
 * 
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
 * 用于账号切换或退出登录场景
 */
export const clearUserStatus = () => {
    setDatas('user', null);
    setDatas('isLoggedIn', false);
    localStorage.removeItem('auth-token');
};

/**
 * 执行用户登出操作
 * 清除登录状态、移除认证令牌、重置头像
 */
export const logout = () => {
    setDatas('user', null);
    setDatas('isLoggedIn', false);
    localStorage.removeItem('auth-token');
    // 重置头像为默认
    setGlobalUserAvatar('/icons/user.svg');
};

/** 主题颜色信号，从本地存储读取或使用默认色 #08ddf9 */
export const [themeColor, setThemeColor] = createSignal(localStorage.getItem('theme-color') || '#08ddf9');

// 监听主题颜色变化并同步到 CSS 变量和本地存储
createEffect(() => {
    const color = themeColor();
    document.documentElement.style.setProperty('--primary-color', color);
    localStorage.setItem('theme-color', color);
});