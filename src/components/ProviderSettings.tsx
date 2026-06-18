import { Component, createSignal, For, Show, onMount, createMemo, createEffect, onCleanup } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import {
    selectedModel,
    setSelectedModel,
    providerConfigs,
    setProviderConfigs,
    modelsCatalog,
    modelsCatalogSource,
    modelsCatalogPath,
    modelsCatalogVersion,
    modelsCatalogGeneratedAt,
    setModelsCatalog,
    setModelsCatalogSource,
    setModelsCatalogPath,
    setModelsCatalogVersion,
    setModelsCatalogGeneratedAt,
} from '../store/store';
import {
    findModel,
    formatContextWindow,
    formatPricing,
    updateModelsCatalog,
    getCatalogMeta,
    formatRelativeTime,
} from '../utils/models';
import { getProviderLogo } from '../utils/modelLogo';
import type { ProviderConfig, TestConnectionResult } from '../utils/models';
import type { ModelMeta } from '@aio/models-data';
import Icon from './Icon';

/** 已激活的模型完整配置（含 API 连接信息，仅用于本地模型） */
interface LocalModel {
    model_id: string;
    owned_by: string;
    api_url: string;
    api_key: string;
    local_path?: string;
    engine_type?: string;
}

/** 本地引擎类型定义 */
const ENGINE_OPTIONS = [
    { id: 'llama_cpp', name: 'llama.cpp', ownedBy: 'Local-llama.cpp', extensions: ['gguf'] },
] as const;

/** Provider 列表的展示顺序（决定卡片顺序） */
const PROVIDER_ORDER = [
    'openai', 'anthropic', 'google', 'deepseek',
    'groq', 'mistral', 'xai', 'cohere',
] as const;

/**
 * 拉取失败时的回退默认模型列表（与 Rust `default_providers()` 保持一致）
 * 键为 provider id，值为 (modelId, ownedBy) 元组列表
 */
const DEFAULT_MODELS_BY_PROVIDER: Record<string, Array<{ id: string; ownedBy: string }>> = {
    openai: [
        { id: 'gpt-4o', ownedBy: 'OpenAI' },
        { id: 'gpt-4o-mini', ownedBy: 'OpenAI' },
        { id: 'o3', ownedBy: 'OpenAI' },
        { id: 'o4-mini', ownedBy: 'OpenAI' },
    ],
    anthropic: [
        { id: 'claude-sonnet-4-5', ownedBy: 'Anthropic' },
        { id: 'claude-opus-4-1', ownedBy: 'Anthropic' },
        { id: 'claude-haiku-4-5', ownedBy: 'Anthropic' },
    ],
    google: [
        { id: 'gemini-2.5-pro', ownedBy: 'Google' },
        { id: 'gemini-2.5-flash', ownedBy: 'Google' },
    ],
    deepseek: [
        { id: 'deepseek-v4-pro', ownedBy: 'DeepSeek' },
        { id: 'deepseek-v4-flash', ownedBy: 'DeepSeek' },
    ],
    groq: [
        { id: 'llama-3.3-70b-versatile', ownedBy: 'Groq' },
        { id: 'llama-3.1-8b-instant', ownedBy: 'Groq' },
    ],
    mistral: [
        { id: 'mistral-large-latest', ownedBy: 'Mistral' },
        { id: 'codestral-latest', ownedBy: 'Mistral' },
    ],
    xai: [
        { id: 'grok-4', ownedBy: 'xAI' },
        { id: 'grok-4-fast', ownedBy: 'xAI' },
    ],
    cohere: [
        { id: 'command-a', ownedBy: 'Cohere' },
        { id: 'command-r-plus', ownedBy: 'Cohere' },
    ],
};

/**
 * ProviderSettings 页面 (lobehub 形态)
 * @description 顶部保留本地模型管理；下面是多 provider 卡片列表，每个独立配置。
 */
