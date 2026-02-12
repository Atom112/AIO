/**
 * ============================================================================
 * 文件功能摘要
 * ============================================================================
 * 
 * 【核心功能】
 * AI 模型提供商管理设置页面，提供 API 配置管理、模型列表获取、本地模型管理三大功能模块。
 * 支持云端 API 模型（OpenAI 格式）的激活/停用，以及本地 Llama.cpp 模型的启动/停止控制。
 * 
 * 【数据流流向】
 * ┌─────────────────────────────────────────────────────────────────────────┐
 * │  全局状态流入                                                            │
 * │  ├── config() ← 当前 API 配置（url/key）                                 │
 * │  ├── selectedModel() ← 当前选中的模型                                    │
 * │  └── setDatas('activatedModels') → 更新全局已激活模型列表                │
 * │                                                                          │
 * │  Tauri 后端命令调用                                                      │
 * │  ├── invoke('load_app_config') → 加载应用配置                            │
 * │  ├── invoke('save_app_config') → 保存 API 配置                           │
 * │  ├── invoke('load_activated_models') → 加载已激活模型                      │
 * │  ├── invoke('save_activated_models') → 持久化激活模型列表                │
 * │  ├── invoke('load_fetched_models') → 加载缓存的可用模型                    │
 * │  ├── invoke('save_fetched_models') → 缓存可用模型列表                      │
 * │  ├── invoke('fetch_models') → 从 API 端点获取模型列表                      │
 * │  ├── invoke('start_local_server') → 启动本地 Llama 服务                    │
 * │  ├── invoke('stop_local_server') → 停止本地 Llama 服务                   │
 * │  ├── invoke('is_local_server_running') → 检查本地服务状态                  │
 * │  └── open() (dialog plugin) → 选择本地 .gguf 模型文件                      │
 * │                                                                          │
 * │  本地状态                                                                │
 * │  ├── apiUrl/apiKey: 表单输入的 API 配置                                    │
 * │  ├── localModelPath: 选中的本地模型文件路径                                │
 * │  ├── isLocalRunning: 本地服务运行状态                                      │
 * │  ├── models: 从 API 获取的可用模型列表                                     │
 * │  ├── activatedModels: 已激活的模型配置列表                                 │
 * │  ├── searchQuery/searchQueryAct: 搜索关键词                                │
 * │  ├── selectedProvider/selectedProviderAct: 厂商筛选条件                  │
 * │  └── isDropdownOpen/isDropdownOpenAct: 下拉菜单展开状态                    │
 * └─────────────────────────────────────────────────────────────────────────┘
 * 
 * 【组件层级】
 * ProviderSettings (本组件)
 * ├── 左侧：API 配置面板
 * │   ├── API URL 输入
 * │   ├── API Key 输入
 * │   ├── 保存配置按钮
 * │   ├── 获取模型列表按钮
 * │   └── 本地模型管理区（选择文件 + 启动/停止服务）
 * ├── 中间：已激活模型列表面板（可搜索筛选、移除）
 * └── 右侧：可用模型列表面板（从 API 获取，可勾选激活）
 * ============================================================================
 */

// SolidJS 核心 API
import { Component, createSignal, For, createMemo, onCleanup, onMount } from 'solid-js';
// 全局状态管理
import { config, saveConfig, setDatas, selectedModel } from '../store/store';
// Tauri 核心 API
import { invoke } from '@tauri-apps/api/core';
// Tauri 对话框插件
import { open } from '@tauri-apps/plugin-dialog';
// 本地样式
import './ProviderSettings.css';

/** 从 API 获取的原始模型信息 */
interface ModelItem { 
    id: string; 
    owned_by: string; 
}

/** 已激活的模型完整配置（含 API 连接信息） */
interface ActivatedModel { 
    owned_by: string; 
    api_url: string; 
    api_key: string; 
    model_id: string; 
    local_path?: string; 
}

/**
 * 提供商设置页面组件
 * 
 * @component
 * @description 管理 AI 模型 API 配置和模型激活状态，支持云端和本地模型。
 * 
 * @returns {JSX.Element} 设置页面 JSX 元素
 */
