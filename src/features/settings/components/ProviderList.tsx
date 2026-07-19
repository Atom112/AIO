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
import Icon from '../../../shared/components/Icon';
import {
    providerConfigs,
    setProviderConfigs,
    modelsCatalog,
    modelsCatalogGeneratedAt,
} from '../../../core/store/store';
import {
    updateModelsCatalog,
    formatRelativeTime,
    searchProviders,
    loadModelsCatalog,
} from '../../../core/utils/models';
import { getProviderLogo } from '../../../core/utils/modelLogo';
import type { ProviderConfig, ProviderMeta } from '../../../core/utils/models';

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
                // vLLM: 用户确认 --trust-remote-code
                let trustRemoteCode = false;
                if (engine.id === 'vllm') {
                    trustRemoteCode = window.confirm(
                        '[!] 安全警告\n\nvLLM 的 --trust-remote-code 选项允许模型仓库中的\n' +
                        'Python 代码以当前用户权限执行。\n\n' +
                        '仅当你信任该模型来源时才启用此选项。\n\n' +
                        '是否启用 --trust-remote-code？'
                    );
                }
                const serverUrl: string = await invoke('start_local_server', {
                    modelPath: localModelPath(),
                    port: 8080,
                    gpuLayers: 99,
                    engineType: engine.id,
                    trustRemoteCode,
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
        <div class="glass-card mb-4 animate-row-in">
            <div class="flex items-center justify-between mb-2.5">
                <h3 class="text-sm font-bold text-white tracking-wider flex items-center gap-2">
                    <Icon name="cpu" class="text-pri" size={16} />
                    本地推理引擎
                </h3>
                <Show when={localSaveStatus()}>
                    <span class="text-xs text-pri font-medium animate-row-in">{localSaveStatus()}</span>
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
                <div class="text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold mb-1.5">已激活的本地模型 ({localActivatedModels().length})</div>
                <div class="flex flex-wrap gap-1.5">
                    <For each={localActivatedModels()}>
                        {(m, i) => (
                            <span
                                class="inline-flex items-center gap-1.5 px-2 py-1 rounded-md inline-flex items-center px-[7px] py-px rounded-full text-[9px] font-semibold tracking-[0.5px] uppercase leading-[1.6] font-mono animate-row-in" style={{ background: 'rgba(var(--primary-rgb), 0.15)', color: 'rgba(var(--primary-rgb), 1)', border: '1px solid rgba(var(--primary-rgb), 0.25)', "animation-delay": `${i() * 30}ms` }}
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
    const [result, setResult] = createSignal<{ ok: boolean; msg: string } | null>(null);

    const handleSync = async () => {
        setUpdating(true);
        setResult(null);
        try {
            const r = await updateModelsCatalog();
            if (r.success) {
                await loadModelsCatalog();
                setResult({ ok: true, msg: `已更新 ${r.modelCount} 个模型` });
            } else {
                setResult({ ok: false, msg: r.error ?? '同步失败' });
            }
        } catch {
            setResult({ ok: false, msg: '网络错误' });
        }
        setUpdating(false);
        setTimeout(() => setResult(null), 3000);
    };

    return (
        <div class="glass-card mb-4 flex items-center justify-between animate-row-in" style={{ "animation-delay": "30ms" }}>
            <span class="text-xl text-white uppercase tracking-[1.5px] font-semibold">模型供应商</span>
            <div class="flex items-center gap-3">
                <Show when={result()}>
                    <span
                        class="text-[10px]"
                        classList={{
                            'text-green-300': result()!.ok,
                            'text-red-300': !result()!.ok,
                        }}
                    >{result()!.msg}</span>
                </Show>
                <span class="text-[10px] text-white/30">
                    更新于{formatRelativeTime(modelsCatalogGeneratedAt())}
                </span>
                <button
                    type="button"
                    class="flex items-center gap-1 px-2 py-1 text-[10px] rounded-md border border-white/10 text-[#aaa] hover:border-pri-30 hover:text-white transition-all duration-200 active:scale-95 disabled:opacity-50"
                    onClick={handleSync}
                    disabled={updating()}
                >
                    <Show when={updating()} fallback={<Icon name="refresh" size={11} />}>
                        <Icon name="spinner" size={11} class="animate-spin" />
                    </Show>
                    {updating() ? '同步中...' : '同步'}
                </button>
            </div>
        </div>
    );
};

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

    /** 搜索后的 catalog providers, 按字母排序, 启用的置顶 */
    const sortedCatalog = createMemo(() => {
        const providers = filteredCatalog();
        const cfgs = providerConfigs();
        const enabled: ProviderMeta[] = [];
        const disabled: ProviderMeta[] = [];
        for (const p of providers) {
            (cfgs[p.id]?.enabled ? enabled : disabled).push(p);
        }
        const byName = (a: ProviderMeta, b: ProviderMeta) => a.name.localeCompare(b.name);
        enabled.sort(byName);
        disabled.sort(byName);
        return { enabled, disabled };
    });

    /** custom providers 按字母排序, 启用的置顶 */
    const sortedCustom = createMemo(() => {
        const providers = customProviders();
        const enabled = providers.filter(c => c.enabled);
        const disabled = providers.filter(c => !c.enabled);
        const byName = (a: ProviderConfig, b: ProviderConfig) => a.displayName.localeCompare(b.displayName);
        enabled.sort(byName);
        disabled.sort(byName);
        return { enabled, disabled };
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
            <div class="border-t border-white/[0.06] my-4" />
            <CatalogStats />

            {/* 搜索 */}
            <div class="mb-3 flex items-center gap-2.5 animate-row-in" style={{ "animation-delay": "60ms" }}>
                <div class="relative flex-1">
                    <Icon name="search" size={14} class="absolute left-3 top-1/2 -translate-y-1/2 text-[#666] pointer-events-none" />
                    <input
                        type="text"
                        placeholder="搜索供应商"
                        class="bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none w-full pl-9 pr-3 py-1.5 text-sm"
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
                <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/55 backdrop-blur" style={{ animation: 'modalOverlayIn 0.2s ease forwards' }} onClick={() => setShowAddCustom(false)}>
                    <div class="bg-[rgba(18,22,35,0.85)] border border-white/[0.08] rounded-2xl shadow-[0_20px_60px_rgba(0,0,0,0.55)] p-6 w-[440px] max-w-[90%]" style={{ backdropFilter: 'blur(60px) saturate(180%)', WebkitBackdropFilter: 'blur(60px) saturate(180%)', animation: 'modalIn 0.25s cubic-bezier(0.2, 0.8, 0.2, 1) forwards' }} onClick={(e) => e.stopPropagation()}>
                        <h3 class="text-lg font-bold text-white mb-1">添加自定义 Provider</h3>
                        <p class="text-xs text-[#888] mb-5">通过 OpenAI-兼容端点接入任何 LLM 服务</p>
                        <div class="mb-3">
                            <label class="block text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold mb-1.5">显示名称</label>
                            <input
                                type="text"
                                class="bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none w-full px-3 py-2 text-sm"
                                placeholder="My OpenAI Gateway"
                                value={newCustomName()}
                                onInput={(e) => setNewCustomName(e.currentTarget.value)}
                            />
                        </div>
                        <div class="mb-5">
                            <label class="block text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold mb-1.5">API URL</label>
                            <input
                                type="text"
                                class="bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none w-full px-3 py-2 text-sm font-mono"
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
                <Show when={sortedCatalog().enabled.length === 0 && sortedCatalog().disabled.length === 0}>
                    <div class="text-center text-[#666] py-8 italic text-sm">没有匹配的 provider</div>
                </Show>

                {/* 已启用 */}
                <Show when={sortedCatalog().enabled.length > 0}>
                    <div class="space-y-1.5">
                        <For each={sortedCatalog().enabled}>
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
                </Show>

                {/* 分隔线 */}
                <Show when={sortedCatalog().enabled.length > 0 && sortedCatalog().disabled.length > 0}>
                    <div class="flex items-center gap-3 my-4">
                        <div class="flex-1 h-px bg-white/[0.06]" />
                        <span class="text-[10px] text-white/30 uppercase tracking-[1.5px] font-semibold shrink-0">未启用</span>
                        <div class="flex-1 h-px bg-white/[0.06]" />
                    </div>
                </Show>

                {/* 未启用 */}
                <Show when={sortedCatalog().disabled.length > 0}>
                    <div class="space-y-1.5">
                        <For each={sortedCatalog().disabled}>
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
                </Show>

                {/* 自定义 provider 区 */}
                <Show when={sortedCustom().enabled.length > 0 || sortedCustom().disabled.length > 0}>
                    <div class="flex items-center gap-3 mt-5 mb-2">
                        <div class="flex-1 h-px bg-white/[0.06]" />
                        <span class="text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold shrink-0">
                            自定义 Provider ({customProviders().length})
                        </span>
                        <div class="flex-1 h-px bg-white/[0.06]" />
                    </div>
                    {/* 已启用 */}
                    <Show when={sortedCustom().enabled.length > 0}>
                        <div class="space-y-1.5">
                            <For each={sortedCustom().enabled}>
                                {(cfg, i) => (
                                    <div
                                        class="relative bg-white/[0.025] border border-white/[0.05] rounded-[10px] transition-all duration-[250ms] hover:bg-pri-5 hover:border-pri hover:translate-x-0.5 hover:shadow-[0_4px_16px_rgba(0,0,0,0.25)] active:translate-x-0.5 active:scale-[0.995] flex items-center gap-3 px-3 py-2.5 cursor-pointer animate-row-in"
                                        style={{ "animation-delay": `${(i() + 1) * 30}ms` }}
                                        onClick={() => navigate('/settings/provider/' + encodeURIComponent(cfg.id))}
                                    >
                                        <div class="flex items-center justify-center w-9 h-9 rounded-lg bg-white border border-white/85 shadow-[0_1px_3px_rgba(0,0,0,0.15)] overflow-hidden shrink-0 transition-[border-color,box-shadow] duration-200 text-[#1a1e2c] font-bold text-[15px]" style={{ color: '#1a1e2c' }}>
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
                                            class="relative inline-flex items-center h-[22px] w-[40px] rounded-full bg-white/[0.08] border border-white/[0.08] cursor-pointer shrink-0 focus:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--primary-rgb),0.25)]"
                                            style={{
                                                transition: 'background 0.3s cubic-bezier(0.4, 0, 0.2, 1), border-color 0.3s ease, box-shadow 0.3s ease',
                                                ...(cfg.enabled ? { background: 'rgba(var(--primary-rgb), 0.7)', 'border-color': 'rgba(var(--primary-rgb), 0.5)', 'box-shadow': '0 0 12px rgba(var(--primary-rgb), 0.35)' } : {})
                                            }}
                                            onClick={(e) => { e.stopPropagation(); toggleEnabled(cfg.id, cfg.enabled); }}
                                            title={cfg.enabled ? '点击停用' : '点击启用'}
                                        >
                                            <span
                                                class="inline-block h-4 w-4 rounded-full bg-white transition-transform duration-300 shadow-[0_2px_6px_rgba(0,0,0,0.4)]"
                                                style={{ transform: cfg.enabled ? 'translateX(21px)' : 'translateX(3px)' }}
                                            />
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
                    {/* 自定义分隔线 */}
                    <Show when={sortedCustom().enabled.length > 0 && sortedCustom().disabled.length > 0}>
                        <div class="flex items-center gap-3 my-3">
                            <div class="flex-1 h-px bg-white/[0.06]" />
                            <span class="text-[10px] text-white/30 uppercase tracking-[1.5px] font-semibold shrink-0">未启用</span>
                            <div class="flex-1 h-px bg-white/[0.06]" />
                        </div>
                    </Show>
                    {/* 未启用 */}
                    <Show when={sortedCustom().disabled.length > 0}>
                        <div class="space-y-1.5">
                            <For each={sortedCustom().disabled}>
                                {(cfg, i) => (
                                    <div
                                        class="relative bg-white/[0.025] border border-white/[0.05] rounded-[10px] transition-all duration-[250ms] hover:bg-pri-5 hover:border-pri hover:translate-x-0.5 hover:shadow-[0_4px_16px_rgba(0,0,0,0.25)] active:translate-x-0.5 active:scale-[0.995] flex items-center gap-3 px-3 py-2.5 cursor-pointer animate-row-in"
                                        style={{ "animation-delay": `${(i() + 1) * 30}ms` }}
                                        onClick={() => navigate('/settings/provider/' + encodeURIComponent(cfg.id))}
                                    >
                                        <div class="flex items-center justify-center w-9 h-9 rounded-lg bg-white border border-white/85 shadow-[0_1px_3px_rgba(0,0,0,0.15)] overflow-hidden shrink-0 transition-[border-color,box-shadow] duration-200 text-[#1a1e2c] font-bold text-[15px]" style={{ color: '#1a1e2c' }}>
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
                                            class="relative inline-flex items-center h-[22px] w-[40px] rounded-full bg-white/[0.08] border border-white/[0.08] cursor-pointer shrink-0 focus:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--primary-rgb),0.25)]"
                                            style={{
                                                transition: 'background 0.3s cubic-bezier(0.4, 0, 0.2, 1), border-color 0.3s ease, box-shadow 0.3s ease',
                                                ...(cfg.enabled ? { background: 'rgba(var(--primary-rgb), 0.7)', 'border-color': 'rgba(var(--primary-rgb), 0.5)', 'box-shadow': '0 0 12px rgba(var(--primary-rgb), 0.35)' } : {})
                                            }}
                                            onClick={(e) => { e.stopPropagation(); toggleEnabled(cfg.id, cfg.enabled); }}
                                            title={cfg.enabled ? '点击停用' : '点击启用'}
                                        >
                                            <span
                                                class="inline-block h-4 w-4 rounded-full bg-white transition-transform duration-300 shadow-[0_2px_6px_rgba(0,0,0,0.4)]"
                                                style={{ transform: cfg.enabled ? 'translateX(21px)' : 'translateX(3px)' }}
                                            />
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
            </Show>

            {/* Toast 提示 */}
            <Show when={toast()}>
                <div
                    class="border border-white/[0.08] rounded-xl px-[18px] py-[10px] text-white text-[13px] font-medium shadow-[0_8px_32px_rgba(0,0,0,0.45)] fixed bottom-6 left-1/2 z-50"
                    classList={{
                        'text-green-300': toast()!.ok,
                        'text-red-300': !toast()!.ok,
                    }}
                    style={{ background: 'rgba(18, 22, 35, 0.88)', backdropFilter: 'blur(30px) saturate(180%)', WebkitBackdropFilter: 'blur(30px) saturate(180%)', animation: 'toastIn 0.3s cubic-bezier(0.175, 0.885, 0.32, 1.275) forwards' }}
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
        if (!c) return { label: '未配置', cls: 'bg-white/5 text-white/40 border border-white/[0.06]' };
        if (isEnabled() && c.apiKey) return { label: '已配置', cls: 'bg-green-400/15 text-green-300 border border-green-400/20' };
        if (c.apiKey) return { label: '已配置 · 禁用', cls: 'bg-yellow-400/[0.12] text-yellow-200 border border-yellow-400/20' };
        return { label: '未配置', cls: 'bg-white/5 text-white/40 border border-white/[0.06]' };
    });
    const enabledCount = createMemo(() => cfg()?.enabledModels.length ?? 0);

    return (
        <div
            class="relative bg-white/[0.025] border border-white/[0.05] rounded-[10px] transition-all duration-[250ms] hover:bg-pri-5 hover:border-pri hover:translate-x-0.5 hover:shadow-[0_4px_16px_rgba(0,0,0,0.25)] active:translate-x-0.5 active:scale-[0.995] flex items-center gap-3 px-3 py-2.5 cursor-pointer animate-row-in"
            style={props.style}
            onClick={props.onClick}
        >
            <div class="flex items-center justify-center w-9 h-9 rounded-lg bg-white border border-white/85 shadow-[0_1px_3px_rgba(0,0,0,0.15)] overflow-hidden shrink-0 transition-[border-color,box-shadow] duration-200 text-[#1a1e2c]">
                {getProviderLogo(props.provider.id)
                    ? <img src={getProviderLogo(props.provider.id)!} alt={props.provider.name} class="w-5 h-5 object-contain" />
                    : <span class="font-bold text-[15px]" style={{ color: '#1a1e2c' }}>{props.provider.name.charAt(0).toUpperCase()}</span>
                }
            </div>
            <div class="grow min-w-0">
                <div class="flex items-center gap-1.5">
                    <span class="text-sm text-white truncate font-medium">{props.provider.name}</span>
                    <span class={`inline-flex items-center px-[7px] py-px rounded-full text-[9px] font-semibold tracking-[0.5px] uppercase leading-[1.6] ${status().cls}`}>{status().label}</span>
                    <Show when={props.provider.isAggregator}>
                        <span class="inline-flex items-center px-[7px] py-px rounded-full text-[9px] font-semibold tracking-[0.5px] uppercase leading-[1.6]" style={{ background: 'rgba(var(--primary-rgb), 0.15)', color: 'rgba(var(--primary-rgb), 1)', border: '1px solid rgba(var(--primary-rgb), 0.25)' }}>聚合</span>
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
                class="relative inline-flex items-center h-[22px] w-[40px] rounded-full bg-white/[0.08] border border-white/[0.08] cursor-pointer shrink-0 focus:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--primary-rgb),0.25)]"
                style={{
                    transition: 'background 0.3s cubic-bezier(0.4, 0, 0.2, 1), border-color 0.3s ease, box-shadow 0.3s ease',
                    ...(isEnabled() ? { background: 'rgba(var(--primary-rgb), 0.7)', 'border-color': 'rgba(var(--primary-rgb), 0.5)', 'box-shadow': '0 0 12px rgba(var(--primary-rgb), 0.35)' } : {})
                }}
                onClick={(e) => {
                    e.stopPropagation();
                    props.onToggleEnabled(props.provider.id, isEnabled());
                }}
                title={isEnabled() ? '点击停用' : '点击启用'}
            >
                <span
                    class="inline-block h-4 w-4 rounded-full bg-white transition-transform duration-300 shadow-[0_2px_6px_rgba(0,0,0,0.4)]"
                    style={{ transform: isEnabled() ? 'translateX(21px)' : 'translateX(3px)' }}
                />
            </button>
            <span class="text-[#666] text-lg transition-transform duration-200 group-hover:translate-x-0.5">›</span>
        </div>
    );
};

export default ProviderList;
