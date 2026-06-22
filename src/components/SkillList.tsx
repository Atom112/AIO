import { Component, For, Show, createMemo, createSignal, onMount } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { datas, saveSingleAssistantToBackend, setDatas, setSkills, skills } from '../store/store';
import type { MarketSkill, SkillConfig, SkillMarketCategory } from '../types/skill';

type MarketSort = 'all' | 'trending' | 'hot';
type ViewMode = 'market' | 'downloaded';

const MARKET_CACHE_KEY = 'aio-skill-market-cache-v1';
const MARKET_CACHE_TTL_MS = 6 * 60 * 60 * 1000;

interface SkillMarketCache {
    updatedAt: number;
    categories: SkillMarketCategory[];
    markets: Partial<Record<MarketSort, MarketSkill[]>>;
    marketUpdatedAt: Partial<Record<MarketSort, number>>;
}

const readMarketCache = (): SkillMarketCache | null => {
    try {
        const raw = localStorage.getItem(MARKET_CACHE_KEY);
        if (!raw) return null;
        const cache = JSON.parse(raw) as SkillMarketCache;
        if (!cache.updatedAt || !Array.isArray(cache.categories) || !cache.markets) return null;
        cache.marketUpdatedAt ??= { all: cache.updatedAt };
        return cache;
    } catch {
        return null;
    }
};

const writeMarketCache = (cache: SkillMarketCache) => {
    try {
        localStorage.setItem(MARKET_CACHE_KEY, JSON.stringify(cache));
    } catch {
        // 缓存失败不应阻塞市场浏览。
    }
};

const emptySkill = (): SkillConfig => ({
    id: `skill-${Date.now().toString(36)}`,
    name: '',
    description: '',
    content: '',
});

const formatInstalls = (value: number): string => {
    if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
    if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
    return String(value);
};