const ProviderSettings: Component = () => {
    // ===== 本地模型相关状态（保留旧逻辑） =====
    const [localModelPath, setLocalModelPath] = createSignal('');
    const [isLocalRunning, setIsLocalRunning] = createSignal(false);
    const [localActivatedModels, setLocalActivatedModels] = createSignal<LocalModel[]>([]);
    const [localSaveStatus, setLocalSaveStatus] = createSignal('');

    // ===== Provider 卡片相关状态 =====
    const [searchQuery, setSearchQuery] = createSignal('');
    const [expandedIds, setExpandedIds] = createSignal<Set<string>>(new Set());
    const [saving, setSaving] = createSignal(false);
    const [saveBanner, setSaveBanner] = createSignal<{ ok: boolean; msg: string } | null>(null);

    // 临时输入值（不直接绑到全局 signal，避免输入过程触发重渲染）
    const [editingConfigs, setEditingConfigs] = createSignal<Record<string, ProviderConfig>>({});

    // 测试连接 / 拉取模型 的瞬态状态（按 provider id 存）
    const [testStates, setTestStates] = createSignal<Record<string, { status: 'idle' | 'testing' | 'ok' | 'fail'; msg?: string; sampleModels?: string[] }>>({});
    const [fetchStates, setFetchStates] = createSignal<Record<string, { status: 'idle' | 'fetching' | 'ok' | 'fail'; msg?: string; liveModels?: Array<{ id: string; owned_by: string }> }>>({});

    // 添加自定义 provider 的弹窗
    const [showAddCustom, setShowAddCustom] = createSignal(false);
    const [newCustomName, setNewCustomName] = createSignal('');
    const [newCustomUrl, setNewCustomUrl] = createSignal('');

    // ===== 模型元数据库 (catalog) =====
    const [catalogUpdating, setCatalogUpdating] = createSignal(false);
    const [catalogUpdateResult, setCatalogUpdateResult] = createSignal<{ ok: boolean; msg: string } | null>(null);
    const [catalogUrlDisplay, setCatalogUrlDisplay] = createSignal<string>('');

    /**
     * 手动检查并更新模型元数据
     */
    const handleUpdateCatalog = async () => {
        setCatalogUpdating(true);
        setCatalogUpdateResult(null);
        try {
            const result = await updateModelsCatalog();
            if (result.success) {
                setCatalogUpdateResult({
                    ok: true,
                    msg: `已更新 · v${result.version} · ${result.modelCount} 个模型 · ${result.elapsedMs}ms · ${(result.bytes / 1024).toFixed(0)} KB`,
                });
                // 刷新 store 中的元数据
                const cat = modelsCatalog();
                if (cat) {
                    const meta = getCatalogMeta();
                    setModelsCatalog(cat);
                    setModelsCatalogSource(meta.source);
                    setModelsCatalogPath(meta.path);
                    setModelsCatalogVersion(meta.version);
                    setModelsCatalogGeneratedAt(meta.generatedAt);
                }
            } else {
                setCatalogUpdateResult({
                    ok: false,
                    msg: result.error ?? '更新失败',
                });
            }
        } catch (e) {
            setCatalogUpdateResult({
                ok: false,
                msg: typeof e === 'string' ? e : (e instanceof Error ? e.message : '未知错误'),
            });
        } finally {
            setCatalogUpdating(false);
        }
    };

    /** catalog 来源的人类可读描述 */
    const catalogSourceLabel = (): string => {
        switch (modelsCatalogSource()) {
            case 'appdata':       return 'AppData 缓存';
            case 'bundled':       return '应用内置';
            case 'dev_fallback':  return '开发模式';
            case 'empty':         return '无';
        }
    };

    onMount(async () => {
        // 加载本地模型相关
        try {
            const listAct = await invoke<LocalModel[]>('load_activated_models');
            setLocalActivatedModels(listAct);
        } catch { /* 静默 */ }

        try {
            const running = await invoke<boolean>('is_local_server_running');
            setIsLocalRunning(running);
        } catch { /* 静默 */ }

        try {
            const configData: any = await invoke('load_app_config');
            if (configData?.localModelPath) setLocalModelPath(configData.localModelPath);
        } catch { /* 静默 */ }

        // 加载 catalog URL 用于调试展示
        try {
            const u = await invoke<string>('get_catalog_url');
            setCatalogUrlDisplay(u);
        } catch { /* 静默 */ }

        // 加载引擎状态
        try {
            const statuses: any[] = await invoke('get_engines_status');
            const llama = statuses.find((s: any) => s.id === 'llama_cpp');
            const vllm = statuses.find((s: any) => s.id === 'vllm');
            const el = document.getElementById('engine-status');
            if (el) {
                const llamaStatus = llama?.installed ? `● 已安装 ${llama.version || ''}` : '○ 未安装';
                const vllmStatus = vllm?.platform_supported
                    ? (vllm?.installed ? '● 已安装' : '○ 未安装')
                    : '╳ 当前平台不支持';
                el.innerHTML = `
                    <div>llama.cpp: ${llamaStatus}</div>
                    <div>vLLM: ${vllmStatus}</div>
                `;
            }
        } catch { /* 静默 */ }
    });

    // 关键修复：providerConfigs 异步加载完成后，自动同步到 editingConfigs
    // 避免用户进页面时看到空白（onMount 只跑一次）
    let editingInitialized = false;
    createEffect(() => {
        const cfgs = providerConfigs();
        if (Object.keys(cfgs).length === 0) return;
        if (!editingInitialized) {
            setEditingConfigs({ ...cfgs });
            editingInitialized = true;
        }
    });

    // ===== 本地模型相关函数 =====
    const selectModelFile = async () => {
        const file = await open({
            multiple: false,
            filters: [{ name: 'GGUF Model', extensions: ['gguf'] }],
        });
        if (file) {
            setLocalModelPath(file);
            const fileNameWithExt = file.split(/[\\/]/).pop() || 'local-model';
            const modelName = fileNameWithExt.replace(/\.[^/.]+$/, '');
            const engine = ENGINE_OPTIONS[0];
            const newLocal: LocalModel = {
                model_id: modelName,
                owned_by: engine.ownedBy,
                api_url: 'http://127.0.0.1:8080/v1',
                api_key: 'local-no-key',
                local_path: file,
                engine_type: engine.id,
            };
            if (!localActivatedModels().find(m => m.local_path === file)) {
                const newList = [...localActivatedModels(), newLocal];
                setLocalActivatedModels(newList);
                await invoke('save_activated_models', { models: newList });
                setLocalSaveStatus(`已添加本地模型: ${modelName} (${engine.name})`);
                setTimeout(() => setLocalSaveStatus(''), 3000);
            }
        }
    };

    const toggleLocalEngine = async () => {
        if (isLocalRunning()) {
            await invoke('stop_local_server');
            setIsLocalRunning(false);
            setLocalSaveStatus('本地引擎已停止');
        } else {
            if (!localModelPath()) return alert('请先选择模型文件');
            try {
                const currentCfg: any = await invoke('load_app_config');
                await invoke('save_app_config', { config: { ...currentCfg, localModelPath: localModelPath() } });
                setLocalSaveStatus('正在启动本地引擎...');
                const engine = ENGINE_OPTIONS[0];
                const serverUrl: string = await invoke('start_local_server', {
                    modelPath: localModelPath(),
                    port: 8080,
                    gpuLayers: 99,
                    engineType: engine.id,
                });
                setIsLocalRunning(true);
                setLocalSaveStatus('本地引擎已就绪');
                const fullPath = localModelPath();
                const fileNameWithExt = fullPath.split(/[\\/]/).pop() || 'local-model';
                const modelName = fileNameWithExt.replace(/\.[^/.]+$/, '');
                const newLocal: LocalModel = {
                    model_id: modelName,
                    owned_by: engine.ownedBy,
                    api_url: serverUrl,
                    api_key: 'local-no-key',
                    engine_type: engine.id,
                };
                if (!localActivatedModels().some(m => m.model_id === modelName && m.api_url === serverUrl)) {
                    const newList = [...localActivatedModels(), newLocal];
                    setLocalActivatedModels(newList);
                    await invoke('save_activated_models', { models: newList });
                }
                setLocalSaveStatus(`本地模型 ${modelName} 已启动 (${engine.name})`);
            } catch (err) {
                alert('启动失败: ' + err);
                setIsLocalRunning(false);
            }
        }
        setTimeout(() => setLocalSaveStatus(''), 3000);
    };

    const removeLocalModel = async (target: LocalModel) => {
        const newList = localActivatedModels().filter(m => !(m.model_id === target.model_id && m.api_url === target.api_url));
        setLocalActivatedModels(newList);
        await invoke('save_activated_models', { models: newList });
    };

    // ===== Provider 卡片相关函数 =====
    const isDirty = createMemo(() => {
        const cur = providerConfigs();
        const edt = editingConfigs();
        const curIds = Object.keys(cur).sort();
        const edtIds = Object.keys(edt).sort();
        if (curIds.length !== edtIds.length) return true;
        for (const id of curIds) {
            if (!edt[id]) return true;
            const a = cur[id];
            const b = edt[id];
            if (a.enabled !== b.enabled) return true;
            if (a.apiUrl !== b.apiUrl) return true;
            if (a.apiKey !== b.apiKey) return true;
            if (a.displayName !== b.displayName) return true;
            if (a.enabledModels.length !== b.enabledModels.length) return true;
            for (let i = 0; i < a.enabledModels.length; i++) {
                if (a.enabledModels[i] !== b.enabledModels[i]) return true;
            }
        }
        return false;
    });

    const handleSave = async () => {
        setSaving(true);
        setSaveBanner(null);
        try {
            const file = {
                version: 1,
                updatedAt: String(Date.now()),
                providers: editingConfigs(),
            };
            await invoke('save_provider_configs', { file });
            setProviderConfigs(editingConfigs());
            setSaveBanner({ ok: true, msg: '已保存' });
        } catch (e) {
            setSaveBanner({ ok: false, msg: typeof e === 'string' ? e : (e instanceof Error ? e.message : '未知错误') });
        } finally {
            setSaving(false);
            setTimeout(() => setSaveBanner(null), 4000);
        }
    };

    const updateProvider = (id: string, patch: Partial<ProviderConfig>) => {
        const cur = editingConfigs()[id];
        if (!cur) return;
        setEditingConfigs({ ...editingConfigs(), [id]: { ...cur, ...patch } });
    };

    const toggleExpanded = (id: string) => {
        const s = new Set(expandedIds());
        if (s.has(id)) s.delete(id);
        else s.add(id);
        setExpandedIds(s);
    };

    const toggleModelEnabled = (providerId: string, modelId: string) => {
        const cur = editingConfigs()[providerId];
        if (!cur) return;
        const enabled = cur.enabledModels.includes(modelId);
        const newList = enabled
            ? cur.enabledModels.filter(m => m !== modelId)
            : [...cur.enabledModels, modelId];
        updateProvider(providerId, { enabledModels: newList });
    };

    const handleTestConnection = async (cfg: ProviderConfig) => {
        const id = cfg.id;
        setTestStates(s => ({ ...s, [id]: { status: 'testing' } }));
        try {
            const r = await invoke<TestConnectionResult>('test_provider_connection', {
                apiUrl: cfg.apiUrl,
                apiKey: cfg.apiKey,
                proxyUrl: cfg.proxyUrl ?? null,
            });
            if (r.success) {
                setTestStates(s => ({
                    ...s,
                    [id]: { status: 'ok', msg: `✓ ${r.modelCount} 个模型 · ${r.elapsedMs}ms`, sampleModels: r.sampleModelIds },
                }));
            } else {
                setTestStates(s => ({ ...s, [id]: { status: 'fail', msg: r.error ?? '失败' } }));
            }
        } catch (e) {
            setTestStates(s => ({ ...s, [id]: { status: 'fail', msg: typeof e === 'string' ? e : String(e) } }));
        }
    };

    const handleFetchModels = async (cfg: ProviderConfig) => {
        const id = cfg.id;
        setFetchStates(s => ({ ...s, [id]: { status: 'fetching' } }));
        try {
            const r = await invoke<{ success: boolean; models: Array<{ id: string; owned_by: string }>; error: string | null; elapsedMs: number }>(
                'fetch_provider_models',
                { apiUrl: cfg.apiUrl, apiKey: cfg.apiKey, proxyUrl: cfg.proxyUrl ?? null },
            );
            if (r.success) {
                setFetchStates(s => ({
                    ...s,
                    [id]: { status: 'ok', msg: `已拉到 ${r.models.length} 个`, liveModels: r.models },
                }));
                // 合并到 customModelIds
                const cur = editingConfigs()[id];
                if (cur) {
                    const newCustom = Array.from(new Set([...cur.customModelIds, ...r.models.map(m => m.id)]));
                    updateProvider(id, { customModelIds: newCustom });
                }
            } else {
                setFetchStates(s => ({ ...s, [id]: { status: 'fail', msg: r.error ?? '失败' } }));
            }
        } catch (e) {
            setFetchStates(s => ({ ...s, [id]: { status: 'fail', msg: typeof e === 'string' ? e : String(e) } }));
        }
    };

    /**
     * 拉取失败时使用该 provider 的硬编码默认模型列表（与 Rust `default_providers()` 同步）
     */
    const handleUseDefaultModels = (id: string) => {
        const defaults = DEFAULT_MODELS_BY_PROVIDER[id];
        if (!defaults || defaults.length === 0) {
            setFetchStates(s => ({ ...s, [id]: { status: 'fail', msg: '该 provider 无内置默认模型，请手动添加' } }));
            return;
        }
        const cur = editingConfigs()[id];
        if (!cur) return;
        const newCustom = Array.from(new Set([...cur.customModelIds, ...defaults.map(d => d.id)]));
        updateProvider(id, { customModelIds: newCustom });
        setFetchStates(s => ({
            ...s,
            [id]: { status: 'ok', msg: `已使用 ${defaults.length} 个内置默认模型`, liveModels: defaults.map(d => ({ id: d.id, owned_by: d.ownedBy })) },
        }));
    };

    const addCustomProvider = () => {
        const name = newCustomName().trim();
        const url = newCustomUrl().trim();
        if (!name || !url) return alert('名称和 URL 都不能为空');
        const id = 'custom-' + name.toLowerCase().replace(/[^a-z0-9]+/g, '-').slice(0, 32);
        if (editingConfigs()[id]) return alert('已存在同 ID 的 provider');
        const newCfg: ProviderConfig = {
            id,
            enabled: true,
            displayName: name,
            apiUrl: url,
            apiKey: '',
            enabledModels: [],
            isCustom: true,
            customModelIds: [],
            proxyUrl: undefined,
        };
        setEditingConfigs({ ...editingConfigs(), [id]: newCfg });
        setShowAddCustom(false);
        setNewCustomName('');
        setNewCustomUrl('');
        setExpandedIds(s => { const ns = new Set(s); ns.add(id); return ns; });
    };

    const removeCustomProvider = (id: string) => {
        if (!confirm('确认删除该自定义 provider？')) return;
        const next = { ...editingConfigs() };
        delete next[id];
        setEditingConfigs(next);
    };

    // ===== 派生数据 =====
    const allProviders = createMemo(() => {
        const edt = editingConfigs();
        const ids = new Set<string>();
        // 按预设顺序
        for (const id of PROVIDER_ORDER) {
            if (edt[id]) ids.add(id);
        }
        // 自定义 provider 排后
        for (const id of Object.keys(edt)) {
            if (!ids.has(id) && edt[id]?.isCustom) ids.add(id);
        }
        return Array.from(ids);
    });

    const filteredProviderIds = createMemo(() => {
        const q = searchQuery().trim().toLowerCase();
        if (!q) return allProviders();
        const edt = editingConfigs();
        return allProviders().filter(id => {
            const c = edt[id];
            if (!c) return false;
            if (c.displayName.toLowerCase().includes(q)) return true;
            if (c.id.toLowerCase().includes(q)) return true;
            // 搜模型 id
            const cat = findModel(null as any, '', '') ; // 避免未使用警告
            return c.enabledModels.some(m => m.toLowerCase().includes(q)) ||
                   c.customModelIds.some(m => m.toLowerCase().includes(q));
        });
    });

    const getModelLogo = (providerId: string) => {
        return getProviderLogo(providerId);
    };

    // 统计当前激活的 provider 数量
    const enabledCount = createMemo(() => Object.values(editingConfigs()).filter(c => c.enabled).length);
    const totalEnabledModels = createMemo(() => {
        return Object.values(editingConfigs())
            .filter(c => c.enabled)
            .reduce((sum, c) => sum + c.enabledModels.length, 0);
    });

    return (
        <div class="flex flex-col gap-4 h-full overflow-y-auto pr-2">
            {/* 顶部：本地模型管理（保留原逻辑） */}
            <div class="border border-pri rounded-xl bg-pri-5 p-5">
                <div class="border-b border-pri-20 pb-2.5 mb-4">
                    <h3 class="text-base font-bold">🖥 本地模型管理</h3>
                </div>
                <div class="flex flex-col gap-3">
                    <div class="text-sm text-[#aaa]">
                        llama.cpp (GGUF 模型) · 当前路径：<span class="font-mono text-[#ccc]">{localModelPath() || '未选择'}</span>
                    </div>
                    <div class="flex gap-3">
                        <button
                            class="primary-btn p-2.5 flex-1"
                            onClick={selectModelFile}
                        >
                            📁 选择并添加本地模型
                        </button>
                        <button
                            class="p-2.5 flex-1 font-bold rounded-md transition-all duration-200 hover:opacity-80 text-black border-none"
                            style={{ 'background-color': isLocalRunning() ? '#E08090' : 'var(--primary-color)' }}
                            onClick={toggleLocalEngine}
                        >
                            {isLocalRunning() ? '⏹ 停止本地推理引擎' : '▶ 启动本地 llama.cpp 引擎'}
                        </button>
                    </div>

                    <Show when={localSaveStatus()}>
                        <div class="text-sm text-pri animate-pulse text-center">{localSaveStatus()}</div>
                    </Show>

                    <div id="engine-status" class="text-xs text-[#aaa] space-y-1 leading-relaxed px-1">
                        加载中...
                    </div>

                    <div class="flex gap-2">
                        <button
                            class="primary-btn p-2.5 flex-1 text-sm"
                            onClick={async () => {
                                const btn = document.getElementById('btn-install-engine') as HTMLButtonElement;
                                btn.disabled = true;
                                btn.textContent = '⏳ 正在安装...';
                                try {
                                    await invoke('install_engine');
                                    setLocalSaveStatus('llama.cpp 引擎安装/更新完成！');
                                } catch (err) {
                                    alert('安装失败: ' + err);
                                } finally {
                                    btn.disabled = false;
                                    btn.textContent = '🔄 安装/更新引擎';
                                }
                            }}
                        >
                            🔄 安装/更新引擎
                        </button>
                        <button
                            class="p-2.5 flex-1 rounded-md cursor-pointer text-xs border border-pri-30 bg-transparent text-[#aaa] hover:bg-pri-10 transition-all"
                            onClick={async () => {
                                try {
                                    const info: any = await invoke('check_llama_update');
                                    if (info.has_update) {
                                        if (confirm(`新版本可用: ${info.latest_version}\n当前版本: ${info.current_version || '未安装'}\n是否现在安装？`)) {
                                            (document.getElementById('btn-install-engine') as HTMLButtonElement)?.click();
                                        }
                                    } else {
                                        alert(`✅ 已是最新版本: ${info.current_version}`);
                                    }
                                } catch (err) {
                                    alert('检查更新失败: ' + err);
                                }
                            }}
                        >
                            📋 检查引擎更新
                        </button>
                    </div>

                    <Show when={localActivatedModels().length > 0}>
                        <div class="mt-2 border-t border-pri-20 pt-3">
                            <h4 class="text-sm font-bold mb-2">本地模型列表</h4>
                            <For each={localActivatedModels()}>
                                {(m) => (
                                    <div class="flex items-center gap-3 p-2 rounded bg-white/5 mb-1.5">
                                        <span class="font-mono text-sm text-[#eee] grow truncate">{m.model_id}</span>
                                        <span class="text-[10px] text-[#666]">● 本地服务</span>
                                        <button
                                            class="text-xs text-danger border border-danger bg-transparent px-2 py-0.5 rounded hover:bg-danger hover:text-dark-850 transition-all"
                                            onClick={() => removeLocalModel(m)}
                                        >
                                            移除
                                        </button>
                                    </div>
                                )}
                            </For>
                        </div>
                    </Show>
                </div>
            </div>

            {/* 模型元数据库 (从 AppSettings 移动过来) */}
            <div class="border border-pri rounded-xl bg-pri-5 p-5">
                <div class="border-b border-pri-20 pb-2.5 mb-4 flex justify-between items-center">
                    <h3 class="text-base font-bold">📊 模型元数据库</h3>
                    <div class="flex items-center gap-2">
                        <span class="text-xs text-[#888]">来源:</span>
                        <div
                            class="text-xs px-2.5 py-0.5 rounded-full font-medium"
                            style={{
                                background: 'rgba(124,154,191,0.15)',
                                color: 'rgba(255,255,255,0.7)',
                                'font-family': "'JetBrains Mono', monospace",
                            }}
                        >
                            {catalogSourceLabel()}
                        </div>
                    </div>
                </div>

                <div class="grid grid-cols-3 gap-3 mb-4">
                    <div class="bg-white/5 border border-white/10 rounded-lg p-3">
                        <div class="text-[10px] text-[#888] uppercase tracking-wider mb-1">厂商</div>
                        <div class="text-xl font-bold text-white font-mono">
                            {modelsCatalog()?.providerCount ?? 0}
                        </div>
                    </div>
                    <div class="bg-white/5 border border-white/10 rounded-lg p-3">
                        <div class="text-[10px] text-[#888] uppercase tracking-wider mb-1">模型</div>
                        <div class="text-xl font-bold text-white font-mono">
                            {modelsCatalog()?.modelCount ?? 0}
                        </div>
                    </div>
                    <div class="bg-white/5 border border-white/10 rounded-lg p-3">
                        <div class="text-[10px] text-[#888] uppercase tracking-wider mb-1">版本</div>
                        <div
                            class="text-base font-bold text-white font-mono truncate"
                            title={modelsCatalogVersion() ?? ''}
                        >
                            {modelsCatalogVersion() ? `v${modelsCatalogVersion()}` : '—'}
                        </div>
                    </div>
                </div>

                <div class="flex justify-between items-center py-2">
                    <div class="flex-1 min-w-0 pr-4">
                        <span class="block text-[#eee] text-[14px]">检查数据更新</span>
                        <p
                            class="text-xs mt-1"
                            style={{
                                color: (() => {
                                    const r = catalogUpdateResult();
                                    if (!r) return '#777';
                                    return r.ok ? 'var(--primary-color)' : '#d99';
                                })(),
                            }}
                        >
                            {(() => {
                                const r = catalogUpdateResult();
                                if (r) return r.msg;
                                const gen = modelsCatalogGeneratedAt();
                                if (gen) return `数据生成于 ${formatRelativeTime(gen)}`;
                                return '点击右侧按钮从 aio-models-data 拉取最新数据';
                            })()}
                        </p>
                    </div>

                    <button
                        class="flex items-center gap-2 px-4 py-2 rounded-lg cursor-pointer transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed shrink-0"
                        style={{
                            background: 'rgba(var(--primary-rgb), 0.18)',
                            color: 'var(--primary-color)',
                            border: '1px solid rgba(var(--primary-rgb), 0.25)',
                        }}
                        disabled={catalogUpdating()}
                        onClick={handleUpdateCatalog}
                        title="从 aio-models-data 仓库拉取最新模型元数据"
                    >
                        <Icon src="/icons/app-logo/switch-arrows.svg" class="w-4 h-4" />
                        <span class="text-sm font-medium">
                            {catalogUpdating() ? '下载中…' : '检查数据更新'}
                        </span>
                    </button>
                </div>

                <Show when={catalogUrlDisplay()}>
                    <div
                        class="mt-3 px-3 py-2 rounded-lg text-[11px] font-mono leading-relaxed"
                        style={{
                            background: 'rgba(255, 255, 255, 0.03)',
                            color: 'rgba(255, 255, 255, 0.45)',
                            border: '1px solid rgba(255, 255, 255, 0.05)',
                            'word-break': 'break-all',
                        }}
                        title="catalog 数据下载端点 (aio-models-data 仓库 raw)"
                    >
                        <span style="color: rgba(255,255,255,0.3)">source URL:</span> {catalogUrlDisplay()}
                    </div>
                </Show>

                <Show when={modelsCatalogPath()}>
                    <div
                        class="mt-2 px-3 py-2 rounded-lg text-[11px] font-mono leading-relaxed"
                        style={{
                            background: 'rgba(255, 255, 255, 0.03)',
                            color: 'rgba(255, 255, 255, 0.35)',
                            border: '1px solid rgba(255, 255, 255, 0.05)',
                            'word-break': 'break-all',
                        }}
                        title="当前生效的 catalog 本地路径"
                    >
                        <span style="color: rgba(255,255,255,0.25)">local path:</span> {modelsCatalogPath()}
                    </div>
                </Show>
            </div>

            {/* Provider 卡片区 */}
            <div class="border border-pri rounded-xl bg-pri-5 p-5">
                <Show when={enabledCount() === 0}>
                    <div
                        class="mb-4 px-4 py-3 rounded-lg text-sm"
                        style={{
                            background: 'rgba(251, 191, 36, 0.1)',
                            color: 'rgb(252, 211, 77)',
                            border: '1px solid rgba(251, 191, 36, 0.3)',
                        }}
                    >
                        <div class="font-bold mb-1">⚠ 还没有启用任何 provider</div>
                        <div class="text-xs opacity-80 leading-relaxed">
                            切换下方任一 provider 卡片右上角的开关，填写 API Key 后保存。
                            启用后即可在聊天顶部的模型下拉中看到。
                        </div>
                    </div>
                </Show>
                <Show when={enabledCount() > 0 && Object.values(editingConfigs()).some(c => c.enabled && !c.apiKey)}>
                    <div
                        class="mb-4 px-4 py-3 rounded-lg text-sm"
                        style={{
                            background: 'rgba(251, 191, 36, 0.08)',
                            color: 'rgb(252, 211, 77)',
                            border: '1px solid rgba(251, 191, 36, 0.2)',
                        }}
                    >
                        <div class="font-bold mb-1">⚠ 部分 provider 未配置 API Key</div>
                        <div class="text-xs opacity-80 leading-relaxed">
                            展开对应卡片填入 Key 后保存。未配置 Key 的模型在聊天中会调用失败。
                        </div>
                    </div>
                </Show>
                <div class="border-b border-pri-20 pb-2.5 mb-4 flex justify-between items-center">
                    <h3 class="text-base font-bold">☁ 云端模型供应商</h3>
                    <div class="flex items-center gap-3 text-xs text-[#888]">
                        <span>已启用 <span class="text-pri font-bold">{enabledCount()}</span> 个</span>
                        <span>·</span>
                        <span>已选 <span class="text-pri font-bold">{totalEnabledModels()}</span> 个模型</span>
                    </div>
                </div>

                <div class="flex gap-3 mb-4">
                    <input
                        type="text"
                        placeholder="搜索供应商或模型..."
                        class="flex-1 bg-dark-850 border border-dark-300 text-white px-3 py-2 rounded-md outline-none text-sm transition-colors focus:border-pri"
                        onInput={(e) => setSearchQuery(e.currentTarget.value)}
                    />
                    <button
                        class="px-4 py-2 rounded-md text-sm font-medium transition-all"
                        style={{
                            background: 'rgba(var(--primary-rgb), 0.18)',
                            color: 'var(--primary-color)',
                            border: '1px solid rgba(var(--primary-rgb), 0.25)',
                        }}
                        onClick={() => setShowAddCustom(true)}
                    >
                        ➕ 添加自定义
                    </button>
                </div>

                <Show when={showAddCustom()}>
                    <div class="border border-pri-30 rounded-lg p-4 mb-4 bg-pri-5">
                        <h4 class="text-sm font-bold mb-3">添加自定义 OpenAI 兼容 Provider</h4>
                        <div class="flex flex-col gap-2.5">
                            <input
                                type="text"
                                placeholder="显示名称，如：My Azure OpenAI"
                                class="bg-dark-850 border border-dark-300 text-white px-3 py-2 rounded-md outline-none text-sm focus:border-pri"
                                value={newCustomName()}
                                onInput={(e) => setNewCustomName(e.currentTarget.value)}
                            />
                            <input
                                type="text"
                                placeholder="API URL，如：https://my-resource.openai.azure.com/openai/deployments"
                                class="bg-dark-850 border border-dark-300 text-white px-3 py-2 rounded-md outline-none text-sm focus:border-pri font-mono"
                                value={newCustomUrl()}
                                onInput={(e) => setNewCustomUrl(e.currentTarget.value)}
                            />
                            <div class="flex gap-2 justify-end">
                                <button
                                    class="px-3 py-1.5 text-sm rounded border border-pri-30 bg-transparent text-[#aaa] hover:bg-pri-10"
                                    onClick={() => { setShowAddCustom(false); setNewCustomName(''); setNewCustomUrl(''); }}
                                >
                                    取消
                                </button>
                                <button
                                    class="primary-btn px-3 py-1.5 text-sm"
                                    onClick={addCustomProvider}
                                >
                                    添加
                                </button>
                            </div>
                        </div>
                    </div>
                </Show>

                <div class="flex flex-col gap-3">
                    <For each={filteredProviderIds()}>
                        {(id) => {
                            const cfg = () => editingConfigs()[id];
                            const isExpanded = () => expandedIds().has(id);
                            return (
                                <div class="border border-pri-20 rounded-lg overflow-hidden bg-white/3">
                                    {/* 卡片头 */}
                                    <div
                                        class="flex items-center gap-3 p-3 cursor-pointer hover:bg-pri-10 transition-colors"
                                        onClick={() => toggleExpanded(id)}
                                    >
                                        <div class="w-8 h-8 bg-white rounded-full flex items-center justify-center shrink-0 border border-pri-20">
                                            <img src={getModelLogo(id)} alt="logo" class="w-5 h-5 object-contain" />
                                        </div>
                                        <div class="grow min-w-0">
                                            <div class="text-sm font-bold text-[#eee] truncate">{cfg()?.displayName || id}</div>
                                            <div class="text-[10px] text-[#666] font-mono truncate">
                                                {cfg()?.apiUrl || '(未配置)'} · {cfg()?.enabledModels.length ?? 0} 个模型
                                            </div>
                                        </div>
                                        <label class="relative inline-block w-9 h-5 cursor-pointer shrink-0" onClick={(e) => e.stopPropagation()}>
                                            <input
                                                class="opacity-0 w-0 h-0 peer"
                                                type="checkbox"
                                                checked={cfg()?.enabled ?? false}
                                                onChange={(e) => updateProvider(id, { enabled: e.currentTarget.checked })}
                                            />
                                            <span class="absolute inset-0 bg-dark-300 border border-dark-100 rounded-full transition-all duration-300 peer-checked:bg-pri peer-checked:border-pri after:content-[''] after:absolute after:top-0.5 after:left-0.5 after:bg-white after:w-3.5 after:h-3.5 after:rounded-full after:transition-all peer-checked:after:translate-x-4"></span>
                                        </label>
                                        <span class={`text-[#888] transition-transform duration-200 ${isExpanded() ? 'rotate-180' : ''}`}>▼</span>
                                    </div>

                                    {/* 卡片体 */}
                                    <Show when={isExpanded()}>
                                        <div class="border-t border-pri-20 p-4 bg-pri-5">
                                            <div class="grid grid-cols-2 gap-3 mb-4">
                                                <div>
                                                    <label class="block text-[10px] text-[#888] uppercase tracking-wider mb-1.5">显示名称</label>
                                                    <input
                                                        type="text"
                                                        class="w-full bg-dark-850 border border-dark-300 text-white px-3 py-1.5 rounded text-sm font-mono focus:border-pri outline-none"
                                                        value={cfg()?.displayName ?? ''}
                                                        onInput={(e) => updateProvider(id, { displayName: e.currentTarget.value })}
                                                    />
                                                </div>
                                                <div>
                                                    <label class="block text-[10px] text-[#888] uppercase tracking-wider mb-1.5">API URL</label>
                                                    <input
                                                        type="text"
                                                        class="w-full bg-dark-850 border border-dark-300 text-white px-3 py-1.5 rounded text-sm font-mono focus:border-pri outline-none"
                                                        value={cfg()?.apiUrl ?? ''}
                                                        onInput={(e) => updateProvider(id, { apiUrl: e.currentTarget.value })}
                                                    />
                                                </div>
                                            </div>

                                            <div class="mb-4">
                                                <label class="block text-[10px] text-[#888] uppercase tracking-wider mb-1.5">API Key</label>
                                                <input
                                                    type="password"
                                                    placeholder="sk-..."
                                                    class="w-full bg-dark-850 border border-dark-300 text-white px-3 py-1.5 rounded text-sm font-mono focus:border-pri outline-none"
                                                    value={cfg()?.apiKey ?? ''}
                                                    onInput={(e) => updateProvider(id, { apiKey: e.currentTarget.value })}
                                                />
                                            </div>

                                            <div class="mb-4">
                                                <label class="block text-[10px] text-[#888] uppercase tracking-wider mb-1.5">
                                                    代理 URL <span class="text-[#666]">(可选，用于解决国内访问 OpenAI/Google 的网络问题，例如 http://127.0.0.1:7890)</span>
                                                </label>
                                                <input
                                                    type="text"
                                                    placeholder="留空则不使用代理"
                                                    class="w-full bg-dark-850 border border-dark-300 text-white px-3 py-1.5 rounded text-sm font-mono focus:border-pri outline-none"
                                                    value={cfg()?.proxyUrl ?? ''}
                                                    onInput={(e) => updateProvider(id, { proxyUrl: e.currentTarget.value || undefined })}
                                                />
                                            </div>

                                            {/* 操作按钮区 */}
                                            <div class="flex gap-2 mb-4 flex-wrap">
                                                <button
                                                    class="px-3 py-1.5 text-xs rounded border border-pri-30 bg-pri-10 text-pri hover:bg-pri-20 transition-all disabled:opacity-50"
                                                    disabled={testStates()[id]?.status === 'testing'}
                                                    onClick={() => cfg() && handleTestConnection(cfg())}
                                                >
                                                    {testStates()[id]?.status === 'testing' ? '⏳ 测试中...' : '🧪 测试连接'}
                                                </button>
                                                <button
                                                    class="px-3 py-1.5 text-xs rounded border border-pri-30 bg-pri-10 text-pri hover:bg-pri-20 transition-all disabled:opacity-50"
                                                    disabled={fetchStates()[id]?.status === 'fetching'}
                                                    onClick={() => cfg() && handleFetchModels(cfg())}
                                                >
                                                    {fetchStates()[id]?.status === 'fetching' ? '⏳ 拉取中...' : '📥 从 API 拉取模型'}
                                                </button>
                                                <Show when={cfg()?.isCustom}>
                                                    <button
                                                        class="px-3 py-1.5 text-xs rounded border border-danger bg-transparent text-danger hover:bg-danger hover:text-dark-850 transition-all"
                                                        onClick={() => removeCustomProvider(id)}
                                                    >
                                                        🗑 删除
                                                    </button>
                                                </Show>
                                            </div>

                                            <Show when={testStates()[id]}>
                                                <div
                                                    class="mb-3 px-3 py-2 rounded text-xs"
                                                    style={{
                                                        background: testStates()[id]?.status === 'ok' ? 'rgba(74,222,128,0.1)' : 'rgba(248,113,113,0.1)',
                                                        color: testStates()[id]?.status === 'ok' ? '#4ade80' : '#f87171',
                                                    }}
                                                >
                                                    {testStates()[id]?.msg}
                                                    <Show when={testStates()[id]?.sampleModels && testStates()[id]!.sampleModels!.length > 0}>
                                                        <span class="text-[#888] ml-2">
                                                            ({testStates()[id]!.sampleModels!.slice(0, 3).join(', ')}...)
                                                        </span>
                                                    </Show>
                                                </div>
                                            </Show>

                                            <Show when={fetchStates()[id]}>
                                                <div
                                                    class="mb-3 px-3 py-2 rounded text-xs"
                                                    style={{
                                                        background: fetchStates()[id]?.status === 'ok' ? 'rgba(74,222,128,0.1)' : 'rgba(248,113,113,0.1)',
                                                        color: fetchStates()[id]?.status === 'ok' ? '#4ade80' : '#f87171',
                                                    }}
                                                >
                                                    {fetchStates()[id]?.msg}
                                                </div>
                                            </Show>

                                            {/* 拉取失败时显示回退按钮 */}
                                            <Show when={fetchStates()[id]?.status === 'fail'}>
                                                <button
                                                    class="mb-3 px-3 py-1.5 text-xs rounded border border-yellow-500 bg-transparent text-yellow-300 hover:bg-yellow-500 hover:text-dark-850 transition-all"
                                                    onClick={() => handleUseDefaultModels(id)}
                                                >
                                                    ⚡ 使用内置默认模型列表
                                                </button>
                                            </Show>

                                            {/* 模型列表 */}
                                            <div>
                                                <div class="text-[10px] text-[#888] uppercase tracking-wider mb-2">
                                                    启用模型 <span class="text-pri">({cfg()?.enabledModels.length ?? 0})</span>
                                                </div>

                                                <Show when={(cfg()?.customModelIds.length ?? 0) > 0}>
                                                    <details class="mb-3">
                                                        <summary class="text-xs text-[#888] cursor-pointer hover:text-[#aaa] py-1">
                                                            📋 从 API 拉取的模型 ({cfg()?.customModelIds.length}) — 勾选以启用
                                                        </summary>
                                                        <div class="mt-2 max-h-48 overflow-y-auto space-y-1 pr-1">
                                                            <For each={cfg()?.customModelIds ?? []}>
                                                                {(mid) => (
                                                                    <label class="flex items-center gap-2 px-2 py-1 rounded hover:bg-pri-10 cursor-pointer">
                                                                        <input
                                                                            type="checkbox"
                                                                            checked={cfg()?.enabledModels.includes(mid) ?? false}
                                                                            onChange={() => toggleModelEnabled(id, mid)}
                                                                            class="accent-pri"
                                                                        />
                                                                        <span class="font-mono text-xs text-[#ccc] grow truncate">{mid}</span>
                                                                    </label>
                                                                )}
                                                            </For>
                                                        </div>
                                                    </details>
                                                </Show>

                                                <Show when={(cfg()?.enabledModels.length ?? 0) > 0}>
                                                    <div class="flex flex-wrap gap-1.5">
                                                        <For each={cfg()?.enabledModels ?? []}>
                                                            {(mid) => (
                                                                <span class="inline-flex items-center gap-1.5 px-2 py-1 rounded bg-pri-20 text-pri text-[11px] font-mono">
                                                                    <span class="truncate max-w-[200px]">{mid}</span>
                                                                    <button
                                                                        class="text-pri hover:text-danger text-[14px] leading-none shrink-0"
                                                                        title="移除"
                                                                        onClick={() => toggleModelEnabled(id, mid)}
                                                                    >
                                                                        ×
                                                                    </button>
                                                                </span>
                                                            )}
                                                        </For>
                                                    </div>
                                                </Show>

                                                <Show when={(cfg()?.enabledModels.length ?? 0) === 0}>
                                                    <div class="text-[10px] text-[#666] italic py-2">
                                                        未启用任何模型 — 该 provider 配置后不会出现在聊天模型选择中
                                                    </div>
                                                </Show>
                                            </div>
                                        </div>
                                    </Show>
                                </div>
                            );
                        }}
                    </For>

                    <Show when={filteredProviderIds().length === 0}>
                        <div class="text-center text-[#666] py-8 italic text-sm">
                            没有匹配的供应商
                        </div>
                    </Show>
                </div>

                {/* 底部保存栏 */}
                <div class="mt-5 pt-4 border-t border-pri-20 flex items-center justify-between gap-3">
                    <Show when={saveBanner()}>
                        <div
                            class="text-sm"
                            style={{ color: saveBanner()!.ok ? 'var(--primary-color)' : '#f87171' }}
                        >
                            {saveBanner()!.msg}
                        </div>
                    </Show>
                    <div class="grow"></div>
                    <button
                        class="primary-btn px-6 py-2.5 disabled:opacity-50"
                        disabled={!isDirty() || saving()}
                        onClick={handleSave}
                    >
                        {saving() ? '保存中…' : '保存所有配置'}
                    </button>
                </div>
            </div>
        </div>
    );
};

export default ProviderSettings;
