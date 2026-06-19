/**
 * Provider 列表页 (lobehub v2)
 * 路由: /settings/ (嵌套路由默认页)
 *
 * 结构:
 * 1. 顶部: 本地模型管理块 (从 ProviderSettings.tsx 迁移, 保留全部逻辑)
 * 2. 中部: catalog 元数据统计 (provider/model/version)
 * 3. 底部: provider 列表 (catalog + 自定义), 每行可点进详情
 *
 * 数据流:
 * - provider 列表: catalog.providers + providerConfigs 中的自定义
 * - 列表上的 inline 开关: 仅修改 `enabled` 标志, 详细配置在 ProviderDetail 页
 * - 保存: 写入整个 providerConfigs map (与 ProviderDetail 一致)
 */
import { Component, createSignal, For, Show, onMount, createMemo, onCleanup, createEffect } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { invoke } from '@tauri-apps/api/core';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import {
    providerConfigs,
    setProviderConfigs,
    modelsCatalog,
    modelsCatalogSource,
    modelsCatalogVersion,
    modelsCatalogGeneratedAt,
} from '../store/store';
import {
    updateModelsCatalog,
    getCatalogUrl,
    formatRelativeTime,
    searchProviders,
    loadModelsCatalog,
} from '../utils/models';
import { getProviderLogo } from '../utils/modelLogo';
import type { ProviderConfig, ProviderMeta } from '../utils/models';

// ============== 本地模型子组件 (从 ProviderSettings.tsx 抽出) ==============

interface LocalModel {
    model_id: string;
    owned_by: string;
    api_url: string;
    api_key: string;
    local_path?: string;
    engine_type?: string;
}

const ENGINE_OPTIONS = [
    { id: 'llama_cpp', name: 'llama.cpp', ownedBy: 'Local-llama.cpp', extensions: ['gguf'] },
] as const;

