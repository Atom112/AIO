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
import Icon from './Icon';
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
        <div class="glass-card mb-4 animate-row">
            <div class="flex items-center justify-between mb-2.5">
                <h3 class="text-sm font-bold text-white tracking-wider flex items-center gap-2">
                    <Icon name="cpu" class="text-pri" size={16} />
                    本地推理引擎
                </h3>
                <Show when={localSaveStatus()}>
                    <span class="text-xs text-pri font-medium animate-row">{localSaveStatus()}</span>
                </Show>
            </div>
            <div class="text-xs text-[#aaa] mb-3">
                llama.cpp (GGUF 模型) · 当前路径: <span class="font-mono text-[#ccc]">{localModelPath() || '未选择'}</span>
            </div>
            <div class="flex gap-2 flex-wrap mb-3">
                <button
                    class="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-pri-30 bg-pri-10 text-pri hover:bg-pri-20 hover:border-pri-50 transition-all duration-200 active:scale-95"
                    onClick={pickLocalFile}
                >
                    <Icon name="folder" size={14} /> 选择模型文件
                </button>
                <button
                    class="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-pri-30 bg-pri-10 text-pri hover:bg-pri-20 hover:border-pri-50 transition-all duration-200 active:scale-95"
                    onClick={addLocalModel}
                >
                    <Icon name="plus" size={14} /> 添加到模型列表
                </button>
                <button
                    class="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md text-dark-850 font-medium transition-all duration-200 active:scale-95"
                    style={{ 'background-color': isLocalRunning() ? '#E08090' : 'var(--primary-color)' }}
                    onClick={toggleLocalEngine}
                >
                    <Show when={isLocalRunning()} fallback={<Icon name="play" size={12} class="text-dark-850" />}>
                        <Icon name="stop" size={12} class="text-dark-850" />
                    </Show>
                    {isLocalRunning() ? '停止本地推理引擎' : '启动本地 llama.cpp 引擎'}
                </button>
                <Show when={enginesStatus()}>
                    <span class="text-[10px] text-[#888] self-center ml-auto flex items-center gap-1">
                        <Show when={enginesStatus()!.installed} fallback={<Icon name="alert-triangle" size={12} class="text-yellow-400" />}>
                            <Icon name="check-circle" size={12} class="text-green-400" />
                        </Show>
                        {enginesStatus()!.installed ? '引擎已安装' : '引擎未安装, 启动时会自动下载'}
                    </span>
                </Show>
            </div>
            <Show when={localActivatedModels().length > 0}>
                <div class="section-label mb-1.5">已激活的本地模型 ({localActivatedModels().length})</div>
                <div class="flex flex-wrap gap-1.5">
                    <For each={localActivatedModels()}>
                        {(m, i) => (
                            <span
                                class="inline-flex items-center gap-1.5 px-2 py-1 rounded-md chip chip-info font-mono animate-row"
                                style={{ "animation-delay": `${i() * 30}ms` }}
                            >
                                <span class="truncate max-w-[200px]">{m.model_id}</span>
                                <span class="text-[#888]">({m.owned_by})</span>
                                <button
                                    class="text-pri hover:text-white hover:bg-white/10 rounded-full w-4 h-4 flex items-center justify-center transition-colors"
                                    title="移除"
                                    onClick={() => removeLocalModel(m)}
                                >
                                    <Icon name="x" size={10} />
                                </button>
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
        <div class="glass-card mb-4 flex items-center gap-4 flex-wrap animate-row" style={{ "animation-delay": "30ms" }}>
            <div class="grow min-w-0">
                <div class="section-label mb-1.5 flex items-center gap-1.5">
                    <Icon name="chart-bar" size={11} class="text-pri" /> 模型元数据库
                </div>
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
            <div class="flex flex-col items-end gap-1.5">
                <button
                    class="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-pri-30 bg-pri-10 text-pri hover:bg-pri-20 hover:border-pri-50 transition-all duration-200 active:scale-95 disabled:opacity-50 disabled:cursor-not-allowed"
                    disabled={updating()}
                    onClick={handleSync}
                >
                    <Show when={updating()} fallback={<Icon name="refresh" size={12} class={updating() ? 'animate-spin' : ''} />}>
                        <Icon name="spinner" size={12} class="animate-spin" />
                    </Show>
                    {updating() ? '同步中...' : '同步元数据'}
                </button>
                <Show when={updateResult()}>
                    <span
                        class="text-[10px] animate-row"
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
            <div class="mb-3 flex items-center gap-2.5 animate-row" style={{ "animation-delay": "60ms" }}>
                <div class="relative flex-1">
                    <Icon name="search" size={14} class="absolute left-3 top-1/2 -translate-y-1/2 text-[#666] pointer-events-none" />
                    <input
                        type="text"
                        placeholder="搜索 provider 或模型..."
                        class="input-glass w-full pl-9 pr-3 py-1.5 text-sm"
                        value={search()}
                        onInput={(e) => setSearch(e.currentTarget.value)}
                    />
                </div>
                <button
                    type="button"
                    class="px-3 py-1.5 text-xs rounded-md border border-pri-30 bg-pri-10 text-pri hover:bg-pri-20 hover:border-pri-50 transition-all duration-200 active:scale-95"
                    onClick={() => setShowAddCustom(true)}
                >
                    + 添加自定义 Provider
                </button>
            </div>

            {/* 自定义 provider 模态框 */}
            <Show when={showAddCustom()}>
                <div class="modal-overlay-glass" onClick={() => setShowAddCustom(false)}>
                    <div class="modal-glass modal-content-glass p-6 w-[440px] max-w-[90%]" onClick={(e) => e.stopPropagation()}>
                        <h3 class="text-lg font-bold text-white mb-1">添加自定义 Provider</h3>
                        <p class="text-xs text-[#888] mb-5">通过 OpenAI-兼容端点接入任何 LLM 服务</p>
                        <div class="mb-3">
                            <label class="block section-label mb-1.5">显示名称</label>
                            <input
                                type="text"
                                class="input-glass w-full px-3 py-2 text-sm"
                                placeholder="My OpenAI Gateway"
                                value={newCustomName()}
                                onInput={(e) => setNewCustomName(e.currentTarget.value)}
                            />
                        </div>
                        <div class="mb-5">
                            <label class="block section-label mb-1.5">API URL</label>
                            <input
                                type="text"
                                class="input-glass w-full px-3 py-2 text-sm font-mono"
                                placeholder="https://my-gateway.example.com/v1"
                                value={newCustomUrl()}
                                onInput={(e) => setNewCustomUrl(e.currentTarget.value)}
                            />
                        </div>
                        <div class="flex justify-end gap-2">
                            <button
                                type="button"
                                class="px-4 py-1.5 text-xs rounded-md border border-white/10 text-[#aaa] hover:border-pri-30 hover:text-white transition-all duration-200"
                                onClick={() => { setShowAddCustom(false); setNewCustomName(''); setNewCustomUrl(''); }}
                            >取消</button>
                            <button
                                type="button"
                                class="px-4 py-1.5 text-xs rounded-md font-semibold transition-all duration-200 active:scale-95"
                                style={{ 'background-color': 'var(--primary-color)', color: '#0e121f' }}
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
                        {(p, i) => (
                            <ProviderRow
                                provider={p}
                                onToggleEnabled={toggleEnabled}
                                onClick={() => navigate('/settings/provider/' + encodeURIComponent(p.id))}
                                style={{ "animation-delay": `${(i() + 1) * 30}ms` }}
                            />
                        )}
                    </For>
                </div>

                {/* 自定义 provider 区 */}
                <Show when={customProviders().length > 0}>
                    <div class="section-label mt-5 mb-2">
                        自定义 Provider ({customProviders().length})
                    </div>
                    <div class="space-y-1.5">
                        <For each={customProviders()}>
                            {(cfg, i) => (
                                <div
                                    class="list-row flex items-center gap-3 px-3 py-2.5 cursor-pointer animate-row"
                                    style={{ "animation-delay": `${(i() + 1) * 30}ms` }}
                                    onClick={() => navigate('/settings/provider/' + encodeURIComponent(cfg.id))}
                                >
                                    <div class="logo-tile font-bold text-[15px]" style={{ color: '#1a1e2c' }}>
                                        {cfg.displayName.charAt(0).toUpperCase()}
                                    </div>
                                    <div class="grow min-w-0">
                                        <div class="text-sm text-white truncate font-medium">{cfg.displayName}</div>
                                        <div class="text-[10px] text-[#888] font-mono truncate mt-0.5">
                                            {cfg.apiUrl || '(未配置)'} · 自定义
                                        </div>
                                    </div>
                                    <span class="text-[10px] text-[#666] hidden sm:inline">已启用 {cfg.enabledModels.length} 个</span>
                                    <button
                                        type="button"
                                        class="toggle-glass"
                                        classList={{ 'on': cfg.enabled }}
                                        onClick={(e) => { e.stopPropagation(); toggleEnabled(cfg.id, cfg.enabled); }}
                                        title={cfg.enabled ? '点击停用' : '点击启用'}
                                    >
                                        <span class="toggle-knob" />
                                    </button>
                                    <button
                                        type="button"
                                        class="px-2.5 py-1 text-[11px] rounded-md border border-danger/40 text-danger hover:bg-danger hover:text-white transition-all duration-200 active:scale-95"
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
                    class="toast-glass fixed bottom-6 left-1/2 z-50"
                    classList={{
                        'text-green-300': toast()!.ok,
                        'text-red-300': !toast()!.ok,
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
    style?: any;
}> = (props) => {
    const cfg = createMemo(() => providerConfigs()[props.provider.id]);
    const isEnabled = () => cfg()?.enabled ?? false;
    const status = createMemo(() => {
        const c = cfg();
        if (!c) return { label: '未配置', cls: 'chip-mute' };
        if (isEnabled() && c.apiKey) return { label: '已配置', cls: 'chip-ok' };
        if (c.apiKey) return { label: '已配置 · 禁用', cls: 'chip-warn' };
        return { label: '未配置', cls: 'chip-mute' };
    });
    const enabledCount = createMemo(() => cfg()?.enabledModels.length ?? 0);

    return (
        <div
            class="list-row flex items-center gap-3 px-3 py-2.5 cursor-pointer animate-row"
            style={props.style}
            onClick={props.onClick}
        >
            <div class="logo-tile">
                {getProviderLogo(props.provider.id)
                    ? <img src={getProviderLogo(props.provider.id)!} alt={props.provider.name} class="w-5 h-5 object-contain" />
                    : <span class="font-bold text-[15px]" style={{ color: '#1a1e2c' }}>{props.provider.name.charAt(0).toUpperCase()}</span>
                }
            </div>
            <div class="grow min-w-0">
                <div class="flex items-center gap-1.5">
                    <span class="text-sm text-white truncate font-medium">{props.provider.name}</span>
                    <span class={`chip ${status().cls}`}>{status().label}</span>
                    <Show when={props.provider.isAggregator}>
                        <span class="chip chip-info">聚合</span>
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
                class="toggle-glass"
                classList={{ 'on': isEnabled() }}
                onClick={(e) => {
                    e.stopPropagation();
                    props.onToggleEnabled(props.provider.id, isEnabled());
                }}
                title={isEnabled() ? '点击停用' : '点击启用'}
            >
                <span class="toggle-knob" />
            </button>
            <span class="text-[#666] text-lg transition-transform duration-200 group-hover:translate-x-0.5">›</span>
        </div>
    );
};

export default ProviderList;
