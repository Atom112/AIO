import { Component, createSignal, For, createMemo, onCleanup, onMount } from 'solid-js';
import { config, saveConfig, setDatas, selectedModel } from '../store/store';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import './ProviderSettings.css';
interface ModelItem { id: string; owned_by: string; }
interface ActivatedModel { owned_by: string; api_url: string; api_key: string; model_id: string; local_path?: string; }

const ProviderSettings: Component = () => {
    const [apiUrl, setApiUrl] = createSignal(config().apiUrl);
    const [apiKey, setApiKey] = createSignal(config().apiKey);
    const [saveStatus, setSaveStatus] = createSignal("");
    const [localModelPath, setLocalModelPath] = createSignal("");
    const [isLocalRunning, setIsLocalRunning] = createSignal(false);
    const [models, setModels] = createSignal<ModelItem[]>([]);
    const [isLoading, setIsLoading] = createSignal(false);
    const [searchQuery, setSearchQuery] = createSignal("");
    const [selectedProvider, setSelectedProvider] = createSignal("All");
    const [isDropdownOpen, setIsDropdownOpen] = createSignal(false);
    const [activatedModels, setActivatedModels] = createSignal<ActivatedModel[]>([]);
    const [searchQueryAct, setSearchQueryAct] = createSignal("");
    const [selectedProviderAct, setSelectedProviderAct] = createSignal("All");
    const [isDropdownOpenAct, setIsDropdownOpenAct] = createSignal(false);

    let dropdownRef: HTMLDivElement | undefined;
    let dropdownRefAct: HTMLDivElement | undefined;

    const getModelLogo = (modelName: string) => {
        const name = modelName.toLowerCase();
        if (name.includes('gpt')) return '/icons/openai.svg';
        if (name.includes('claude')) return '/icons/claude-color.svg';
        if (name.includes('deepseek')) return '/icons/deepseek-color.svg';
        return '/icons/ollama.svg';
    };


    // ==========================================
    // 4. 业务逻辑函数
    // ==========================================

    /**
     * 打开文件对话框选择本地 .gguf 模型文件
     * 选择后将自动提取信息并添加到已激活模型列表中
     */
    const selectModelFile = async () => {
        const file = await open({
            multiple: false,
            filters: [{ name: 'GGUF Models', extensions: ['gguf'] }]
        });

        if (file) {
            setLocalModelPath(file);

            // 选择完立即添加到激活列表
            const fileNameWithExt = file.split(/[\\/]/).pop() || "local-model";
            const modelName = fileNameWithExt.replace(/\.[^/.]+$/, "");

            const localModelInfo: ActivatedModel = {
                model_id: modelName,
                owned_by: "Local-Llama.cpp",
                api_url: "http://127.0.0.1:8080/v1", // 默认本地端口
                api_key: "local-no-key",
                local_path: file
            };

            // 更新列表并持久化保存
            const exists = activatedModels().find(m => m.local_path === file);
            if (!exists) {
                const newList = [...activatedModels(), localModelInfo];
                setActivatedModels(newList);
                setDatas('activatedModels', newList);
                await invoke('save_activated_models', { models: newList });
                setSaveStatus(`已添加本地模型: ${modelName}`);
            }
        }
    };

    /**
     * 切换本地 Llama 服务器的运行状态
     * 启动时会将选中的模型路径和 GPU 参数发送给后端
     */
    const toggleLocalEngine = async () => {
        if (isLocalRunning()) {
            await invoke('stop_local_server');
            setIsLocalRunning(false);
            setSaveStatus("本地引擎已停止");
        } else {
            if (!localModelPath()) return alert("请先选择模型文件");
            try {
                const currentCfg = await invoke<any>('load_app_config');
                // 先保存路径到配置中
                await invoke('save_app_config', {
                    config: { ...currentCfg, localModelPath: localModelPath() }
                });
                setSaveStatus("正在启动本地引擎...");

                // 调用 Rust 启动服务器
                const serverUrl: string = await invoke('start_local_server', {
                    modelPath: localModelPath(),
                    port: 8080,
                    gpuLayers: 99 // 默认尽可能使用 GPU 加速
                });

                setIsLocalRunning(true);
                setSaveStatus("本地引擎已就绪");

                const fullPath = localModelPath();
                const fileNameWithExt = fullPath.split(/[\\/]/).pop() || "local-model";
                const modelName = fileNameWithExt.replace(/\.[^/.]+$/, "");

                const localModelInfo: ActivatedModel = {
                    model_id: modelName,
                    owned_by: "Local-Llama.cpp",
                    api_url: serverUrl,
                    api_key: "local-no-key"
                };

                // 防止重复添加相同配置的模型
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
     * 从激活列表中移除指定的模型
     * @param target 待移除的 ActivatedModel 对象
     */
    const removeActivatedModel = async (target: ActivatedModel) => {
        const newList = activatedModels().filter(m =>
            !(m.model_id === target.model_id && m.api_url === target.api_url)
        );

        setActivatedModels(newList);
        setDatas('activatedModels', newList);
        await invoke('save_activated_models', { models: newList });
    };
    // ... (此处保留你原有的 selectModelFile, toggleLocalEngine, removeActivatedModel, toggleActivation, handleSave, handleQueryModels 函数逻辑)
    // 注意：为了篇幅，这些函数内部实现请参考你原有的代码

    onMount(async () => {
        const listAct = await invoke<ActivatedModel[]>('load_activated_models');
        setActivatedModels(listAct);
        const cachedModels = await invoke<ModelItem[]>('load_fetched_models');
        if (cachedModels) setModels(cachedModels);
        const running = await invoke<boolean>('is_local_server_running');
        setIsLocalRunning(running);
        const configData = await invoke<any>('load_app_config');
        if (configData.localModelPath) setLocalModelPath(configData.localModelPath);

        const handleClickOutside = (e: MouseEvent) => {
            if (dropdownRef && !dropdownRef.contains(e.target as Node)) setIsDropdownOpen(false);
            if (dropdownRefAct && !dropdownRefAct.contains(e.target as Node)) setIsDropdownOpenAct(false);
        };
        window.addEventListener('click', handleClickOutside);
        onCleanup(() => window.removeEventListener('click', handleClickOutside));
    });

    const providers = createMemo(() => ["All", ...Array.from(new Set(models().map(m => m.owned_by || "unknown")))]);
    const filteredModels = createMemo(() => models().filter(m => m.id.toLowerCase().includes(searchQuery().toLowerCase()) && (selectedProvider() === "All" || m.owned_by === selectedProvider())));
    const providersAct = createMemo(() => ["All", ...Array.from(new Set(activatedModels().map(m => m.owned_by || "unknown")))]);
    const filteredActivatedModels = createMemo(() => activatedModels().filter(m => m.model_id.toLowerCase().includes(searchQueryAct().toLowerCase()) && (selectedProviderAct() === "All" || m.owned_by === selectedProviderAct())));

    // ==========================================
    // 7. 核心操作交互
    // ==========================================

    /**
     * 在可用列表中勾选/取消勾选，以激活或停用某个模型
     * @param item 被选中的可用模型项
     */
    const toggleActivation = async (item: ModelItem) => {
        const isCurrentlyActive = activatedModels().some(m => m.model_id === item.id && m.api_url === apiUrl());
        let newList;
        if (isCurrentlyActive) {
            newList = activatedModels().filter(m => !(m.model_id === item.id && m.api_url === apiUrl()));
        } else {
            const newActive: ActivatedModel = {
                model_id: item.id,
                owned_by: item.owned_by || "unknown",
                api_url: apiUrl(),
                api_key: apiKey(),
            };
            newList = [...activatedModels(), newActive];
        }
        setActivatedModels(newList);
        setDatas('activatedModels', newList); // 同步到全局 store 供 NavBar 等组件感知
        await invoke('save_activated_models', { models: newList });
    };

    /**
     * 保存当前输入的 API 供应商网址和密钥
     */
    const handleSave = async () => {
        const newConfig = { apiUrl: apiUrl(), apiKey: apiKey(), defaultModel: selectedModel()?.model_id || "" };
        await invoke('save_app_config', { config: newConfig });
        saveConfig(newConfig as any);
        setSaveStatus("配置已成功保存！");
        setTimeout(() => setSaveStatus(""), 3000);
    };

    /**
     * 调用后端接口，从指定的 API 供应商处拉取可用的模型列表
     */
    const handleQueryModels = async () => {
        setIsLoading(true);
        try {
            const list: ModelItem[] = await invoke('fetch_models', {
                apiUrl: apiUrl(),
                apiKey: apiKey()
            });
            setModels(list);
            // 搜索成功后将列表持久化到本地
            await invoke('save_fetched_models', { models: list });
        } catch (err) {
            alert(`查询失败: ${err}`);
        }
        finally {
            setIsLoading(false);
        }
    };


    return (
        <div class="tab-content-provider">
            <div class="settings-container">
                <div class="info-header" style="border-bottom: 1px solid; padding-bottom: 10px; margin-bottom: 30px;">
                    <h3>API 服务配置</h3>
                </div>
                <div class="settings-form">
                    <div class="setting-item">
                        <label>API 供应商网址</label>
                        <input type="text" class="settings-input" value={apiUrl()} onInput={(e) => setApiUrl(e.currentTarget.value)} />
                    </div>
                    <div class="setting-item">
                        <label>API Key</label>
                        <input type="password" class="settings-input" value={apiKey()} onInput={(e) => setApiKey(e.currentTarget.value)} />
                    </div>
                    <button class="save-settings-button" onClick={handleSave}>保存当前配置</button>
                    <button class="query-models-button" onClick={handleQueryModels} disabled={isLoading()}>{isLoading() ? "查询中..." : "获取可用模型列表"}</button>

                    <div class="setting-item">
                        <label>本地模型管理 (.gguf)</label>
                        <div style="display:flex; flex-direction: column; gap: 10px;">
                            <button class="select-file-button" onClick={selectModelFile} style="width: 100%">
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
                    {saveStatus() && <div class="save-hint">{saveStatus()}</div>}
                </div>
            </div>

            {/* 中间：已激活/已保存的模型卡片列表 */}
            <div class="models-list-panel activated-panel">
                <div class="models-header-complex">
                    <h4>已激活模型 ({filteredActivatedModels().length})</h4>
                    <div class="models-tools-row">
                        <input type="text" placeholder="搜索已激活..." class="models-search-input" onInput={(e) => setSearchQueryAct(e.currentTarget.value)} />
                        {/* 厂商筛选下拉框 */}
                        <div class="custom-select-container" ref={dropdownRefAct}>
                            <div class={`custom-select-trigger ${isDropdownOpenAct() ? 'open' : ''}`} onClick={() => setIsDropdownOpenAct(!isDropdownOpenAct())}>
                                {selectedProviderAct() === "All" ? "所有厂商" : selectedProviderAct()}
                                <span class="arrow-icon">▼</span>
                            </div>
                            {isDropdownOpenAct() && (
                                <div class="custom-select-dropdown">
                                    <For each={providersAct()}>{(p) => (
                                        <div class={`custom-select-item ${selectedProviderAct() === p ? 'active' : ''}`}
                                            onClick={() => { setSelectedProviderAct(p); setIsDropdownOpenAct(false); }}>
                                            {p === "All" ? "所有厂商" : p}
                                        </div>
                                    )}</For>
                                </div>
                            )}
                        </div>
                    </div>
                </div>
                <div class="models-scroll-area">
                    <For each={filteredActivatedModels()}>{(m) => (
                        <div class="model-card activated">
                            {/* 1. 在这里插入 Logo */}
                            <div class="model-logo-container">
                                <img src={getModelLogo(m.model_id)} alt="logo" class="model-item-logo" />
                            </div>

                            <div class="model-info">
                                <span class="model-id">{m.model_id}</span>
                                <span class="model-provider">
                                    {m.api_url.includes('127.0.0.1') ? '本地服务' : m.api_url.replace('https://', '').split('/')[0]}
                                </span>
                            </div>
                            <button class="remove-act-btn" onClick={() => removeActivatedModel(m)}>移除</button>
                        </div>
                    )}</For>
                </div>
            </div>

            {/* 右侧：从 API 获取的可利用模型列表 */}
            <div class="models-list-panel">
                <div class="models-header-complex">
                    <h4>可用模型列表 ({filteredModels().length})</h4>
                    <div class="models-tools-row">
                        <input type="text" placeholder="搜索可用..." class="models-search-input" onInput={(e) => setSearchQuery(e.currentTarget.value)} />
                        <div class="custom-select-container" ref={dropdownRef}>
                            <div class={`custom-select-trigger ${isDropdownOpen() ? 'open' : ''}`} onClick={() => setIsDropdownOpen(!isDropdownOpen())}>
                                {selectedProvider() === "All" ? "所有厂商" : selectedProvider()}
                                <span class="arrow-icon">▼</span>
                            </div>
                            {isDropdownOpen() && (
                                <div class="custom-select-dropdown">
                                    <For each={providers()}>{(p) => (
                                        <div class={`custom-select-item ${selectedProvider() === p ? 'active' : ''}`}
                                            onClick={() => { setSelectedProvider(p); setIsDropdownOpen(false); }}>
                                            {p === "All" ? "所有厂商" : p}
                                        </div>
                                    )}</For>
                                </div>
                            )}
                        </div>
                    </div>
                </div>
                <div class="models-scroll-area">
                    {models().length === 0 && (
                        <div style="color: #444; text-align: center; margin-top: 50px;">
                            点击左侧“查询模型”按钮获取列表
                        </div>
                    )}
                    <For each={filteredModels()}>{(m) => (
                        <div class="model-card">
                            {/* 2. 在这里插入 Logo */}
                            <div class="model-logo-container">
                                <img src={getModelLogo(m.id)} alt="logo" class="model-item-logo" />
                            </div>

                            <div class="model-info">
                                <span class="model-id">{m.id}</span>
                                <span class="model-provider">Provider: {m.owned_by}</span>
                            </div>
                            <div class="switch-container">
                                <label class="switch">
                                    <input type="checkbox"
                                        checked={activatedModels().some(am => am.model_id === m.id && am.api_url === apiUrl())}
                                        onChange={() => toggleActivation(m)} />
                                    <span class="slider"></span>
                                </label>
                            </div>
                        </div>
                    )}</For>
                </div>
            </div>
        </div>
    );
};

export default ProviderSettings;