const LocalEngineSection: Component = () => {
    const [localModelPath, setLocalModelPath] = createSignal('');
    const [isLocalRunning, setIsLocalRunning] = createSignal(false);
    const [localActivatedModels, setLocalActivatedModels] = createSignal<LocalModel[]>([]);
    const [localSaveStatus, setLocalSaveStatus] = createSignal('');
    const [enginesStatus, setEnginesStatus] = createSignal<any>(null);

    let pollHandle: number | null = null;

    const refreshLocalStatus = async () => {
        try {
            const running: boolean = await invoke('is_local_server_running');
            setIsLocalRunning(running);
        } catch (e) { /* ignore */ }
    };

    onMount(async () => {
        try {
            const models: LocalModel[] = await invoke('load_activated_models') || [];
            setLocalActivatedModels(models);
        } catch (e) { /* ignore */ }
        try {
            const cfg: any = await invoke('load_app_config');
            if (cfg?.localModelPath) setLocalModelPath(cfg.localModelPath);
        } catch (e) { /* ignore */ }
        try {
            const s = await invoke('get_engines_status');
            setEnginesStatus(s);
        } catch (e) { /* ignore */ }
        refreshLocalStatus();
        pollHandle = window.setInterval(refreshLocalStatus, 3000);
    });

    onCleanup(() => {
        if (pollHandle !== null) clearInterval(pollHandle);
    });

    const pickLocalFile = async () => {
        try {
            const file = await openDialog({
                multiple: false,
                filters: [{ name: 'GGUF', extensions: ['gguf'] }],
            });
            if (file && typeof file === 'string') {
                setLocalModelPath(file);
                setLocalSaveStatus(`已选择: ${file}`);
                setTimeout(() => setLocalSaveStatus(''), 3000);
            }
        } catch (e) {
            alert('选择文件失败: ' + e);
        }
    };

    const addLocalModel = async () => {
        const path = localModelPath();
        if (!path) return alert('请先选择模型文件');
        const engine = ENGINE_OPTIONS[0];
        const fileName = path.split(/[\\/]/).pop() || 'local-model';
        const modelName = fileName.replace(/\.[^/.]+$/, '');
        if (localActivatedModels().find(m => m.local_path === path)) return;
        const newLocal: LocalModel = {
            model_id: modelName,
            owned_by: engine.ownedBy,
            api_url: 'http://127.0.0.1:8080/v1',
            api_key: 'local-no-key',
            local_path: path,
            engine_type: engine.id,
        };
        const newList = [...localActivatedModels(), newLocal];
        setLocalActivatedModels(newList);
        await invoke('save_activated_models', { models: newList });
        setLocalSaveStatus(`已添加本地模型: ${modelName} (${engine.name})`);
        setTimeout(() => setLocalSaveStatus(''), 3000);
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

    return (
        <div class="mb-4 p-4 rounded-lg border border-dark-300" style="background: rgba(18, 22, 35, 0.25); backdrop-filter: blur(30px);">
            <div class="flex items-center justify-between mb-2">
                <h3 class="text-sm font-bold text-white uppercase tracking-wider">🖥 本地推理引擎</h3>
                <Show when={localSaveStatus()}>
                    <span class="text-xs text-pri">{localSaveStatus()}</span>
                </Show>
            </div>
            <div class="text-xs text-[#aaa] mb-3">
                llama.cpp (GGUF 模型) · 当前路径: <span class="font-mono text-[#ccc]">{localModelPath() || '未选择'}</span>
            </div>
            <div class="flex gap-2 flex-wrap mb-3">
                <button
                    class="px-3 py-1.5 text-xs rounded border border-pri-30 bg-pri-10 text-pri hover:bg-pri-20 transition-all"
                    onClick={pickLocalFile}
                >📁 选择模型文件</button>
                <button
                    class="px-3 py-1.5 text-xs rounded border border-pri-30 bg-pri-10 text-pri hover:bg-pri-20 transition-all"
                    onClick={addLocalModel}
                >➕ 添加到模型列表</button>
                <button
                    class="px-3 py-1.5 text-xs rounded text-dark-850 transition-all"
                    style={{ 'background-color': isLocalRunning() ? '#E08090' : 'var(--primary-color)' }}
                    onClick={toggleLocalEngine}
                >
                    {isLocalRunning() ? '⏹ 停止本地推理引擎' : '▶ 启动本地 llama.cpp 引擎'}
                </button>
                <Show when={enginesStatus()}>
                    <span class="text-[10px] text-[#888] self-center ml-auto">
                        {enginesStatus()!.installed ? '✓ 引擎已安装' : '⚠ 引擎未安装, 启动时会自动下载'}
                    </span>
                </Show>
            </div>
            <Show when={localActivatedModels().length > 0}>
                <div class="text-[10px] text-[#888] uppercase tracking-wider mb-1.5">已激活的本地模型 ({localActivatedModels().length})</div>
                <div class="flex flex-wrap gap-1.5">
                    <For each={localActivatedModels()}>
                        {(m) => (
                            <span class="inline-flex items-center gap-1.5 px-2 py-1 rounded bg-pri-20 text-pri text-[11px] font-mono">
                                <span class="truncate max-w-[200px]">{m.model_id}</span>
                                <span class="text-[#888]">({m.owned_by})</span>
                                <button
                                    class="text-pri hover:text-danger text-[14px] leading-none"
                                    title="移除"
                                    onClick={() => removeLocalModel(m)}
                                >×</button>
                            </span>
                        )}
                    </For>
                </div>
            </Show>
        </div>
    );
};

// ============== Catalog 统计 + 同步 ==============

const CatalogStats: Component = () => {
    const [updating, setUpdating] = createSignal(false);
    const [updateResult, setUpdateResult] = createSignal<{ ok: boolean; msg: string } | null>(null);
    const [endpoint, setEndpoint] = createSignal('');

    onMount(async () => {
        try { setEndpoint(await getCatalogUrl()); } catch (e) { /* ignore */ }
    });

    const handleSync = async () => {
        setUpdating(true);
        setUpdateResult(null);
        try {
            const r = await updateModelsCatalog();
            if (r.success) {
                setUpdateResult({ ok: true, msg: `已更新 · v${r.version} · ${r.modelCount} 个模型 · ${(r.bytes / 1024).toFixed(0)} KB` });
            } else {
                setUpdateResult({ ok: false, msg: r.error ?? '更新失败' });
            }
        } catch (e) {
            setUpdateResult({ ok: false, msg: typeof e === 'string' ? e : String(e) });
        } finally {
            setUpdating(false);
            setTimeout(() => setUpdateResult(null), 6000);
        }
    };

    return (
        <div class="mb-4 p-3 rounded-lg border border-dark-300 flex items-center gap-4 flex-wrap" style="background: rgba(18, 22, 35, 0.15);">
            <div class="grow min-w-0">
                <div class="text-[10px] text-[#888] uppercase tracking-wider mb-1">📊 模型元数据库</div>
                <div class="text-xs text-[#ccc]">
                    <Show when={modelsCatalog()}>
                        提供商 <span class="text-pri font-bold">{modelsCatalog()!.providerCount}</span> ·
                        模型 <span class="text-pri font-bold">{modelsCatalog()!.modelCount}</span> ·
                        来源 <span class="text-pri">{modelsCatalogSource()}</span> ·
                        版本 <span class="font-mono">{modelsCatalogVersion() ?? '?'}</span> ·
                        更新 <span>{formatRelativeTime(modelsCatalogGeneratedAt())}</span>
                    </Show>
                </div>
                <Show when={endpoint()}>
                    <div class="text-[10px] text-[#666] font-mono mt-1 truncate" title={endpoint()}>
                        endpoint: {endpoint()}
                    </div>
                </Show>
            </div>
            <div class="flex flex-col items-end gap-1">
                <button
                    class="px-3 py-1.5 text-xs rounded border border-pri-30 bg-pri-10 text-pri hover:bg-pri-20 transition-all disabled:opacity-50"
                    disabled={updating()}
                    onClick={handleSync}
                >
                    {updating() ? '⏳ 同步中...' : '🔄 同步元数据'}
                </button>
                <Show when={updateResult()}>
                    <span
                        class="text-[10px]"
                        classList={{
                            'text-green-400': updateResult()!.ok,
                            'text-red-400': !updateResult()!.ok,
                        }}
                    >{updateResult()!.msg}</span>
                </Show>
            </div>
        </div>
    );
};

// ============== Provider 列表 ==============

const ProviderList: Component = () => {
    const navigate = useNavigate();

    const [search, setSearch] = createSignal('');
    const [toast, setToast] = createSignal<{ msg: string; ok: boolean } | null>(null);

    const [showAddCustom, setShowAddCustom] = createSignal(false);
    const [newCustomName, setNewCustomName] = createSignal('');
    const [newCustomUrl, setNewCustomUrl] = createSignal('');
    const [catalogReady, setCatalogReady] = createSignal(false);

    onMount(async () => {
        if (!modelsCatalog()) await loadModelsCatalog();
        setCatalogReady(true);
    });

    /** Catalog 中的 provider (排除 custom) */
    const catalogProviders = createMemo(() => modelsCatalog()?.providers ?? []);

    /** 用户配置中的 custom providers */
    const customProviders = createMemo(() => {
        return Object.values(providerConfigs()).filter(c => c.isCustom);
    });

    /** 搜索后的 catalog providers */
    const filteredCatalog = createMemo(() => {
        const c = modelsCatalog();
        if (!c) return [] as ProviderMeta[];
        return searchProviders(c, search());
    });

    /** 通用持久化: 把 providers map 写盘 */
    const persist = async (next: Record<string, ProviderConfig>, okMsg?: string) => {
        try {
            await invoke('save_provider_configs', {
                file: { version: 2, updatedAt: String(Date.now()), providers: next },
            });
            setProviderConfigs(next);
            if (okMsg) {
                setToast({ msg: okMsg, ok: true });
                setTimeout(() => setToast(null), 2000);
            }
        } catch (e) {
            setToast({ msg: '保存失败: ' + e, ok: false });
            setTimeout(() => setToast(null), 3000);
        }
    };

    /** 开关切换: 直接写盘, 无需保存按钮 */
    const toggleEnabled = (id: string, currentVal: boolean) => {
        const next = { ...providerConfigs() };
        const cur = next[id];
        if (cur) {
            next[id] = { ...cur, enabled: !currentVal };
        } else {
            const meta = modelsCatalog()?.providers.find(p => p.id === id);
            next[id] = {
                id,
                enabled: !currentVal,
                displayName: meta?.name ?? id,
                apiUrl: '',
                apiKey: '',
                enabledModels: [],
                isCustom: false,
                customModelIds: [],
            };
        }
        persist(next);
    };

    const addCustomProvider = () => {
        const name = newCustomName().trim();
        const url = newCustomUrl().trim();
        if (!name || !url) return alert('名称和 URL 都不能为空');
        const id = 'custom-' + name.toLowerCase().replace(/[^a-z0-9]+/g, '-').slice(0, 32);
        if (providerConfigs()[id]) return alert('已存在同 ID 的 provider');
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
            fetchedModels: undefined,
        };
        const next = { ...providerConfigs(), [id]: newCfg };
        invoke('save_provider_configs', { file: { version: 2, updatedAt: String(Date.now()), providers: next } })
            .then(() => {
                setProviderConfigs(next);
                setShowAddCustom(false);
                setNewCustomName('');
                setNewCustomUrl('');
                setToast({ msg: `已添加自定义 provider "${name}"`, ok: true });
                setTimeout(() => setToast(null), 2000);
            })
            .catch(e => alert('保存失败: ' + e));
    };

    const removeCustomProvider = async (id: string) => {
        if (!confirm(`确认删除自定义 provider "${providerConfigs()[id]?.displayName}"？`)) return;
        const next = { ...providerConfigs() };
        delete next[id];
        try {
            await invoke('save_provider_configs', { file: { version: 2, updatedAt: String(Date.now()), providers: next } });
            setProviderConfigs(next);
        } catch (e) {
            alert('删除失败: ' + e);
        }
    };

    return (
        <div class="h-full overflow-y-auto pr-1">
            <LocalEngineSection />
            <CatalogStats />

            {/* 搜索 */}
            <div class="mb-3 flex items-center gap-3">
                <input
                    type="text"
                    placeholder="🔍 搜索 provider 或模型..."
                    class="flex-1 bg-dark-850 border border-dark-300 text-white px-3 py-1.5 rounded text-sm focus:border-pri outline-none"
                    value={search()}
                    onInput={(e) => setSearch(e.currentTarget.value)}
                />
                <button
                    type="button"
                    class="px-3 py-1.5 text-xs rounded border border-pri-30 bg-pri-10 text-pri hover:bg-pri-20 transition-all"
                    onClick={() => setShowAddCustom(true)}
                >
                    + 添加自定义 Provider
                </button>
            </div>

            {/* 自定义 provider 模态框 */}
            <Show when={showAddCustom()}>
                <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={() => setShowAddCustom(false)}>
                    <div class="bg-dark-900 border border-pri-30 rounded-lg p-5 w-[420px] max-w-[90%]" onClick={(e) => e.stopPropagation()}>
                        <h3 class="text-lg font-bold text-white mb-3">添加自定义 Provider</h3>
                        <div class="mb-3">
                            <label class="block text-xs text-[#aaa] mb-1">显示名称</label>
                            <input
                                type="text"
                                class="w-full bg-dark-850 border border-dark-300 text-white px-3 py-1.5 rounded text-sm"
                                placeholder="My OpenAI Gateway"
                                value={newCustomName()}
                                onInput={(e) => setNewCustomName(e.currentTarget.value)}
                            />
                        </div>
                        <div class="mb-4">
                            <label class="block text-xs text-[#aaa] mb-1">API URL</label>
                            <input
                                type="text"
                                class="w-full bg-dark-850 border border-dark-300 text-white px-3 py-1.5 rounded text-sm font-mono"
                                placeholder="https://my-gateway.example.com/v1"
                                value={newCustomUrl()}
                                onInput={(e) => setNewCustomUrl(e.currentTarget.value)}
                            />
                        </div>
                        <div class="flex justify-end gap-2">
                            <button
                                type="button"
                                class="px-3 py-1.5 text-xs rounded border border-dark-300 text-[#aaa] hover:border-pri-30"
                                onClick={() => { setShowAddCustom(false); setNewCustomName(''); setNewCustomUrl(''); }}
                            >取消</button>
                            <button
                                type="button"
                                class="px-3 py-1.5 text-xs rounded bg-pri text-dark-850 font-medium hover:bg-pri/80"
                                onClick={addCustomProvider}
                            >添加</button>
                        </div>
                    </div>
                </div>
            </Show>

            {/* Provider 列表 (catalog) */}
            <Show when={catalogReady()} fallback={<div class="text-center text-[#888] py-8">加载 catalog 中...</div>}>
                <Show when={filteredCatalog().length === 0}>
                    <div class="text-center text-[#666] py-8 italic text-sm">没有匹配的 provider</div>
                </Show>
                <div class="space-y-1.5">
                    <For each={filteredCatalog()}>
                        {(p) => <ProviderRow provider={p} onToggleEnabled={toggleEnabled} onClick={() => navigate('/settings/provider/' + encodeURIComponent(p.id))} />}
                    </For>
                </div>

                {/* 自定义 provider 区 */}
                <Show when={customProviders().length > 0}>
                    <div class="text-[10px] text-[#888] uppercase tracking-wider mt-5 mb-1.5">
                        自定义 Provider ({customProviders().length})
                    </div>
                    <div class="space-y-1.5">
                        <For each={customProviders()}>
                            {(cfg) => (
                                <div
                                    class="flex items-center gap-3 px-3 py-2 rounded border border-pri-20 bg-pri-10/20 hover:border-pri-30 transition-colors cursor-pointer"
                                    onClick={() => navigate('/settings/provider/' + encodeURIComponent(cfg.id))}
                                >
                                    <div class="w-9 h-9 rounded bg-dark-850 border border-dark-300 flex items-center justify-center text-base shrink-0">
                                        {cfg.displayName.charAt(0).toUpperCase()}
                                    </div>
                                    <div class="grow min-w-0">
                                        <div class="text-sm text-white truncate">{cfg.displayName}</div>
                                        <div class="text-[10px] text-[#888] font-mono truncate">
                                            {cfg.apiUrl || '(未配置)'} · 自定义
                                        </div>
                                    </div>
                                    <span class="text-[10px] text-[#888]">已启用 {cfg.enabledModels.length} 个</span>
                                    <button
                                        type="button"
                                        class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors shrink-0"
                                        classList={{
                                            'bg-pri': cfg.enabled,
                                            'bg-dark-300': !cfg.enabled,
                                        }}
                                        onClick={(e) => { e.stopPropagation(); toggleEnabled(cfg.id, cfg.enabled); }}
                                    >
                                        <span
                                            class="inline-block h-3 w-3 transform rounded-full bg-white transition-transform"
                                            classList={{
                                                'translate-x-5': cfg.enabled,
                                                'translate-x-1': !cfg.enabled,
                                            }}
                                        />
                                    </button>
                                    <button
                                        type="button"
                                        class="px-2 py-1 text-xs rounded border border-danger text-danger hover:bg-danger hover:text-dark-850"
                                        onClick={(e) => { e.stopPropagation(); removeCustomProvider(cfg.id); }}
                                    >删除</button>
                                </div>
                            )}
                        </For>
                    </div>
                </Show>
            </Show>

            {/* Toast 提示 */}
            <Show when={toast()}>
                <div
                    class="fixed bottom-6 left-1/2 -translate-x-1/2 px-4 py-2 rounded text-sm z-50"
                    classList={{
                        'bg-green-500/20 text-green-300': toast()!.ok,
                        'bg-red-500/20 text-red-300': !toast()!.ok,
                    }}
                >{toast()!.msg}</div>
            </Show>
        </div>
    );
};

