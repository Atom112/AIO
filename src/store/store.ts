// src/store.ts  
import { createStore } from "solid-js/store";
import { createSignal } from "solid-js";
import { invoke } from '@tauri-apps/api/core';
import { readFile } from '@tauri-apps/plugin-fs';
// =============================================================================
// I. 接口定义 (Interfaces)
// =============================================================================

/**
 * 消息项接口
 */
export interface Message {
    /** 角色：'user' (用户) 或 'assistant' (助手) */
    role: 'user' | 'assistant';
    /** 消息文本内容 */
    content: string;
}

/**
 * 话题接口定义
 * 每个助手可以包含多个不同的对话话题
 */
export interface Topic {
    /** 话题唯一标识符 */
    id: string;
    /** 话题显示的名称 */
    name: string;
    /** 当前话题的历史对话记录 */
    history: Message[];
}

/**
 * 助手接口定义
 */
export interface Assistant {
    /** 助手唯一标识符 */
    id: string;
    /** 助手显示的名称 */
    name: string;
    /** 系统提示词（System Prompt），定义助手的人设和行为 */
    prompt: string;
    /** 该助手下属的话题列表 */
    topics: Topic[];
}

/**
 * 应用基础配置接口定义
 */
export interface AppConfig {
    /** 默认的 API 供应商网址 */
    apiUrl: string;
    /** 默认的 API 密钥 */
    apiKey: string;
}

/**
 * 已启动/激活的模型配置接口
 */
export interface ActivatedModel {
    /** 模型所在的 API 终端地址 */
    api_url: string;
    /** 访问该模型所需的 API Key */
    api_key: string;
    /** 模型 ID（如 'gpt-4o', 'llama3' 等） */
    model_id: string;
    /** 供应商或厂商名称 */
    owned_by: string;
    /** 若为本地模型，存储其在磁盘上的绝对路径 */
    local_path?: string;
}

export interface User {
    id: string;
    username: string;
    nickname?: string;
    token: string;
}

// =============================================================================
// II. 全局响应式状态 (Global State)
// =============================================================================

export const [globalUserAvatar, setGlobalUserAvatar] = createSignal('/icons/user.svg');

/**
 * 智能加载头像：支持 Base64 或 物理路径
 * @param input 可能是 Base64 字符串，也可能是文件路径
 */
let lastBlobUrl: string | null = null;

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
        
        // --- 优化：释放之前的内存引用 ---
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
 * 全局核心数据 Store
 * 包含：
 * 1. assistants: 加载的所有助手列表
 * 2. activatedModels: 全局同步的激活模型列表（用于模型切换下拉浮窗）
 */
export const [datas, setDatas] = createStore({
    assistants: [] as any[],
    activatedModels: [] as ActivatedModel[],
    user: null as User | null,
    isLoggedIn: false
});

/**
 * 全局当前选中的模型 Signal
 * 供 Chat 页面直接获取当前对话应使用的 AI 配置
 */
export const [selectedModel, setSelectedModel] = createSignal<ActivatedModel | null>(null);

/**
 * 当前选中的助手 ID Signal
 * 用于切换侧边栏选中的助手
 */
export const [currentAssistantId, setCurrentAssistantId] = createSignal<string | null>(null);

/**
 * 当前选中的话题 ID Signal
 * 用于在 Chat 页面跟踪和展示当前具体的对话话题
 */
export const [currentTopicId, setCurrentTopicId] = createSignal<string | null>(null);

// =============================================================================
// III. 配置初始化与管理 (Configuration)
// =============================================================================

/**
 * 从本地存储加载初始配置
 * 默认为空的 URL 和 Key，默认模型设为 'gpt-4o'
 */
const initialConfig: AppConfig = JSON.parse(
    localStorage.getItem('app_config') || '{"apiUrl":"","apiKey":"","defaultModel":"gpt-4o"}'
);

/**
 * 应用基础配置 Signal
 * 存储 API URL 和 Key 等全局设置
 */
export const [config, setConfig] = createSignal<AppConfig>(initialConfig);

/**
 * 保存配置到本地存储并更新当前状态
 * @param newConfig 新的配置对象
 */
export const saveConfig = (newConfig: AppConfig) => {
    setConfig(newConfig);
    localStorage.setItem('app_config', JSON.stringify(newConfig));
};

// =============================================================================
// IV. 后端持久化逻辑 (Backend Persistence)
// =============================================================================

/**
 * 助手数据保存助手函数
 * 将特定的助手对象推送到后端 Rust 层进行文件持久化
 * @param id 助手的唯一 ID
 */
export const saveSingleAssistantToBackend = async (id: string) => {
    const asst = datas.assistants.find(a => a.id === id);
    if (!asst) return;

    try {
        // 深度复制助手对象，防止响应式代理对象在通过 Tauri 传递时出现序列化问题
        await invoke('save_assistant', {
            assistant: JSON.parse(JSON.stringify(asst))
        });
    } catch (err) {
        console.error('保存助手失败:', err);
    }
};

/**
 * 助手物理删除函数
 * 触发后端删除对应的 JSON 配置文件
 * @param id 助手的唯一 ID
 */
export const deleteAssistantFile = async (id: string) => {
    try {
        await invoke('delete_assistant', { id });
    } catch (err) {
        console.error('物理删除失败:', err);
    }
};

// 新增：清除登录状态的方法（供切换账号或退出使用）
export const clearUserStatus = () => {
    setDatas('user', null);
    setDatas('isLoggedIn', false);
    localStorage.removeItem('auth-token');
};

export const logout = () => {
    setDatas('user', null);
    setDatas('isLoggedIn', false);
    localStorage.removeItem('auth-token');
    // 可选：重置头像到默认
    setGlobalUserAvatar('/icons/user.svg');
};