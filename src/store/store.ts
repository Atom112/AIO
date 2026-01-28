// src/store.ts
import { createStore } from "solid-js/store";
import { createSignal } from "solid-js";
import { invoke } from '@tauri-apps/api/core';

export interface Message {
    role: 'user' | 'assistant';
    content: string;
}

export interface Topic {
    id: string;
    name: string;
    history: Message[];
}

export interface Assistant {
    id: string;
    name: string;
    prompt: string;
    topics: Topic[]; // 核心：增加话题列表
}

export interface AppConfig {
    apiUrl: string;
    apiKey: string;
}

export interface ActivatedModel {
    api_url: string;
    api_key: string;
    model_id: string;
    owned_by: string;
    local_path?: string;
}

// 在 datas 中增加 activatedModels
export const [datas, setDatas] = createStore({
    assistants: [] as any[],
    activatedModels: [] as ActivatedModel[], // 新增：全局同步的激活模型列表
});

// 新增：全局当前选中的模型（供 Chat 页面直接使用）
export const [selectedModel, setSelectedModel] = createSignal<ActivatedModel | null>(null);


// 创建全局 Signal 存储当前选中的助手 ID
export const [currentAssistantId, setCurrentAssistantId] = createSignal<string | null>(null);
// 增加一个全局状态来跟踪当前选中的话题 ID
export const [currentTopicId, setCurrentTopicId] = createSignal<string | null>(null);

const initialConfig: AppConfig = JSON.parse(localStorage.getItem('app_config') || '{"apiUrl":"","apiKey":"","defaultModel":"gpt-4o"}');

// 辅助函数：保存数据到后端
export const saveSingleAssistantToBackend = async (id: string) => {
    const asst = datas.assistants.find(a => a.id === id);
    if (!asst) return;

    try {
        // 调用新的细粒度命令
        await invoke('save_assistant', { assistant: JSON.parse(JSON.stringify(asst)) });
    } catch (err) {
        console.error('保存助手失败:', err);
    }
};

// 后端删除命令调用
export const deleteAssistantFile = async (id: string) => {
    try {
        await invoke('delete_assistant', { id });
    } catch (err) {
        console.error('物理删除失败:', err);
    }
}

export const [config, setConfig] = createSignal<AppConfig>(initialConfig);

export const saveConfig = (newConfig: AppConfig) => {
    setConfig(newConfig);
    localStorage.setItem('app_config', JSON.stringify(newConfig));
};
