import { Component, createSignal, For, createMemo, onCleanup, onMount } from 'solid-js';
import { config, saveConfig,setDatas } from '../store/store';
import { invoke } from '@tauri-apps/api/core';
import './Settings.css';

interface ModelItem {
    id: string;
    owned_by: string;
}

interface ActivatedModel  {
    owned_by: string;
    api_url: string;
    api_key: string;
    model_id: string; // 对应 ModelItem 的 id
}

const Settings: Component = () => {
    // 1. 基础配置状态
    const [apiUrl, setApiUrl] = createSignal(config().apiUrl);
    const [apiKey, setApiKey] = createSignal(config().apiKey);
    const [saveStatus, setSaveStatus] = createSignal("");

    // 2. 右侧：可用模型列表状态
    const [models, setModels] = createSignal<ModelItem[]>([]);
    const [isLoading, setIsLoading] = createSignal(false);
    const [searchQuery, setSearchQuery] = createSignal("");
    const [selectedProvider, setSelectedProvider] = createSignal("All");
    const [isDropdownOpen, setIsDropdownOpen] = createSignal(false);

    // 3. 中间：已激活模型列表状态
    const [activatedModels, setActivatedModels] = createSignal<ActivatedModel[]>([]);
    const [searchQueryAct, setSearchQueryAct] = createSignal("");
    const [selectedProviderAct, setSelectedProviderAct] = createSignal("All");
    const [isDropdownOpenAct, setIsDropdownOpenAct] = createSignal(false);

    let dropdownRef: HTMLDivElement | undefined;
    let dropdownRefAct: HTMLDivElement | undefined;

    // 初始化加载
    onMount(async () => {
        try {
            const list = await invoke<ActivatedModel[]>('load_activated_models');
            setActivatedModels(list);
        } catch (e) { console.error(e); }

        const handleClickOutside = (e: MouseEvent) => {
            if (dropdownRef && !dropdownRef.contains(e.target as Node)) setIsDropdownOpen(false);
            if (dropdownRefAct && !dropdownRefAct.contains(e.target as Node)) setIsDropdownOpenAct(false);
        };
        window.addEventListener('click', handleClickOutside);
        onCleanup(() => window.removeEventListener('click', handleClickOutside));
    });

    // --- 计算属性：右侧可用列表 ---
    const providers = createMemo(() => ["All", ...Array.from(new Set(models().map(m => m.owned_by || "unknown")))]);
    const filteredModels = createMemo(() => models().filter(m =>
        m.id.toLowerCase().includes(searchQuery().toLowerCase()) &&
        (selectedProvider() === "All" || m.owned_by === selectedProvider())
    ));

    // --- 计算属性：中间激活列表 ---
    const providersAct = createMemo(() => ["All", ...Array.from(new Set(activatedModels().map(m => m.owned_by || "unknown")))]);
    const filteredActivatedModels = createMemo(() => activatedModels().filter(m =>
        m.model_id.toLowerCase().includes(searchQueryAct().toLowerCase()) &&
        (selectedProviderAct() === "All" || m.owned_by === selectedProviderAct())
    ));

    // 核心逻辑：开关切换
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
    setDatas('activatedModels', newList); // 重要：同步到全局 Store，NavBar 会立即感知
    await invoke('save_activated_models', { models: newList });
};

const handleSave = async () => {
    const newConfig = { apiUrl: apiUrl(), apiKey: apiKey() }; // 删除了 defaultModel
    await invoke('save_app_config', { config: newConfig });
    saveConfig(newConfig as any);
    setSaveStatus("配置已成功保存！");
    setTimeout(() => setSaveStatus(""), 3000);
};

    const handleQueryModels = async () => {
        setIsLoading(true);
        try {
            const list: ModelItem[] = await invoke('fetch_models', { apiUrl: apiUrl(), apiKey: apiKey() });
            setModels(list);
        } catch (err) { alert(`查询失败: ${err}`); }
        finally { setIsLoading(false); }
    };

    return (
        <div class="settings-page">
            {/* 左侧：表单 */}
            <div class="settings-container">
                <div class="info-header" style="border-bottom: 1px solid #08ddf9; padding-bottom: 10px; margin-bottom: 30px;">
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
                    {saveStatus() && <div class="save-hint">{saveStatus()}</div>}
                </div>
            </div>

            {/* 中间：已激活模型列表 */}
            <div class="models-list-panel activated-panel">
                <div class="models-header-complex">
                    <h4>已激活模型 ({filteredActivatedModels().length})</h4>
                    <div class="models-tools-row">
                        <input type="text" placeholder="搜索已激活..." class="models-search-input" onInput={(e) => setSearchQueryAct(e.currentTarget.value)} />
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
                            <div class="model-info">
                                <span class="model-id">{m.model_id}</span>
                                <span class="model-provider" style="color: #08ddf9;">{m.api_url.replace('https://', '').split('/')[0]}</span>
                            </div>
                            <button class="remove-act-btn" onClick={() => toggleActivation({ id: m.model_id, owned_by: m.owned_by })}>移除</button>
                        </div>
                    )}</For>
                </div>
            </div>

            {/* 右侧：可用模型列表 */}
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

export default Settings;