const ProviderSettings: Component = () => {
    // ==================== 表单状态 ====================

    /** API 供应商网址输入值 */
    const [apiUrl, setApiUrl] = createSignal(config().apiUrl);
    /** API Key 输入值 */
    const [apiKey, setApiKey] = createSignal(config().apiKey);
    /** 保存状态提示文本（如"配置已保存"） */
    const [saveStatus, setSaveStatus] = createSignal("");

    // ==================== 本地模型状态 ====================

    /** 选中的本地 .gguf 模型文件路径 */
    const [localModelPath, setLocalModelPath] = createSignal("");
    /** 本地 Llama 服务是否正在运行 */
    const [isLocalRunning, setIsLocalRunning] = createSignal(false);

    // ==================== 模型列表状态 ====================

    /** 从 API 获取的可用模型列表 */
    const [models, setModels] = createSignal<ModelItem[]>([]);
    /** 是否正在查询模型列表 */
    const [isLoading, setIsLoading] = createSignal(false);
    /** 已激活的模型配置列表（持久化到本地） */
    const [activatedModels, setActivatedModels] = createSignal<ActivatedModel[]>([]);

    // ==================== 搜索筛选状态（可用模型）====================

    /** 可用模型搜索关键词 */
    const [searchQuery, setSearchQuery] = createSignal("");
    /** 可用模型厂商筛选条件 */
    const [selectedProvider, setSelectedProvider] = createSignal("All");
    /** 可用模型厂商下拉菜单展开状态 */
    const [isDropdownOpen, setIsDropdownOpen] = createSignal(false);

    // ==================== 搜索筛选状态（已激活模型）====================

    /** 已激活模型搜索关键词 */
    const [searchQueryAct, setSearchQueryAct] = createSignal("");
    /** 已激活模型厂商筛选条件 */
    const [selectedProviderAct, setSelectedProviderAct] = createSignal("All");
    /** 已激活模型厂商下拉菜单展开状态 */
    const [isDropdownOpenAct, setIsDropdownOpenAct] = createSignal(false);

    // ==================== DOM 引用 ====================

    /** 可用模型厂商下拉容器引用（用于点击外部关闭） */
    let dropdownRef: HTMLDivElement | undefined;
    /** 已激活模型厂商下拉容器引用（用于点击外部关闭） */
    let dropdownRefAct: HTMLDivElement | undefined;

    /**
     * 根据模型名称获取对应的品牌 Logo 路径
     * 
     * @param {string} modelName - 模型名称或 ID
     * @returns {string} Logo 图片的 URL 路径
     */
    const getModelLogo = (modelName: string) => {
        const name = modelName.toLowerCase();
        if (name.includes('gpt')) return '/icons/openai.svg';
        if (name.includes('claude')) return '/icons/claude-color.svg';
        if (name.includes('grok')) return '/icons/grok.svg';
        if (name.includes('gemini')) return '/icons/gemini-color.svg';
        if (name.includes('deepseek')) return '/icons/deepseek-color.svg';
        if (name.includes('qwen')) return '/icons/qwen-color.svg';
        if (name.includes('kimi') || name.includes('moonshot')) return '/icons/moonshot.svg';
        if (name.includes('doubao')) return '/icons/doubao-color.svg';
        if (name.includes('glm')) return '/icons/zhipu-color.svg';
        return '/icons/ollama.svg';
    };

    // ==================== 本地模型管理 ====================

    /**
     * 打开文件对话框选择本地 .gguf 模型文件
     * 
     * 数据流：
     * 1. 调用 Tauri open() 打开系统文件选择器（过滤 .gguf）
     * 2. 提取文件名作为模型名称
     * 3. 构建 ActivatedModel 对象（标记为 Local-Llama.cpp）
     * 4. 添加到激活列表并持久化保存
     */
    const selectModelFile = async () => {
        const file = await open({
            multiple: false,
            filters: [{ name: 'GGUF Models', extensions: ['gguf'] }]
        });

        if (file) {
            setLocalModelPath(file);

            // 从路径提取文件名并去除扩展名
            const fileNameWithExt = file.split(/[\\/]/).pop() || "local-model";
            const modelName = fileNameWithExt.replace(/\.[^/.]+$/, "");

            // 构建本地模型配置对象
            const localModelInfo: ActivatedModel = {
                model_id: modelName,
                owned_by: "Local-Llama.cpp",
                api_url: "http://127.0.0.1:8080/v1", // 默认本地端口
                api_key: "local-no-key",
                local_path: file
            };

            // 防止重复添加相同路径的模型
            const exists = activatedModels().find(m => m.local_path === file);
            if (!exists) {
                const newList = [...activatedModels(), localModelInfo];
                setActivatedModels(newList);
                setDatas('activatedModels', newList); // 同步到全局 Store
                await invoke('save_activated_models', { models: newList });
                setSaveStatus(`已添加本地模型: ${modelName}`);
            }
        }
    };

    /**
     * 切换本地 Llama 服务器的运行状态（启动/停止）
     * 
     * 启动数据流：
     * 1. 保存模型路径到应用配置
     * 2. 调用 start_local_server 启动后端服务（GPU 加速）
     * 3. 将启动的模型添加到激活列表（如未存在）
     * 4. 更新运行状态标志
     * 
     * 停止数据流：
     * 1. 调用 stop_local_server 停止后端服务
     * 2. 更新运行状态标志
     */
    const toggleLocalEngine = async () => {
        if (isLocalRunning()) {
            // 停止分支
            await invoke('stop_local_server');
            setIsLocalRunning(false);
            setSaveStatus("本地引擎已停止");
        } else {
            // 启动分支
            if (!localModelPath()) return alert("请先选择模型文件");
            
            try {
                // 保存路径到配置
                const currentCfg = await invoke<any>('load_app_config');
                await invoke('save_app_config', {
                    config: { ...currentCfg, localModelPath: localModelPath() }
                });
                
                setSaveStatus("正在启动本地引擎...");

                // 启动本地服务
                const serverUrl: string = await invoke('start_local_server', {
                    modelPath: localModelPath(),
                    port: 8080,
                    gpuLayers: 99 // 默认尽可能使用 GPU 加速
                });

                setIsLocalRunning(true);
                setSaveStatus("本地引擎已就绪");

                // 提取模型名称
                const fullPath = localModelPath();
                const fileNameWithExt = fullPath.split(/[\\/]/).pop() || "local-model";
                const modelName = fileNameWithExt.replace(/\.[^/.]+$/, "");

                // 构建服务启动后的模型配置
                const localModelInfo: ActivatedModel = {
                    model_id: modelName,
                    owned_by: "Local-Llama.cpp",
                    api_url: serverUrl,
                    api_key: "local-no-key"
                };

                // 防止重复添加
                if (!activatedModels().some(m => m.model_id === modelName && m.api_url === serverUrl)) {
                    const newList = [...activatedModels(), localModelInfo];
                    setActivatedModels(newList);
                    setDatas('activatedModels', newList);
                    await invoke('save_activated_models', { models: newList });
                }

                setSaveStatus(`本地模型 ${modelName} 已启动`);
            } catch (err) {
                alert("启动失败: " + err);
                setIsLocalRunning(false);
            }
        }
    };

    /**
     * 从激活列表中移除指定模型
     * 
     * @param {ActivatedModel} target - 待移除的模型配置对象
     */
    const removeActivatedModel = async (target: ActivatedModel) => {
        // 根据 model_id 和 api_url 组合唯一标识过滤
        const newList = activatedModels().filter(m =>
            !(m.model_id === target.model_id && m.api_url === target.api_url)
        );

        setActivatedModels(newList);
        setDatas('activatedModels', newList);
        await invoke('save_activated_models', { models: newList });
    };

    // ==================== 生命周期钩子 ====================

    /**
     * 组件挂载时初始化数据
     * 
     * 加载流程：
     * 1. 加载已激活模型列表
     * 2. 加载缓存的可用模型列表
     * 3. 检查本地服务运行状态
     * 4. 加载应用配置（恢复本地模型路径）
     * 5. 注册全局点击监听（关闭下拉菜单）
     */
    onMount(async () => {
        // 加载持久化数据
        const listAct = await invoke<ActivatedModel[]>('load_activated_models');
        setActivatedModels(listAct);
        
        const cachedModels = await invoke<ModelItem[]>('load_fetched_models');
        if (cachedModels) setModels(cachedModels);
        
        const running = await invoke<boolean>('is_local_server_running');
        setIsLocalRunning(running);
        
        const configData = await invoke<any>('load_app_config');
        if (configData.localModelPath) setLocalModelPath(configData.localModelPath);

        // 点击外部关闭下拉菜单
        const handleClickOutside = (e: MouseEvent) => {
            if (dropdownRef && !dropdownRef.contains(e.target as Node)) setIsDropdownOpen(false);
            if (dropdownRefAct && !dropdownRefAct.contains(e.target as Node)) setIsDropdownOpenAct(false);
        };
        window.addEventListener('click', handleClickOutside);
        onCleanup(() => window.removeEventListener('click', handleClickOutside));
    });

    // ==================== 派生状态（Memo）====================

    /** 可用模型的厂商列表（去重，用于筛选下拉） */
    const providers = createMemo(() => ["All", ...Array.from(new Set(models().map(m => m.owned_by || "unknown")))]);
    
    /** 筛选后的可用模型列表（搜索 + 厂商筛选） */
    const filteredModels = createMemo(() => 
        models().filter(m => 
            m.id.toLowerCase().includes(searchQuery().toLowerCase()) && 
            (selectedProvider() === "All" || m.owned_by === selectedProvider())
        )
    );

    /** 已激活模型的厂商列表（去重） */
    const providersAct = createMemo(() => ["All", ...Array.from(new Set(activatedModels().map(m => m.owned_by || "unknown")))]);
    
    /** 筛选后的已激活模型列表（搜索 + 厂商筛选） */
    const filteredActivatedModels = createMemo(() => 
        activatedModels().filter(m => 
            m.model_id.toLowerCase().includes(searchQueryAct().toLowerCase()) && 
            (selectedProviderAct() === "All" || m.owned_by === selectedProviderAct())
        )
    );

    // ==================== 核心操作交互 ====================

    /**
     * 切换模型的激活状态（勾选/取消勾选）
     * 
     * 数据流：
     * - 取消激活：从 activatedModels 中过滤移除
     * - 激活：构建 ActivatedModel 对象（包含当前 API 配置）并添加
     * - 持久化：保存到后端并同步全局 Store
     * 
     * @param {ModelItem} item - 从 API 获取的可用模型项
     */
    const toggleActivation = async (item: ModelItem) => {
        // 检查是否已使用当前 API 配置激活
        const isCurrentlyActive = activatedModels().some(m => 
            m.model_id === item.id && m.api_url === apiUrl()
        );
        
        let newList;
        if (isCurrentlyActive) {
            // 取消激活：移除匹配项
            newList = activatedModels().filter(m => 
                !(m.model_id === item.id && m.api_url === apiUrl())
            );
        } else {
            // 激活：构建完整配置对象
            const newActive: ActivatedModel = {
                model_id: item.id,
                owned_by: item.owned_by || "unknown",
                api_url: apiUrl(),
                api_key: apiKey(),
            };
            newList = [...activatedModels(), newActive];
        }
        
        setActivatedModels(newList);
        setDatas('activatedModels', newList); // 同步到全局 Store 供 NavBar 等组件使用
        await invoke('save_activated_models', { models: newList });
    };

    /**
     * 保存当前 API 配置
     * 
     * 数据流：apiUrl/apiKey → save_app_config → saveConfig → 全局状态更新
     */
    const handleSave = async () => {
        const newConfig = { 
            apiUrl: apiUrl(), 
            apiKey: apiKey(), 
            defaultModel: selectedModel()?.model_id || "" 
        };
        await invoke('save_app_config', { config: newConfig });
        saveConfig(newConfig as any);
        setSaveStatus("配置已成功保存！");
        setTimeout(() => setSaveStatus(""), 3000);
    };

    /**
     * 从配置的 API 端点获取可用模型列表
     * 
     * 数据流：
     * 1. 调用 fetch_models 后端命令（携带 apiUrl/apiKey）
     * 2. 更新本地 models 状态
     * 3. 缓存结果到本地存储（save_fetched_models）
     */
    const handleQueryModels = async () => {
        setIsLoading(true);
        try {
            const list: ModelItem[] = await invoke('fetch_models', {
                apiUrl: apiUrl(),
                apiKey: apiKey()
            });
            setModels(list);
            await invoke('save_fetched_models', { models: list });
        } catch (err) {
            alert(`查询失败: ${err}`);
        } finally {
            setIsLoading(false);
        }
    };

    // ==================== 渲染逻辑 ====================

    return (
        <div class="tab-content-provider">
            <div class="settings-container">
                {/* API 配置区域 */}
                <div class="info-header" style="border-bottom: 1px solid; padding-bottom: 10px; margin-bottom: 30px;">
                    <h3>API 服务配置</h3>
                </div>
                
                <div class="settings-form">
                    {/* API URL 输入 */}
                    <div class="setting-item">
                        <label>API 供应商网址</label>
                        <input 
                            type="text" 
                            class="settings-input" 
                            value={apiUrl()} 
                            onInput={(e) => setApiUrl(e.currentTarget.value)} 
                        />
                    </div>
                    
                    {/* API Key 输入 */}
                    <div class="setting-item">
                        <label>API Key</label>
                        <input 
                            type="password" 
                            class="settings-input" 
                            value={apiKey()} 
                            onInput={(e) => setApiKey(e.currentTarget.value)} 
                        />
                    </div>
                    
                    {/* 操作按钮组 */}
                    <button class="save-settings-button" onClick={handleSave}>
                        保存当前配置
                    </button>
                    <button 
                        class="save-settings-button" 
                        onClick={handleQueryModels} 
                        disabled={isLoading()}
                    >
                        {isLoading() ? "查询中..." : "获取可用模型列表"}
                    </button>

                    {/* 本地模型管理区 */}
                    <div class="setting-item">
                        <label>本地模型管理 (.gguf)</label>
                        <div style="display:flex; flex-direction: column; gap: 10px;">
                            <button 
                                class="save-settings-button" 
                                onClick={selectModelFile} 
                                style="width: 100%"
                            >
                                + 选择并添加本地模型
                            </button>

                            <button
                                class="save-settings-button"
                                style={{
                                    "background-color": isLocalRunning() ? "#ff4444" : "var(--primary-color)",
                                    "margin-top": "10px"
                                }}
                                onClick={toggleLocalEngine}
                            >
                                {isLocalRunning() ? "停止本地 Llama 服务器" : "启动本地 Llama 服务器"}
                            </button>
                        </div>
                    </div>
                    
                    {/* 状态提示 */}
                    {saveStatus() && <div class="save-hint">{saveStatus()}</div>}
                </div>
            </div>

            {/* 中间：已激活模型列表面板 */}
            <div class="models-list-panel activated-panel">
                <div class="models-header-complex">
                    <h4>已激活模型 ({filteredActivatedModels().length})</h4>
                    <div class="models-tools-row">
                        {/* 搜索输入 */}
                        <input 
                            type="text" 
                            placeholder="搜索已激活..." 
                            class="models-search-input" 
                            onInput={(e) => setSearchQueryAct(e.currentTarget.value)} 
                        />
                        
                        {/* 厂商筛选下拉框 */}
                        <div class="custom-select-container" ref={dropdownRefAct}>
                            <div 
                                class={`custom-select-trigger ${isDropdownOpenAct() ? 'open' : ''}`} 
                                onClick={() => setIsDropdownOpenAct(!isDropdownOpenAct())}
                            >
                                {selectedProviderAct() === "All" ? "所有厂商" : selectedProviderAct()}
                                <span class="arrow-icon">▼</span>
                            </div>
                            {isDropdownOpenAct() && (
                                <div class="custom-select-dropdown">
                                    <For each={providersAct()}>
                                        {(p) => (
                                            <div 
                                                class={`custom-select-item ${selectedProviderAct() === p ? 'active' : ''}`}
                                                onClick={() => { setSelectedProviderAct(p); setIsDropdownOpenAct(false); }}
                                            >
                                                {p === "All" ? "所有厂商" : p}
                                            </div>
                                        )}
                                    </For>
                                </div>
                            )}
                        </div>
                    </div>
                </div>
                
                {/* 已激活模型列表 */}
                <div class="models-scroll-area">
                    <For each={filteredActivatedModels()}>
                        {(m) => (
                            <div class="model-card activated">
                                {/* 模型 Logo */}
                                <div class="model-logo-container">
                                    <img src={getModelLogo(m.model_id)} alt="logo" class="model-item-logo" />
                                </div>

                                <div class="model-info">
                                    <span class="model-id">{m.model_id}</span>
                                    <span class="model-provider">
                                        {m.api_url.includes('127.0.0.1') ? '本地服务' : m.api_url.replace('https://', '').split('/')[0]}
                                    </span>
                                </div>
                                <button class="remove-act-btn" onClick={() => removeActivatedModel(m)}>
                                    移除
                                </button>
                            </div>
                        )}
                    </For>
                </div>
            </div>

            {/* 右侧：可用模型列表面板 */}
            <div class="models-list-panel">
                <div class="models-header-complex">
                    <h4>可用模型列表 ({filteredModels().length})</h4>
                    <div class="models-tools-row">
                        <input 
                            type="text" 
                            placeholder="搜索可用..." 
                            class="models-search-input" 
                            onInput={(e) => setSearchQuery(e.currentTarget.value)} 
                        />
                        <div class="custom-select-container" ref={dropdownRef}>
                            <div 
                                class={`custom-select-trigger ${isDropdownOpen() ? 'open' : ''}`} 
                                onClick={() => setIsDropdownOpen(!isDropdownOpen())}
                            >
                                {selectedProvider() === "All" ? "所有厂商" : selectedProvider()}
                                <span class="arrow-icon">▼</span>
                            </div>
                            {isDropdownOpen() && (
                                <div class="custom-select-dropdown">
                                    <For each={providers()}>
                                        {(p) => (
                                            <div 
                                                class={`custom-select-item ${selectedProvider() === p ? 'active' : ''}`}
                                                onClick={() => { setSelectedProvider(p); setIsDropdownOpen(false); }}
                                            >
                                                {p === "All" ? "所有厂商" : p}
                                            </div>
                                        )}
                                    </For>
                                </div>
                            )}
                        </div>
                    </div>
                </div>
                
                {/* 可用模型列表 */}
                <div class="models-scroll-area">
                    {/* 空状态提示 */}
                    {models().length === 0 && (
                        <div style="color: #444; text-align: center; margin-top: 50px;">
                            点击左侧“查询模型”按钮获取列表
                        </div>
                    )}
                    
                    <For each={filteredModels()}>
                        {(m) => (
                            <div class="model-card">
                                {/* 模型 Logo */}
                                <div class="model-logo-container">
                                    <img src={getModelLogo(m.id)} alt="logo" class="model-item-logo" />
                                </div>

                                <div class="model-info">
                                    <span class="model-id">{m.id}</span>
                                    <span class="model-provider">Provider: {m.owned_by}</span>
                                </div>
                                
                                {/* 激活开关 */}
                                <div class="switch-container">
                                    <label class="switch">
                                        <input 
                                            type="checkbox"
                                            checked={activatedModels().some(am => 
                                                am.model_id === m.id && am.api_url === apiUrl()
                                            )}
                                            onChange={() => toggleActivation(m)} 
                                        />
                                        <span class="slider"></span>
                                    </label>
                                </div>
                            </div>
                        )}
                    </For>
                </div>
            </div>
        </div>
    );
};

// 默认导出组件
export default ProviderSettings;