const SkillList: Component = () => {
    const initialCache = readMarketCache();
    const [view, setView] = createSignal<ViewMode>('market');
    const [marketSkills, setMarketSkills] = createSignal<MarketSkill[]>(initialCache?.markets.all ?? []);
    const [categories, setCategories] = createSignal<SkillMarketCategory[]>(initialCache?.categories ?? []);
    const [sort, setSort] = createSignal<MarketSort>('all');
    const [category, setCategory] = createSignal('all');
    const [query, setQuery] = createSignal('');
    const [loading, setLoading] = createSignal(!initialCache?.markets.all);
    const [refreshing, setRefreshing] = createSignal(false);
    const [lastRefreshedAt, setLastRefreshedAt] = createSignal<Date | null>(
        initialCache ? new Date(initialCache.marketUpdatedAt.all ?? initialCache.updatedAt) : null,
    );
    const [downloadingId, setDownloadingId] = createSignal<string | null>(null);
    const [editing, setEditing] = createSignal<SkillConfig | null>(null);
    const [isCreating, setIsCreating] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    const loadMarket = async (
        nextSort = sort(),
        nextCategory = category(),
        forceRefresh = false,
    ) => {
        setLoading(true);
        setError(null);
        try {
            const cache = readMarketCache();
            if (!forceRefresh && nextCategory === 'all') {
                const cachedList = cache?.markets[nextSort];
                const cachedAt = cache?.marketUpdatedAt[nextSort] ?? 0;
                const cacheIsFresh = Date.now() - cachedAt < MARKET_CACHE_TTL_MS;
                if (cachedList && cacheIsFresh) {
                    setMarketSkills(cachedList);
                    setLastRefreshedAt(new Date(cachedAt));
                    return;
                }
            }
            const list = await invoke<MarketSkill[]>('list_skill_market', {
                sort: nextSort,
                category: nextCategory === 'all' ? null : nextCategory,
                forceRefresh,
            });
            setMarketSkills(list);
            if (nextCategory === 'all') {
                const updatedAt = Date.now();
                writeMarketCache({
                    updatedAt,
                    categories: categories(),
                    markets: { ...(cache?.markets ?? {}), [nextSort]: list },
                    marketUpdatedAt: { ...(cache?.marketUpdatedAt ?? {}), [nextSort]: updatedAt },
                });
                setLastRefreshedAt(new Date(updatedAt));
            }
        } catch (e) {
            setError(`加载 skills.sh 失败: ${e}`);
        } finally {
            setLoading(false);
        }
    };

    onMount(async () => {
        const cacheIsFresh = initialCache
            && Date.now() - (initialCache.marketUpdatedAt.all ?? 0) < MARKET_CACHE_TTL_MS
            && Date.now() - initialCache.updatedAt < MARKET_CACHE_TTL_MS;
        setLoading(!initialCache?.markets.all);
        try {
            const localList = await invoke<SkillConfig[]>('list_skills');
            setSkills(Object.fromEntries(localList.map(skill => [skill.id, skill])));
            if (cacheIsFresh) return;

            const [categoryList, marketList] = await Promise.all([
                invoke<SkillMarketCategory[]>('list_skill_market_categories', { forceRefresh: false }),
                invoke<MarketSkill[]>('list_skill_market', {
                    sort: 'all',
                    category: null,
                    forceRefresh: false,
                }),
            ]);
            setCategories(categoryList);
            setMarketSkills(marketList);
            const updatedAt = Date.now();
            writeMarketCache({
                updatedAt,
                categories: categoryList,
                markets: { ...(initialCache?.markets ?? {}), all: marketList },
                marketUpdatedAt: { ...(initialCache?.marketUpdatedAt ?? {}), all: updatedAt },
            });
            setLastRefreshedAt(new Date(updatedAt));
        } catch (e) {
            setError(`加载 Skill 市场失败: ${e}`);
        } finally {
            setLoading(false);
        }
    });

    const filteredMarketSkills = createMemo(() => {
        const keyword = query().trim().toLowerCase();
        if (!keyword) return marketSkills();
        return marketSkills().filter(skill =>
            `${skill.name} ${skill.description} ${skill.owner} ${skill.repo}`
                .toLowerCase()
                .includes(keyword),
        );
    });

    const filteredLocalSkills = createMemo(() => {
        const keyword = query().trim().toLowerCase();
        const list = Object.values(skills()).sort((a, b) => a.name.localeCompare(b.name));
        if (!keyword) return list;
        return list.filter(skill =>
            `${skill.name} ${skill.description} ${skill.sourceOwner ?? ''} ${skill.sourceRepo ?? ''}`
                .toLowerCase()
                .includes(keyword),
        );
    });

    const selectSort = (value: MarketSort) => {
        setSort(value);
        void loadMarket(value, category());
    };

    const selectCategory = (value: string) => {
        setCategory(value);
        void loadMarket(sort(), value);
    };

    const refreshMarket = async () => {
        setRefreshing(true);
        setError(null);
        try {
            const [categoryList, marketList] = await Promise.all([
                invoke<SkillMarketCategory[]>('list_skill_market_categories', { forceRefresh: true }),
                invoke<MarketSkill[]>('list_skill_market', {
                    sort: sort(),
                    category: category() === 'all' ? null : category(),
                    forceRefresh: true,
                }),
            ]);
            setCategories(categoryList);
            setMarketSkills(marketList);
            const updatedAt = Date.now();
            const cache = readMarketCache();
            writeMarketCache({
                updatedAt,
                categories: categoryList,
                markets: { ...(cache?.markets ?? {}), [sort()]: marketList },
                marketUpdatedAt: { ...(cache?.marketUpdatedAt ?? {}), [sort()]: updatedAt },
            });
            setLastRefreshedAt(new Date(updatedAt));
        } catch (e) {
            setError(`更新 Skill 列表失败: ${e}`);
        } finally {
            setRefreshing(false);
        }
    };

    const download = async (skill: MarketSkill) => {
        setDownloadingId(skill.id);
        setError(null);
        try {
            const downloaded = await invoke<SkillConfig>('download_market_skill', {
                owner: skill.owner,
                repo: skill.repo,
                slug: skill.slug,
            });
            setSkills({ ...skills(), [downloaded.id]: downloaded });
        } catch (e) {
            setError(`下载失败: ${e}`);
        } finally {
            setDownloadingId(null);
        }
    };

    const updateField = <K extends keyof SkillConfig>(key: K, value: SkillConfig[K]) => {
        const current = editing();
        if (current) setEditing({ ...current, [key]: value });
    };

    const save = async () => {
        const skill = editing();
        if (!skill) return;
        if (!skill.name.trim() || !skill.content.trim()) {
            setError('Skill 名称和指令内容不能为空');
            return;
        }
        try {
            await invoke('save_skill', { skill });
            setSkills({ ...skills(), [skill.id]: skill });
            setEditing(null);
            setIsCreating(false);
        } catch (e) {
            setError(`保存失败: ${e}`);
        }
    };

    const remove = async (id: string) => {
        if (!confirm('确定移除此 Skill？所有助手中的引用也会被移除。')) return;
        try {
            await invoke('delete_skill', { id });
            const next = { ...skills() };
            delete next[id];
            setSkills(next);

            const affectedAssistantIds = datas.assistants
                .filter(assistant => assistant.skillIds?.includes(id))
                .map(assistant => assistant.id);
            for (const assistantId of affectedAssistantIds) {
                setDatas(
                    'assistants',
                    assistant => assistant.id === assistantId,
                    'skillIds',
                    (ids: string[] | undefined) => (ids ?? []).filter(skillId => skillId !== id),
                );
            }
            await Promise.all(affectedAssistantIds.map(saveSingleAssistantToBackend));
        } catch (e) {
            setError(`移除失败: ${e}`);
        }
    };

    return (
        <div class="flex flex-col h-full overflow-hidden p-6 gap-4" style="color: rgba(255,255,255,0.88);">
            <div class="flex items-start justify-between gap-4">
                <div>
                    <h2 class="text-xl font-semibold">Skill 市场</h2>
                    <p class="text-xs mt-1" style="color: rgba(255,255,255,0.5);">
                        浏览 skills.sh 社区 Skill，下载后可在各助手设置中分别启用。
                    </p>
                </div>
                <button
                    class="px-3 py-1.5 rounded-md text-sm cursor-pointer"
                    style="background: rgba(124,154,191,0.2); border: 1px solid rgba(124,154,191,0.3);"
                    onClick={() => { setIsCreating(true); setEditing(emptySkill()); }}
                >
                    + 创建本地 Skill
                </button>
            </div>

            <div class="flex items-center justify-between gap-3 flex-wrap">
                <div class="flex items-center gap-1 p-1 rounded-lg" style="background: rgba(255,255,255,0.04);">
                    <button class="px-3 py-1.5 rounded-md text-sm"
                        classList={{ 'bg-pri-20 text-pri': view() === 'market' }}
                        onClick={() => setView('market')}>
                        市场
                    </button>
                    <button class="px-3 py-1.5 rounded-md text-sm"
                        classList={{ 'bg-pri-20 text-pri': view() === 'downloaded' }}
                        onClick={() => setView('downloaded')}>
                        已下载 ({Object.keys(skills()).length})
                    </button>
                </div>
                <input
                    class="w-[280px] max-w-full px-3 py-2 rounded-lg text-sm outline-none"
                    style="background: rgba(0,0,0,0.25); border: 1px solid rgba(255,255,255,0.1);"
                    value={query()}
                    onInput={(e) => setQuery(e.currentTarget.value)}
                    placeholder="搜索 Skill、作者或仓库"
                />
            </div>

            <Show when={view() === 'market'}>
                <div class="flex items-center gap-3 flex-wrap">
                    <div class="flex gap-1">
                        {([
                            ['all', '总热度'],
                            ['trending', '24 小时趋势'],
                            ['hot', 'Hot'],
                        ] as const).map(([value, label]) => (
                            <button
                                class="px-3 py-1.5 rounded-md text-xs"
                                classList={{ 'bg-pri-20 text-pri': sort() === value }}
                                style={sort() === value ? '' : 'background: rgba(255,255,255,0.04);'}
                                onClick={() => selectSort(value)}
                            >
                                {label}
                            </button>
                        ))}
                    </div>
                    <select
                        class="px-3 py-1.5 rounded-md text-xs outline-none"
                        style="background: rgba(22,26,40,0.95); border: 1px solid rgba(255,255,255,0.1);"
                        value={category()}
                        onChange={(e) => selectCategory(e.currentTarget.value)}
                    >
                        <option value="all">全部分类</option>
                        <For each={categories()}>
                            {(item) => <option value={item.id}>{item.name} ({item.skillCount})</option>}
                        </For>
                    </select>
                    <span class="text-xs" style="color: rgba(255,255,255,0.4);">
                        {filteredMarketSkills().length} 个结果
                    </span>
                    <button
                        class="px-3 py-1.5 rounded-md text-xs ml-auto"
                        style="background: rgba(255,255,255,0.05); border: 1px solid rgba(255,255,255,0.1);"
                        disabled={refreshing()}
                        onClick={() => void refreshMarket()}
                    >
                        {refreshing() ? '更新中…' : '手动更新'}
                    </button>
                    <Show when={lastRefreshedAt()}>
                        <span class="text-[10px]" style="color: rgba(255,255,255,0.3);">
                            更新于 {lastRefreshedAt()!.toLocaleTimeString()}
                        </span>
                    </Show>
                </div>
            </Show>

            <Show when={error()}>
                <div class="px-3 py-2 rounded-md text-sm" style="background: rgba(255,77,77,0.1); color: #ff8a8a;">
                    {error()}
                    <button class="ml-3 underline" onClick={() => setError(null)}>关闭</button>
                </div>
            </Show>

            <div class="flex-1 overflow-y-auto min-h-0">
                <Show when={loading()}>
                    <div class="h-full flex items-center justify-center text-sm" style="color: rgba(255,255,255,0.45);">
                        正在加载 skills.sh…
                    </div>
                </Show>

                <Show when={!loading() && view() === 'market'}>
                    <div class="grid grid-cols-1 xl:grid-cols-2 gap-3">
                        <For each={filteredMarketSkills()} fallback={
                            <div class="col-span-full py-12 text-center text-sm" style="color: rgba(255,255,255,0.4);">
                                没有匹配的 Skill。
                            </div>
                        }>
                            {(skill, index) => {
                                const downloaded = () => Boolean(skills()[skill.id]);
                                const weekly = () => skill.weeklyInstalls[skill.weeklyInstalls.length - 1] ?? 0;
                                return (
                                    <div class="flex flex-col gap-3 rounded-xl p-4"
                                        style="background: rgba(255,255,255,0.035); border: 1px solid rgba(255,255,255,0.08);">
                                        <div class="flex items-start gap-3">
                                            <div class="w-8 text-center font-mono text-sm pt-0.5" style="color: rgba(255,255,255,0.35);">
                                                #{index() + 1}
                                            </div>
                                            <div class="flex-1 min-w-0">
                                                <div class="font-semibold truncate">{skill.name}</div>
                                                <button class="text-xs font-mono truncate hover:underline"
                                                    style="color: rgba(124,154,191,0.8);"
                                                    onClick={() => void openUrl(skill.sourceUrl)}>
                                                    {skill.owner}/{skill.repo}
                                                </button>
                                            </div>
                                            <div class="text-right shrink-0">
                                                <div class="font-mono text-sm">{skill.installsLabel || formatInstalls(skill.installs)}</div>
                                                <div class="text-[10px]" style="color: rgba(255,255,255,0.35);">
                                                    总安装
                                                </div>
                                            </div>
                                        </div>
                                        <p class="text-xs leading-relaxed line-clamp-3 min-h-[3rem]" style="color: rgba(255,255,255,0.55);">
                                            {skill.description || '该 Skill 暂无简介，下载后可查看完整指令。'}
                                        </p>
                                        <div class="flex items-center justify-between gap-3">
                                            <span class="text-[11px]" style="color: rgba(255,255,255,0.4);">
                                                {weekly() > 0 ? `最近一周 ${formatInstalls(weekly())}` : 'skills.sh 社区内容'}
                                            </span>
                                            <button
                                                class="px-3 py-1.5 rounded-md text-xs"
                                                disabled={downloaded() || downloadingId() === skill.id}
                                                style={downloaded()
                                                    ? 'background: rgba(124,217,160,0.12); color: #7cd9a0;'
                                                    : 'background: rgba(124,154,191,0.2); border: 1px solid rgba(124,154,191,0.3);'}
                                                onClick={() => void download(skill)}
                                            >
                                                {downloaded() ? '已下载' : downloadingId() === skill.id ? '下载中…' : '下载'}
                                            </button>
                                        </div>
                                    </div>
                                );
                            }}
                        </For>
                    </div>
                </Show>

                <Show when={!loading() && view() === 'downloaded'}>
                    <div class="flex flex-col gap-2">
                        <For each={filteredLocalSkills()} fallback={
                            <div class="py-12 text-center text-sm" style="color: rgba(255,255,255,0.4);">
                                尚未下载 Skill。
                            </div>
                        }>
                            {(skill) => (
                                <div class="flex items-center justify-between px-4 py-3 rounded-lg"
                                    style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,255,255,0.08);">
                                    <div class="flex-1 min-w-0">
                                        <div class="flex items-center gap-2">
                                            <span class="font-medium truncate">{skill.name}</span>
                                            <Show when={skill.sourceUrl}>
                                                <span class="text-[10px] px-1.5 py-0.5 rounded bg-pri-10 text-pri">skills.sh</span>
                                            </Show>
                                        </div>
                                        <div class="text-xs truncate mt-1" style="color: rgba(255,255,255,0.5);">
                                            {skill.description || skill.content}
                                        </div>
                                    </div>
                                    <div class="flex gap-2 ml-3">
                                        <Show when={skill.sourceUrl}>
                                            <button class="px-2 py-1 rounded text-xs"
                                                style="background: rgba(255,255,255,0.05);"
                                                onClick={() => void openUrl(skill.sourceUrl!)}>
                                                来源
                                            </button>
                                        </Show>
                                        <button class="px-2 py-1 rounded text-xs"
                                            style="background: rgba(255,255,255,0.05);"
                                            onClick={() => { setIsCreating(false); setEditing({ ...skill }); }}>
                                            编辑
                                        </button>
                                        <button class="px-2 py-1 rounded text-xs"
                                            style="background: rgba(255,77,77,0.1); color: rgba(255,107,107,0.9);"
                                            onClick={() => void remove(skill.id)}>
                                            移除
                                        </button>
                                    </div>
                                </div>
                            )}
                        </For>
                    </div>
                </Show>
            </div>

            <Show when={editing()}>
                <div class="fixed inset-0 z-50 flex items-center justify-center p-6"
                    style="background: rgba(0,0,0,0.6); backdrop-filter: blur(8px);"
                    onClick={(e) => e.target === e.currentTarget && setEditing(null)}>
                    <div class="w-[640px] max-w-full max-h-[90vh] overflow-y-auto rounded-xl p-6 flex flex-col gap-4"
                        style="background: rgba(18,22,35,0.98); border: 1px solid rgba(255,255,255,0.1);">
                        <h3 class="text-base font-semibold">{isCreating() ? '创建本地 Skill' : '编辑 Skill'}</h3>
                        <label class="flex flex-col gap-1 text-xs">
                            名称
                            <input class="px-3 py-2 rounded text-sm outline-none"
                                style="background: rgba(0,0,0,0.3); border: 1px solid rgba(255,255,255,0.1);"
                                value={editing()!.name}
                                onInput={(e) => updateField('name', e.currentTarget.value)} />
                        </label>
                        <label class="flex flex-col gap-1 text-xs">
                            说明
                            <input class="px-3 py-2 rounded text-sm outline-none"
                                style="background: rgba(0,0,0,0.3); border: 1px solid rgba(255,255,255,0.1);"
                                value={editing()!.description}
                                onInput={(e) => updateField('description', e.currentTarget.value)} />
                        </label>
                        <label class="flex flex-col gap-1 text-xs">
                            系统指令
                            <textarea class="px-3 py-2 rounded text-sm outline-none min-h-[260px] resize-y font-mono"
                                style="background: rgba(0,0,0,0.3); border: 1px solid rgba(255,255,255,0.1);"
                                value={editing()!.content}
                                onInput={(e) => updateField('content', e.currentTarget.value)} />
                        </label>
                        <div class="flex justify-end gap-2">
                            <button class="px-3 py-1.5 rounded text-sm" style="background: rgba(255,255,255,0.05);"
                                onClick={() => setEditing(null)}>取消</button>
                            <button class="px-3 py-1.5 rounded text-sm bg-pri text-black" onClick={() => void save()}>
                                保存
                            </button>
                        </div>
                    </div>
                </div>
            </Show>
        </div>
    );
};

export default SkillList;
