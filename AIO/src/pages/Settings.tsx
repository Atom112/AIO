import { Component, createSignal, For, createMemo, onCleanup, onMount } from 'solid-js';
import { config, saveConfig } from '../store/store'; // 需确保 store 中有对应导出
import { invoke } from '@tauri-apps/api/core';
import './Settings.css';

interface ModelItem {
    id: string;
    owned_by: string;
}

const Settings: Component = () => {
    // 内部临时状态，用于输入时响应
    const [apiUrl, setApiUrl] = createSignal(config().apiUrl);
    const [apiKey, setApiKey] = createSignal(config().apiKey);
    const [model, setModel] = createSignal(config().defaultModel);
    const [saveStatus, setSaveStatus] = createSignal("");

    const [models, setModels] = createSignal<ModelItem[]>([]);
    const [isLoading, setIsLoading] = createSignal(false);


    // 新增：搜索和分类状态
    const [searchQuery, setSearchQuery] = createSignal("");
    const [selectedProvider, setSelectedProvider] = createSignal("All");
    const [copiedId, setCopiedId] = createSignal(""); // 用于复制反馈
    const [isDropdownOpen, setIsDropdownOpen] = createSignal(false);
    let dropdownRef: HTMLDivElement | undefined;

    const handleClickOutside = (e: MouseEvent) => {
        if (dropdownRef && !dropdownRef.contains(e.target as Node)) {
            setIsDropdownOpen(false);
        }
    };
    onMount(() => window.addEventListener('click', handleClickOutside));
    onCleanup(() => window.removeEventListener('click', handleClickOutside));

    // 计算属性：提取所有供应商并去重
    const providers = createMemo(() => {
        const p = models().map(m => m.owned_by || "unknown");
        return ["All", ...Array.from(new Set(p))];
    });

    // 计算属性：过滤后的模型列表
    const filteredModels = createMemo(() => {
        return models().filter(m => {
            const matchesSearch = m.id.toLowerCase().includes(searchQuery().toLowerCase());
            const matchesProvider = selectedProvider() === "All" || m.owned_by === selectedProvider();
            return matchesSearch && matchesProvider;
        });
    });

    const handleSave = async () => {
        const newConfig = {
            apiUrl: apiUrl(),
            apiKey: apiKey(),
            defaultModel: model()
        };
        try {
            // 调用 Rust 保存到磁盘文件
            await invoke('save_app_config', { config: newConfig });

            // 同时更新你本地的 store (保持响应式)
            saveConfig(newConfig);

            setSaveStatus("配置已成功保存！");
        } catch (err) {
            alert("保存失败: " + err);
        }

        setTimeout(() => setSaveStatus(""), 3000);
    };

    const handleQueryModels = async () => {
        setIsLoading(true);
        try {
            const list: ModelItem[] = await invoke('fetch_models', {
                apiUrl: apiUrl(),
                apiKey: apiKey()
            });
            setModels(list);
        } catch (err) {
            alert(`查询失败: ${err}`);
        } finally {
            setIsLoading(false);
        }
    };

    const copyToClipboard = (text: string) => {
        navigator.clipboard.writeText(text);
        setCopiedId(text); // 记录当前复制的 ID
        setTimeout(() => setCopiedId(""), 2000); // 2秒后重置
        // 可以加个简单的提示
    };


    return (
        <div class="settings-page">
            <div class="settings-container">
                <div class="info-header" style="border-bottom: 1px solid #08ddf9; padding-bottom: 10px; margin-bottom: 30px;">
                    <h3>API 服务配置</h3>
                    <p style="color: #666; font-size: 0.9em;">配置 LLM 供应商详情</p>
                </div>

                <div class="settings-form">
                    <div class="setting-item">
                        <label>API 供应商网址 (Endpoint)</label>
                        <input
                            type="text"
                            class="settings-input"
                            placeholder="https://api.openai.com/v1/chat/completions"
                            value={apiUrl()}
                            onInput={(e) => setApiUrl(e.currentTarget.value)}
                        />
                    </div>

                    <div class="setting-item">
                        <label>API Key</label>
                        <input
                            type="password"
                            class="settings-input"
                            placeholder="sk-..."
                            value={apiKey()}
                            onInput={(e) => setApiKey(e.currentTarget.value)}
                        />
                    </div>

                    <div class="setting-item">
                        <label>使用模型名称 (Model ID)</label>
                        <input
                            type="text"
                            class="settings-input"
                            placeholder="gpt-4o / claude-3-opus"
                            value={model()}
                            onInput={(e) => setModel(e.currentTarget.value)}
                        />
                    </div>

                    <button class="save-settings-button" onClick={handleSave}>
                        保存配置
                    </button>

                    <button
                        class="query-models-button"
                        onClick={handleQueryModels}
                        disabled={isLoading()}
                    >
                        {isLoading() ? "查询中..." : "查询可用模型列表"}
                    </button>

                    {saveStatus() && <div class="save-hint">{saveStatus()}</div>}
                </div>
            </div>

            {/* 右侧展示栏 */}
            <div class="models-list-panel">
                <div class="models-header-complex">
                    <div class="models-title-row">
                        <h4>可用模型列表 ({filteredModels().length})</h4>
                    </div>

                    {/* 搜索和过滤工具栏 */}
                    <div class="models-tools-row">
                        <input
                            type="text"
                            placeholder="搜索模型名称..."
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
                                                onClick={() => {
                                                    setSelectedProvider(p);
                                                    setIsDropdownOpen(false);
                                                }}
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

                <div class="models-scroll-area">
                    {models().length === 0 && (
                        <div style="color: #444; text-align: center; margin-top: 50px;">
                            点击左侧“查询模型”按钮获取列表
                        </div>
                    )}
                    <For each={filteredModels()}>
                        {(m) => (
                            <div class="model-card">
                                <div class="model-info">
                                    <span class="model-id">{m.id}</span>
                                    <span class="model-provider">Provider: {m.owned_by || 'Unknown'}</span>
                                </div>
                                <button
                                    class={copiedId() === m.id ? "copy-btn copied" : "copy-btn"}
                                    onClick={() => copyToClipboard(m.id)}
                                >
                                    {copiedId() === m.id ? "已复制 ✓" : "复制 ID"}
                                </button>
                            </div>
                        )}
                    </For>
                </div>
            </div>

        </div>
    );
};

export default Settings;