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
} from '../../core/store/store';
import { loadModelsCatalog } from '../../core/utils/models';
import {
    type ProviderConfig,
    type FetchedModel,
    type TestConnectionResult,
    listProviderModels,
} from '../../core/utils/models';
import { getProviderLogo } from '../../core/utils/modelLogo';
import ModelRow from '../../shared/components/ModelRow';
import Icon from '../../shared/components/Icon';
import Dropdown from '../../shared/components/Dropdown';

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
                setTestState({ status: 'ok', msg: `连接成功 · ${r.modelCount} 个模型 · ${r.elapsedMs}ms`, sampleModels: r.sampleModelIds });
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


    return (
        <div class="h-full overflow-y-auto">
            <div class="max-w-5xl mx-auto p-4 sm:p-6">
                {/* 顶部返回 + 标题 */}
                <div class="flex items-center gap-3 mb-5 animate-row">
                    <button
                        type="button"
                        class="px-3 py-1.5 text-sm rounded-md border border-white/10 text-[#ccc] hover:border-pri-30 hover:text-white hover:bg-white/5 transition-all duration-200 active:scale-95 flex items-center gap-1.5"
                        onClick={() => navigate('/settings')}
                    >
                        <Icon name="arrow-left" size={14} class="text-pri" /> 返回 Provider 列表
                    </button>
                    <Show when={!catalogReady()}>
                        <span class="text-xs text-[#888] flex items-center gap-1.5">
                            <span class="w-1.5 h-1.5 rounded-full bg-pri animate-pulse" />
                            加载 catalog 中...
                        </span>
                    </Show>
                </div>

                {/* Provider header card */}
                <div
                    class="glass-card mb-4 flex items-center gap-4 animate-row"
                    style={{ "animation-delay": "30ms" }}
                >
                    <div class="flex items-center justify-center w-12 h-12 rounded-lg bg-white border border-white/85 shadow-[0_1px_3px_rgba(0,0,0,0.15)] overflow-hidden shrink-0 text-[#1a1e2c]">
                        {getProviderLogo(providerId())
                            ? <img src={getProviderLogo(providerId())!} alt={userCfg()?.displayName ?? providerId()} class="w-7 h-7 object-contain" />
                            : <span class="font-bold">{(userCfg()?.displayName ?? providerId()).charAt(0).toUpperCase()}</span>
                        }
                    </div>
                    <div class="grow min-w-0">
                        <h1 class="text-xl font-bold text-white truncate tracking-tight">{userCfg()?.displayName ?? providerMeta()?.name ?? providerId()}</h1>
                        <div class="text-xs text-[#888] font-mono mt-1 flex items-center gap-2 flex-wrap">
                            <Show when={isCustom()}>
                                <span class="inline-flex items-center px-[7px] py-px rounded-full text-[9px] font-semibold tracking-[0.5px] uppercase leading-[1.6]" style={{ background: 'rgba(var(--primary-rgb), 0.15)', color: 'rgba(var(--primary-rgb), 1)', border: '1px solid rgba(var(--primary-rgb), 0.25)' }}>自定义</span>
                            </Show>
                            <Show when={!isCustom()}>
                                <span><span class="text-pri font-semibold">{modelGroups().enabled.length}</span><span class="text-[#666]"> / </span><span>{modelGroups().enabled.length + modelGroups().available.length}</span> 个模型已启用</span>
                                <Show when={modelGroups().enabled.length > 0}>
                                    <span class="text-[#666]">·</span>
                                    <span class="inline-flex items-center px-[7px] py-px rounded-full text-[9px] font-semibold tracking-[0.5px] uppercase leading-[1.6] bg-green-400/15 text-green-300 border border-green-400/20">已启用</span>
                                </Show>
                            </Show>
                        </div>
                    </div>
                    <Show when={!isCustom() && providerMeta()}>
                        <Show when={(providerMeta() as any).doc && /^https?:\/\//i.test((providerMeta() as any).doc)}>
                            <a
                                href={(providerMeta() as any).doc}
                                target="_blank"
                                rel="noopener noreferrer"
                                class="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-white/10 text-[#aaa] hover:border-pri-30 hover:text-pri transition-all duration-200"
                            >
                                <Icon name="book" size={12} /> 文档
                            </a>
                        </Show>
                    </Show>
                </div>

                {/* 表单 + 操作按钮 组合卡片 */}
                <div
                    class="glass-card mb-4 animate-row"
                    style={{ "animation-delay": "60ms" }}
                >
                    <div class="section-label mb-3">连接配置</div>
                    <div class="grid grid-cols-1 md:grid-cols-2 gap-3 mb-4">
                        <div>
                            <label class="block section-label mb-1.5" style={{ 'font-size': '9px' }}>显示名称</label>
                            <input
                                type="text"
                                class="bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none w-full px-3 py-2 text-sm"
                                value={userCfg()?.displayName ?? ''}
                                onInput={(e) => updateField('displayName', e.currentTarget.value)}
                            />
                        </div>
                        <div>
                            <label class="block section-label mb-1.5" style={{ 'font-size': '9px' }}>API URL</label>
                            <input
                                type="text"
                                class="bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none w-full px-3 py-2 text-sm font-mono"
                                value={userCfg()?.apiUrl ?? ''}
                                onInput={(e) => updateField('apiUrl', e.currentTarget.value)}
                            />
                        </div>
                        <div>
                            <label class="block section-label mb-1.5" style={{ 'font-size': '9px' }}>API Key</label>
                            <input
                                type="password"
                                placeholder="sk-..."
                                class="bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none w-full px-3 py-2 text-sm font-mono"
                                value={userCfg()?.apiKey ?? ''}
                                onInput={(e) => updateField('apiKey', e.currentTarget.value)}
                            />
                        </div>
                        <div>
                            <label class="block section-label mb-1.5" style={{ 'font-size': '9px' }}>
                                代理 URL <span class="text-[#666] normal-case tracking-normal font-normal ml-1">(可选, 例如 http://127.0.0.1:7890)</span>
                            </label>
                            <input
                                type="text"
                                placeholder="留空则不使用代理"
                                class="bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none w-full px-3 py-2 text-sm font-mono"
                                value={userCfg()?.proxyUrl ?? ''}
                                onInput={(e) => updateField('proxyUrl', e.currentTarget.value || undefined)}
                            />
                        </div>
                    </div>

                    {/* 启用 toggle + 操作按钮 */}
                    <div class="flex items-center gap-3 flex-wrap pt-3 border-t border-white/5">
                        <button
                            type="button"
                            class="flex items-center gap-2.5 cursor-pointer bg-transparent border-0 p-0"
                            onClick={() => toggleProviderEnabled(!(userCfg()?.enabled ?? false))}
                        >
                            <span
                                class="relative inline-flex items-center h-[22px] w-[40px] rounded-full bg-white/[0.08] border border-white/[0.08] cursor-pointer shrink-0 focus:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--primary-rgb),0.25)]"
                                style={(() => {
                                    const on = userCfg()?.enabled ?? false;
                                    return {
                                        transition: 'background 0.3s cubic-bezier(0.4, 0, 0.2, 1), border-color 0.3s ease, box-shadow 0.3s ease',
                                        ...(on ? { background: 'rgba(var(--primary-rgb), 0.7)', 'border-color': 'rgba(var(--primary-rgb), 0.5)', 'box-shadow': '0 0 12px rgba(var(--primary-rgb), 0.35)' } : {})
                                    };
                                })()}
                            >
                                <span
                                    class="inline-block h-4 w-4 rounded-full bg-white transition-transform duration-300 shadow-[0_2px_6px_rgba(0,0,0,0.4)]"
                                    style={{ transform: (userCfg()?.enabled ?? false) ? 'translateX(21px)' : 'translateX(3px)' }}
                                />
                            </span>
                            <span class="text-sm text-white">启用此 provider</span>
                        </button>
                        <button
                            type="button"
                            class="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-pri-30 bg-pri-10 text-pri hover:bg-pri-20 hover:border-pri-50 transition-all duration-200 active:scale-95 disabled:opacity-50 disabled:cursor-not-allowed"
                            disabled={testState().status === 'testing'}
                            onClick={handleTestConnection}
                        >
                            <Show when={testState().status === 'testing'} fallback={<Icon name="beaker" size={13} />}>
                                <Icon name="spinner" size={13} class="animate-spin" />
                            </Show>
                            {testState().status === 'testing' ? '测试中...' : '测试连接'}
                        </button>
                        <button
                            type="button"
                            class="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-pri-30 bg-pri-10 text-pri hover:bg-pri-20 hover:border-pri-50 transition-all duration-200 active:scale-95 disabled:opacity-50 disabled:cursor-not-allowed"
                            disabled={fetchState().status === 'fetching'}
                            onClick={handleFetchModels}
                        >
                            <Show when={fetchState().status === 'fetching'} fallback={<Icon name="download" size={13} />}>
                                <Icon name="spinner" size={13} class="animate-spin" />
                            </Show>
                            {fetchState().status === 'fetching' ? '拉取中...' : '从 API 拉取模型'}
                        </button>
                    </div>
                </div>

                {/* 测试/拉取反馈 */}
                <Show when={testState().status !== 'idle'}>
                    <div
                        class="flex items-center gap-1.5 px-3 py-2 rounded-lg text-xs border backdrop-blur-[20px] mb-4 animate-row"
                        classList={{
                            'bg-green-400/[0.08] border-green-400/25 text-green-300': testState().status === 'ok',
                            'bg-red-400/[0.08] border-red-400/25 text-red-300': testState().status === 'fail',
                            'bg-pri/5 border-pri text-pri': testState().status === 'testing',
                        }}
                    >
                        <Show when={testState().status === 'testing'} fallback={
                            <Show when={testState().status === 'ok'} fallback={<Icon name="x" size={14} />}>
                                <Icon name="check" size={14} />
                            </Show>
                        }>
                            <Icon name="spinner" size={14} class="animate-spin" />
                        </Show>
                        <span class="flex-1">{testState().msg}</span>
                        <Show when={testState().sampleModels && testState().sampleModels!.length > 0}>
                            <span class="text-[#888] font-mono">({testState().sampleModels!.slice(0, 3).join(', ')}...)</span>
                        </Show>
                    </div>
                </Show>
                <Show when={fetchState().status !== 'idle'}>
                    <div
                        class="flex items-center gap-1.5 px-3 py-2 rounded-lg text-xs border backdrop-blur-[20px] mb-4 animate-row"
                        classList={{
                            'ok': fetchState().status === 'ok',
                            'fail': fetchState().status === 'fail',
                            'test': fetchState().status === 'fetching',
                        }}
                    >
                        <Show when={fetchState().status === 'fetching'} fallback={
                            <Show when={fetchState().status === 'ok'} fallback={<Icon name="x" size={14} />}>
                                <Icon name="check" size={14} />
                            </Show>
                        }>
                            <Icon name="spinner" size={14} class="animate-spin" />
                        </Show>
                        <span class="flex-1">{fetchState().msg}</span>
                    </div>
                </Show>

                {/* ===== 模型列表区 ===== */}
                <Show when={isCustom()}>
                    <div
                        class="glass-card mt-4 animate-row"
                        style={{ "animation-delay": "90ms" }}
                    >
                        <div class="flex items-center justify-between mb-3">
                            <div class="section-label">自定义模型 ({userCfg()?.fetchedModels?.length ?? 0})</div>
                        </div>
                        <div class="text-xs text-[#888] italic mb-3 px-1">
                            自定义 provider 无 catalog 数据, 请通过"从 API 拉取模型"获取列表后再勾选启用
                        </div>
                        <Show when={(userCfg()?.fetchedModels?.length ?? 0) > 0}>
                            <div class="space-y-1.5">
                                <For each={userCfg()?.fetchedModels ?? []}>
                                    {(m, i) => (
                                        <div class="animate-row" style={{ "animation-delay": `${i() * 30}ms` }}>
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
                                        </div>
                                    )}
                                </For>
                            </div>
                        </Show>
                    </div>
                </Show>

                <Show when={!isCustom() && isCatalogProvider()}>
                    <div
                        class="glass-card mt-4 animate-row"
                        style={{ "animation-delay": "90ms" }}
                    >
                        <div class="flex items-center gap-3 mb-3 flex-wrap">
                            <div class="section-label">
                                模型列表 ({modelGroups().enabled.length + modelGroups().available.length})
                            </div>
                            <div class="flex items-center gap-2 ml-auto flex-wrap">
                                <div class="relative">
                                    <Icon name="search" size={12} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-[#666] pointer-events-none" />
                                    <input
                                        type="text"
                                        placeholder="搜索模型..."
                                        class="bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none pl-7 pr-3 py-1 text-xs"
                                        style={{ 'width': '180px' }}
                                        value={search()}
                                        onInput={(e) => setSearch(e.currentTarget.value)}
                                    />
                                </div>
                                <Dropdown
                                    value={sortKey()}
                                    onChange={(v) => setSortKey(v as SortKey)}
                                    options={[
                                        { value: 'releaseDesc', label: '发布日期' },
                                        { value: 'nameAsc', label: '名称 A-Z' },
                                    ]}
                                    class="text-xs" />
                            </div>
                        </div>

                        {/* 已启用 */}
                        <Show when={visibleEnabled().length > 0}>
                            <div class="section-label mt-2 mb-2 flex items-center gap-2">
                                <span class="w-1.5 h-1.5 rounded-full bg-pri" />
                                已启用 ({visibleEnabled().length})
                            </div>
                            <div class="space-y-1.5">
                                <For each={visibleEnabled()}>
                                    {(m, i) => (
                                        <div class="animate-row" style={{ "animation-delay": `${i() * 25}ms` }}>
                                            <ModelRow meta={m} enabled={true} onToggle={() => toggleModel(m.id)} />
                                        </div>
                                    )}
                                </For>
                            </div>
                        </Show>

                        {/* 未启用 */}
                        <Show when={visibleAvailable().length > 0}>
                            <div class="section-label mt-4 mb-2 flex items-center gap-2">
                                <span class="w-1.5 h-1.5 rounded-full bg-[#666]" />
                                未启用 ({visibleAvailable().length})
                            </div>
                            <div class="space-y-1.5">
                                <For each={visibleAvailable()}>
                                    {(m, i) => (
                                        <div class="animate-row" style={{ "animation-delay": `${i() * 25}ms` }}>
                                            <ModelRow meta={m} enabled={false} onToggle={() => toggleModel(m.id)} />
                                        </div>
                                    )}
                                </For>
                            </div>
                        </Show>

                        {/* 孤儿 (catalog 没有但用户启用过) */}
                        <Show when={visibleOrphans().length > 0}>
                            <div class="section-label mt-4 mb-2 flex items-center gap-2">
                                <span class="w-1.5 h-1.5 rounded-full bg-yellow-400/60" />
                                未在 catalog 中 ({visibleOrphans().length})
                            </div>
                            <div class="flex flex-wrap gap-1.5">
                                <For each={visibleOrphans()}>
                                    {(mid, i) => (
                                        <span
                                            class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md inline-flex items-center px-[7px] py-px rounded-full text-[9px] font-semibold tracking-[0.5px] uppercase leading-[1.6] bg-yellow-400/[0.12] text-yellow-200 border border-yellow-400/20 font-mono animate-row"
                                            style={{ "animation-delay": `${i() * 30}ms` }}
                                        >
                                            <span class="truncate max-w-[200px]">{mid}</span>
                                            <button
                                                type="button"
                                                class="w-4 h-4 flex items-center justify-center rounded-full text-[12px] leading-none transition-colors hover:bg-white/15"
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
                            <div class="text-xs text-[#666] italic py-6 text-center">
                                catalog 中暂无该 provider 的模型
                            </div>
                        </Show>
                        <Show when={search() && visibleEnabled().length === 0 && visibleAvailable().length === 0 && visibleOrphans().length === 0}>
                            <div class="text-xs text-[#666] italic py-6 text-center">
                                无匹配 "{search()}" 的模型
                            </div>
                        </Show>
                    </div>
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
