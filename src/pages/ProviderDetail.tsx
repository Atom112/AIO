/**
 * Provider 详情页 (lobehub v2)
 * 路由: /settings/provider/:providerId
 *
 * 数据流:
 * - 启用状态 / 模型启用列表 / 表单字段: 直接写入 providerConfigs + 磁盘, 无需保存按钮
 * - 开关 (model toggle / provider enabled / orphan remove): 立即 auto-save
 * - 文本字段 (URL/Key/Proxy/displayName): onInput 立即 auto-save
 */
import { Component, createSignal, createMemo, Show, For, onMount } from 'solid-js';
import { useNavigate, useParams } from '@solidjs/router';
import { invoke } from '@tauri-apps/api/core';
import {
    providerConfigs,
    setProviderConfigs,
    modelsCatalog,
} from '../store/store';
import { loadModelsCatalog } from '../utils/models';
import {
    type ProviderConfig,
    type FetchedModel,
    type TestConnectionResult,
    listProviderModels,
} from '../utils/models';
import { getProviderLogo } from '../utils/modelLogo';
import ModelRow from '../components/ModelRow';

type SortKey = 'releaseDesc' | 'nameAsc';

const ProviderDetail: Component = () => {
    const params = useParams<{ providerId: string }>();
    const navigate = useNavigate();

    const providerId = () => decodeURIComponent(params.providerId);
    const isCustom = () => providerId().startsWith('custom-');

    // ===== catalog / 状态 =====
    const [catalogReady, setCatalogReady] = createSignal(false);
    const [toast, setToast] = createSignal<{ msg: string; ok: boolean } | null>(null);

    const [testState, setTestState] = createSignal<{ status: 'idle' | 'testing' | 'ok' | 'fail'; msg?: string; sampleModels?: string[] }>({ status: 'idle' });
    const [fetchState, setFetchState] = createSignal<{ status: 'idle' | 'fetching' | 'ok' | 'fail'; msg?: string; models?: FetchedModel[] }>({ status: 'idle' });

    const [search, setSearch] = createSignal('');
    const [sortKey, setSortKey] = createSignal<SortKey>('releaseDesc');

    onMount(async () => {
        if (!modelsCatalog()) {
            await loadModelsCatalog();
        }
        setCatalogReady(true);
    });

    // ===== derived =====
    const cat = () => modelsCatalog();
    const isCatalogProvider = () => !isCustom() && !!cat()?.providers.find(p => p.id === providerId());

    /** 从 catalog 拿 provider 元数据; 找不到则用 fallback */
    const providerMeta = createMemo(() => {
        const c = cat();
        if (!c) return null;
        if (isCustom()) {
            const cfg = providerConfigs()[providerId()];
            return cfg ? { id: providerId(), name: cfg.displayName, modelCount: c.models.filter(m => m.provider === providerId()).length, isCustom: true } : null;
        }
        return c.providers.find(p => p.id === providerId()) ?? null;
    });

    /** 用户对该 provider 的当前配置 (从 store) */
    const userCfg = () => providerConfigs()[providerId()] ?? null;

    /** 写入磁盘并更新 store. overrides 用于局部修改 (如切换 enabledModels) */
    const persist = async (overrides: Partial<ProviderConfig> = {}) => {
        const cur = userCfg();
        const meta = providerMeta();
        const next: ProviderConfig = {
            id: providerId(),
            enabled: cur?.enabled ?? false,
            displayName: cur?.displayName ?? meta?.name ?? providerId(),
            apiUrl: cur?.apiUrl ?? (meta as any)?.api ?? defaultApiUrl(providerId()),
            apiKey: cur?.apiKey ?? '',
            proxyUrl: cur?.proxyUrl,
            enabledModels: cur?.enabledModels ?? [],
            isCustom: isCustom(),
            customModelIds: cur?.customModelIds ?? [],
            fetchedModels: cur?.fetchedModels,
            ...overrides,
        };
        const map = { ...providerConfigs() };
        if (next.enabled || next.apiKey || next.enabledModels.length > 0 || next.isCustom) {
            map[next.id] = next;
        } else {
            delete map[next.id];
        }
        const file = { version: 2, updatedAt: String(Date.now()), providers: map };
        try {
            await invoke('save_provider_configs', { file });
            setProviderConfigs(map);
        } catch (e) {
            setToast({ msg: '保存失败: ' + e, ok: false });
            setTimeout(() => setToast(null), 3000);
        }
    };

    /** 表格字段 onInput: 立即 auto-save 整个 cfg */
    const updateField = <K extends keyof ProviderConfig>(key: K, value: ProviderConfig[K]) => {
        persist({ [key]: value } as Partial<ProviderConfig>);
    };

    /** 模型启用开关: 立即写盘 */
    const toggleModel = (modelId: string) => {
        const cur = userCfg()?.enabledModels ?? [];
        const next = cur.includes(modelId)
            ? cur.filter(m => m !== modelId)
            : [...cur, modelId];
        persist({ enabledModels: next });
    };

    /** 孤儿模型移除 */
    const removeOrphan = (modelId: string) => {
        const cur = userCfg()?.enabledModels ?? [];
        persist({ enabledModels: cur.filter(m => m !== modelId) });
    };

    /** Provider 启用 toggle */
    const toggleProviderEnabled = (enabled: boolean) => {
        persist({ enabled });
    };

    // ===== 模型分组 (修复 IIFE bug: 必须用 createMemo 保留响应式) =====
    const modelGroups = createMemo(() => {
        const c = cat();
        if (!c || isCustom()) return { enabled: [], available: [], orphans: [] as string[] };
        return listProviderModels(c, providerId(), userCfg()?.enabledModels ?? []);
    });

    const visibleAvailable = createMemo(() => {
        const q = search().trim().toLowerCase();
        let list = modelGroups().available;
        if (q) {
            list = list.filter(m =>
                m.id.toLowerCase().includes(q) ||
                m.displayName.toLowerCase().includes(q) ||
                (m.family ?? '').toLowerCase().includes(q)
            );
        }
        if (sortKey() === 'nameAsc') {
            list = [...list].sort((a, b) => a.displayName.localeCompare(b.displayName));
        }
        return list;
    });

    const visibleEnabled = createMemo(() => {
        const q = search().trim().toLowerCase();
        const list = modelGroups().enabled;
        if (!q) return list;
        return list.filter(m =>
            m.id.toLowerCase().includes(q) ||
            m.displayName.toLowerCase().includes(q)
        );
    });

    const visibleOrphans = createMemo(() => {
        const q = search().trim().toLowerCase();
        const list = modelGroups().orphans;
        if (!q) return list;
        return list.filter(id => id.toLowerCase().includes(q));
    });

    const handleTestConnection = async () => {
        const u = userCfg();
        setTestState({ status: 'testing' });
        try {
            const r = await invoke<TestConnectionResult>('test_provider_connection', {
                apiUrl: u?.apiUrl ?? defaultApiUrl(providerId()),
                apiKey: u?.apiKey ?? '',
                proxyUrl: u?.proxyUrl ?? null,
            });
            if (r.success) {
                setTestState({ status: 'ok', msg: `✓ ${r.modelCount} 个模型 · ${r.elapsedMs}ms`, sampleModels: r.sampleModelIds });
            } else {
                setTestState({ status: 'fail', msg: r.error ?? '失败' });
            }
        } catch (e) {
            setTestState({ status: 'fail', msg: typeof e === 'string' ? e : String(e) });
        }
    };

    const handleFetchModels = async () => {
        const u = userCfg();
        setFetchState({ status: 'fetching' });
        try {
            const r = await invoke<{ success: boolean; models: Array<{ id: string; owned_by: string; display_name?: string; released_at?: string }>; error: string | null; elapsedMs: number }>(
                'fetch_provider_models',
                { apiUrl: u?.apiUrl ?? '', apiKey: u?.apiKey ?? '', proxyUrl: u?.proxyUrl ?? null },
            );
            if (r.success) {
                const incoming: FetchedModel[] = r.models.map(m => ({
                    id: m.id, ownedBy: m.owned_by, displayName: m.display_name, releasedAt: m.released_at,
                }));
                setFetchState({ status: 'ok', msg: `已拉到 ${r.models.length} 个`, models: incoming });
                if (isCustom()) {
                    persist({ fetchedModels: incoming });
                }
            } else {
                setFetchState({ status: 'fail', msg: r.error ?? '失败' });
            }
        } catch (e) {
            setFetchState({ status: 'fail', msg: typeof e === 'string' ? e : String(e) });
        }
    };

    const handleDelete = async () => {
        const u = userCfg();
        if (!u) return;
        if (!confirm(`确认删除 provider "${u.displayName}"？此操作不可撤销。`)) return;
        const next = { ...providerConfigs() };
        delete next[providerId()];
        try {
            await invoke('save_provider_configs', { file: { version: 2, updatedAt: String(Date.now()), providers: next } });
            setProviderConfigs(next);
            navigate('/settings');
        } catch (e) {
            alert('删除失败: ' + e);
        }
    };

    return (
        <div class="h-full overflow-y-auto">
            <div class="max-w-5xl mx-auto p-4 sm:p-6">
                {/* 顶部返回 + 标题 */}
                <div class="flex items-center gap-3 mb-4">
                    <button
                        type="button"
                        class="px-3 py-1.5 text-sm rounded border border-dark-300 hover:border-pri-30 transition-colors flex items-center gap-1"
                        onClick={() => navigate('/settings')}
                    >
                        ← 返回 Provider 列表
                    </button>
                    <Show when={!catalogReady()}>
                        <span class="text-xs text-[#888]">加载 catalog 中...</span>
                    </Show>
                </div>

                {/* Provider header */}
                <div class="flex items-center gap-3 mb-4 pb-4 border-b border-dark-300">
                    <div class="w-12 h-12 rounded-lg bg-dark-850 border border-dark-300 flex items-center justify-center text-xl shrink-0 overflow-hidden">
                        {getProviderLogo(providerId())
                            ? <img src={getProviderLogo(providerId())!} alt={userCfg()?.displayName ?? providerId()} class="w-8 h-8 object-contain" />
                            : <span>{(userCfg()?.displayName ?? providerId()).charAt(0).toUpperCase()}</span>
                        }
                    </div>
                    <div class="grow min-w-0">
                        <h1 class="text-xl font-bold text-white truncate">{userCfg()?.displayName ?? providerMeta()?.name ?? providerId()}</h1>
                        <div class="text-xs text-[#888] font-mono mt-0.5">
                            {isCustom() ? '自定义 provider' : `${modelGroups().enabled.length + modelGroups().available.length} 个模型 (catalog) · 已启用 ${modelGroups().enabled.length}`}
                        </div>
                    </div>
                    <Show when={!isCustom() && providerMeta()}>
                        <Show when={(providerMeta() as any).doc}>
                            <a
                                href={(providerMeta() as any).doc}
                                target="_blank"
                                rel="noopener noreferrer"
                                class="text-xs text-pri hover:underline"
                            >📖 文档</a>
                        </Show>
                    </Show>
                </div>

                {/* 表单: 显示名称 / API URL / API Key / 代理 */}
                <div class="grid grid-cols-1 md:grid-cols-2 gap-3 mb-3">
                    <div>
                        <label class="block text-[10px] text-[#888] uppercase tracking-wider mb-1.5">显示名称</label>
                        <input
                            type="text"
                            class="w-full bg-dark-850 border border-dark-300 text-white px-3 py-1.5 rounded text-sm focus:border-pri outline-none"
                            value={userCfg()?.displayName ?? ''}
                            onInput={(e) => updateField('displayName', e.currentTarget.value)}
                        />
                    </div>
                    <div>
                        <label class="block text-[10px] text-[#888] uppercase tracking-wider mb-1.5">API URL</label>
                        <input
                            type="text"
                            class="w-full bg-dark-850 border border-dark-300 text-white px-3 py-1.5 rounded text-sm font-mono focus:border-pri outline-none"
                            value={userCfg()?.apiUrl ?? ''}
                            onInput={(e) => updateField('apiUrl', e.currentTarget.value)}
                        />
                    </div>
                    <div>
                        <label class="block text-[10px] text-[#888] uppercase tracking-wider mb-1.5">API Key</label>
                        <input
                            type="password"
                            placeholder="sk-..."
                            class="w-full bg-dark-850 border border-dark-300 text-white px-3 py-1.5 rounded text-sm font-mono focus:border-pri outline-none"
                            value={userCfg()?.apiKey ?? ''}
                            onInput={(e) => updateField('apiKey', e.currentTarget.value)}
                        />
                    </div>
                    <div>
                        <label class="block text-[10px] text-[#888] uppercase tracking-wider mb-1.5">
                            代理 URL <span class="text-[#666]">(可选, 例如 http://127.0.0.1:7890)</span>
                        </label>
                        <input
                            type="text"
                            placeholder="留空则不使用代理"
                            class="w-full bg-dark-850 border border-dark-300 text-white px-3 py-1.5 rounded text-sm font-mono focus:border-pri outline-none"
                            value={userCfg()?.proxyUrl ?? ''}
                            onInput={(e) => updateField('proxyUrl', e.currentTarget.value || undefined)}
                        />
                    </div>
                </div>

                {/* 启用 toggle + 操作按钮 */}
                <div class="flex items-center gap-3 mb-3 flex-wrap">
                    <label class="flex items-center gap-2 cursor-pointer">
                        <input
                            type="checkbox"
                            checked={userCfg()?.enabled ?? false}
                            onChange={(e) => toggleProviderEnabled(e.currentTarget.checked)}
                            class="accent-pri w-4 h-4"
                        />
                        <span class="text-sm text-white">启用此 provider</span>
                    </label>
                    <button
                        type="button"
                        class="px-3 py-1.5 text-xs rounded border border-pri-30 bg-pri-10 text-pri hover:bg-pri-20 transition-all disabled:opacity-50"
                        disabled={testState().status === 'testing'}
                        onClick={handleTestConnection}
                    >
                        {testState().status === 'testing' ? '⏳ 测试中...' : '🧪 测试连接'}
                    </button>
                    <button
                        type="button"
                        class="px-3 py-1.5 text-xs rounded border border-pri-30 bg-pri-10 text-pri hover:bg-pri-20 transition-all disabled:opacity-50"
                        disabled={fetchState().status === 'fetching'}
                        onClick={handleFetchModels}
                    >
                        {fetchState().status === 'fetching' ? '⏳ 拉取中...' : '📥 从 API 拉取模型'}
                    </button>
                    <Show when={userCfg()}>
                        <button
                            type="button"
                            class="px-3 py-1.5 text-xs rounded border border-danger bg-transparent text-danger hover:bg-danger hover:text-dark-850 transition-all ml-auto"
                            onClick={handleDelete}
                        >
                            🗑 删除
                        </button>
                    </Show>
                </div>

                {/* 测试/拉取反馈 */}
                <Show when={testState().status !== 'idle'}>
                    <div
                        class="mb-3 px-3 py-2 rounded text-xs"
                        style={{
                            background: testState().status === 'ok' ? 'rgba(74,222,128,0.1)' : 'rgba(248,113,113,0.1)',
                            color: testState().status === 'ok' ? '#4ade80' : (testState().status === 'testing' ? '#aaa' : '#f87171'),
                        }}
                    >
                        {testState().msg}
                        <Show when={testState().sampleModels && testState().sampleModels!.length > 0}>
                            <span class="text-[#888] ml-2">({testState().sampleModels!.slice(0, 3).join(', ')}...)</span>
                        </Show>
                    </div>
                </Show>
                <Show when={fetchState().status !== 'idle'}>
                    <div
                        class="mb-3 px-3 py-2 rounded text-xs"
                        style={{
                            background: fetchState().status === 'ok' ? 'rgba(74,222,128,0.1)' : 'rgba(248,113,113,0.1)',
                            color: fetchState().status === 'ok' ? '#4ade80' : (fetchState().status === 'fetching' ? '#aaa' : '#f87171'),
                        }}
                    >
                        {fetchState().msg}
                    </div>
                </Show>

                {/* ===== 模型列表区 ===== */}
                <Show when={isCustom()}>
                    <div class="mt-6 pt-4 border-t border-dark-300">
                        <div class="text-[10px] text-[#888] uppercase tracking-wider mb-2">
                            自定义模型 ({userCfg()?.fetchedModels?.length ?? 0})
                        </div>
                        <div class="text-xs text-[#888] italic mb-3">
                            自定义 provider 无 catalog 数据, 请通过"从 API 拉取模型"获取列表后再勾选启用
                        </div>
                        <Show when={(userCfg()?.fetchedModels?.length ?? 0) > 0}>
                            <div class="space-y-1">
                                <For each={userCfg()?.fetchedModels ?? []}>
                                    {(m) => (
                                        <ModelRow
                                            meta={{
                                                id: m.id,
                                                provider: providerId(),
                                                providerName: userCfg()?.displayName ?? providerId(),
                                                displayName: m.displayName || m.id,
                                                family: null,
                                                releaseDate: m.releasedAt ?? null,
                                                lastUpdated: null,
                                                knowledgeCutoff: null,
                                                contextWindow: 0,
                                                maxOutputTokens: null,
                                                capabilities: {} as any,
                                                modalities: { input: ['text'], output: ['text'] },
                                                pricing: null,
                                                status: 'active',
                                                deprecationDate: null,
                                                replacedBy: null,
                                                aliases: [],
                                                isAggregator: false,
                                                sources: [],
                                            } as any}
                                            enabled={(userCfg()?.enabledModels ?? []).includes(m.id)}
                                            onToggle={() => toggleModel(m.id)}
                                            showPricing={false}
                                        />
                                    )}
                                </For>
                            </div>
                        </Show>
                    </div>
                </Show>

                <Show when={!isCustom() && isCatalogProvider()}>
                    <div class="mt-6 pt-4 border-t border-dark-300">
                        <div class="flex items-center gap-3 mb-3 flex-wrap">
                            <div class="text-[10px] text-[#888] uppercase tracking-wider">
                                模型列表 ({modelGroups().enabled.length + modelGroups().available.length})
                            </div>
                            <input
                                type="text"
                                placeholder="🔍 搜索模型..."
                                class="bg-dark-850 border border-dark-300 text-white px-3 py-1 rounded text-xs focus:border-pri outline-none grow max-w-xs"
                                value={search()}
                                onInput={(e) => setSearch(e.currentTarget.value)}
                            />
                            <select
                                class="bg-dark-850 border border-dark-300 text-white px-2 py-1 rounded text-xs"
                                value={sortKey()}
                                onChange={(e) => setSortKey(e.currentTarget.value as SortKey)}
                            >
                                <option value="releaseDesc">发布日期 ↓</option>
                                <option value="nameAsc">名称 A-Z</option>
                            </select>
                        </div>

                        {/* 已启用 */}
                        <Show when={visibleEnabled().length > 0}>
                            <div class="text-[10px] text-[#666] uppercase tracking-wider mt-3 mb-1.5">
                                已启用 ({visibleEnabled().length})
                            </div>
                            <div class="space-y-1">
                                <For each={visibleEnabled()}>
                                    {(m) => <ModelRow meta={m} enabled={true} onToggle={() => toggleModel(m.id)} />}
                                </For>
                            </div>
                        </Show>

                        {/* 未启用 */}
                        <Show when={visibleAvailable().length > 0}>
                            <div class="text-[10px] text-[#666] uppercase tracking-wider mt-3 mb-1.5">
                                未启用 ({visibleAvailable().length})
                            </div>
                            <div class="space-y-1">
                                <For each={visibleAvailable()}>
                                    {(m) => <ModelRow meta={m} enabled={false} onToggle={() => toggleModel(m.id)} />}
                                </For>
                            </div>
                        </Show>

                        {/* 孤儿 (catalog 没有但用户启用过) */}
                        <Show when={visibleOrphans().length > 0}>
                            <div class="text-[10px] text-[#666] uppercase tracking-wider mt-3 mb-1.5">
                                未在 catalog 中 ({visibleOrphans().length})
                            </div>
                            <div class="flex flex-wrap gap-1.5">
                                <For each={visibleOrphans()}>
                                    {(mid) => (
                                        <span class="inline-flex items-center gap-1.5 px-2 py-1 rounded bg-pri-20 text-pri text-[11px] font-mono">
                                            <span class="truncate max-w-[200px]">{mid}</span>
                                            <button
                                                type="button"
                                                class="text-pri hover:text-danger text-[14px] leading-none"
                                                title="移除"
                                                onClick={() => removeOrphan(mid)}
                                            >×</button>
                                        </span>
                                    )}
                                </For>
                            </div>
                        </Show>

                        {/* 空状态 */}
                        <Show when={modelGroups().enabled.length === 0 && modelGroups().available.length === 0 && !search()}>
                            <div class="text-xs text-[#666] italic py-4 text-center">
                                catalog 中暂无该 provider 的模型
                            </div>
                        </Show>
                        <Show when={search() && visibleEnabled().length === 0 && visibleAvailable().length === 0 && visibleOrphans().length === 0}>
                            <div class="text-xs text-[#666] italic py-4 text-center">
                                无匹配 "{search()}" 的模型
                            </div>
                        </Show>
                    </div>
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
        </div>
    );
};

function defaultApiUrl(id: string): string {
    const map: Record<string, string> = {
        openai: 'https://api.openai.com/v1',
        anthropic: 'https://api.anthropic.com',
        google: 'https://generativelanguage.googleapis.com/v1beta',
        deepseek: 'https://api.deepseek.com/v1',
        groq: 'https://api.groq.com/openai/v1',
        mistral: 'https://api.mistral.ai/v1',
        xai: 'https://api.x.ai/v1',
        cohere: 'https://api.cohere.ai/v1',
        openrouter: 'https://openrouter.ai/api/v1',
    };
    return map[id] ?? '';
}

export default ProviderDetail;