// 单个 provider 行
const ProviderRow: Component<{
    provider: ProviderMeta;
    onToggleEnabled: (id: string, currentVal: boolean) => void;
    onClick: () => void;
}> = (props) => {
    const cfg = createMemo(() => providerConfigs()[props.provider.id]);
    const isEnabled = () => cfg()?.enabled ?? false;
    const status = createMemo(() => {
        const c = cfg();
        if (!c) return { label: '未配置', color: 'text-[#666] bg-[#333]' };
        if (isEnabled() && c.apiKey) return { label: '已配置', color: 'text-green-300 bg-green-500/20' };
        if (c.apiKey) return { label: '已配置 · 禁用', color: 'text-yellow-300 bg-yellow-500/20' };
        return { label: '未配置', color: 'text-[#888] bg-[#333]' };
    });
    const enabledCount = createMemo(() => cfg()?.enabledModels.length ?? 0);

    return (
        <div
            class="flex items-center gap-3 px-3 py-2 rounded border border-dark-300 hover:border-pri-30 transition-colors cursor-pointer bg-dark-900/40"
            onClick={props.onClick}
        >
            <div class="w-9 h-9 rounded bg-dark-850 border border-dark-300 flex items-center justify-center shrink-0 overflow-hidden">
                {getProviderLogo(props.provider.id)
                    ? <img src={getProviderLogo(props.provider.id)!} alt={props.provider.name} class="w-6 h-6 object-contain" />
                    : <span class="text-base">{props.provider.name.charAt(0).toUpperCase()}</span>
                }
            </div>
            <div class="grow min-w-0">
                <div class="flex items-center gap-2">
                    <span class="text-sm text-white truncate">{props.provider.name}</span>
                    <span class={`text-[9px] px-1.5 py-0.5 rounded ${status().color}`}>{status().label}</span>
                    <Show when={props.provider.isAggregator}>
                        <span class="text-[9px] px-1.5 py-0.5 rounded text-blue-300 bg-blue-500/20">聚合</span>
                    </Show>
                </div>
                <div class="text-[10px] text-[#888] font-mono mt-0.5">
                    {props.provider.modelCount} 个模型
                    <Show when={enabledCount() > 0}>
                        <span class="ml-2 text-pri">· 已启用 {enabledCount()} 个</span>
                    </Show>
                </div>
            </div>
            <button
                type="button"
                class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors shrink-0 focus:outline-none focus:ring-2 focus:ring-pri/50"
                classList={{
                    'bg-pri': isEnabled(),
                    'bg-dark-300': !isEnabled(),
                }}
                onClick={(e) => {
                    e.stopPropagation();
                    props.onToggleEnabled(props.provider.id, isEnabled());
                }}
            >
                <span
                    class="inline-block h-3 w-3 transform rounded-full bg-white transition-transform"
                    classList={{
                        'translate-x-5': isEnabled(),
                        'translate-x-1': !isEnabled(),
                    }}
                />
            </button>
            <span class="text-[#666] text-lg">›</span>
        </div>
    );
};

export default ProviderList;
