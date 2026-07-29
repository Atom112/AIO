import {
  Component,
  For,
  Show,
  createMemo,
  createSignal,
  createEffect,
  on,
  onMount,
} from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import {
  datas,
  saveSingleAssistantToBackend,
  setDatas,
  setSkills,
  skills,
  currentProjectId,
  currentProject,
} from '../../../core/store/store';
import Dropdown from '../../../shared/components/Dropdown';
import Icon from '../../../shared/components/Icon';
import { t } from '../../../core/i18n';
import type {
  MarketSkill,
  SkillConfig,
  SkillMarketCategory,
  DiscoveredNpxSkill,
} from '../../../core/types/skill';

type MarketSort = 'all' | 'trending' | 'hot';
type ViewMode = 'market' | 'downloaded' | 'npx';
type ScopeMode = 'global' | 'project';

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

const formatInstalls = (value: number): string => {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return String(value);
};

const SkillList: Component = () => {
  const initialCache = readMarketCache();
  const [view, setView] = createSignal<ViewMode>('market');
  const [scope, setScope] = createSignal<ScopeMode>(currentProjectId() ? 'project' : 'global');
  const projectId = () => (scope() === 'project' ? currentProjectId() : null);
  const [marketSkills, setMarketSkills] = createSignal<MarketSkill[]>(
    initialCache?.markets.all ?? [],
  );
  const [categories, setCategories] = createSignal<SkillMarketCategory[]>(
    initialCache?.categories ?? [],
  );
  const [sort, setSort] = createSignal<MarketSort>('all');
  const [category, setCategory] = createSignal('all');
  const [query, setQuery] = createSignal('');
  const [loading, setLoading] = createSignal(!initialCache?.markets.all);
  const [refreshing, setRefreshing] = createSignal(false);
  const [refreshResult, setRefreshResult] = createSignal<{ ok: boolean; msg: string } | null>(null);
  const [lastRefreshedAt, setLastRefreshedAt] = createSignal<Date | null>(
    initialCache ? new Date(initialCache.marketUpdatedAt.all ?? initialCache.updatedAt) : null,
  );
  const [downloadingId, setDownloadingId] = createSignal<string | null>(null);
  const [editing, setEditing] = createSignal<SkillConfig | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  // ====== npx 发现状态 ======
  const [npxSkills, setNpxSkills] = createSignal<DiscoveredNpxSkill[]>([]);
  const [npxLoading, setNpxLoading] = createSignal(false);
  const [npxImportingId, setNpxImportingId] = createSignal<string | null>(null);
  const [npxRefreshingId, setNpxRefreshingId] = createSignal<string | null>(null);
  let npxScanned = false; // 首次进入 npx 标签自动扫描，之后缓存结果

  const discoverNpx = async () => {
    setNpxLoading(true);
    setError(null);
    try {
      const list = await invoke<DiscoveredNpxSkill[]>('discover_npx_skills');
      setNpxSkills(list);
      npxScanned = true;
    } catch (e) {
      setError(t('skill.scanFailed', { error: String(e) }));
    } finally {
      setNpxLoading(false);
    }
  };

  const importNpx = async (pkgName: string) => {
    // 安全确认：npx 导入会执行来自 npm 的未审查代码
    const confirmed = window.confirm(t('skill.importNpxConfirm', { name: pkgName }));
    if (!confirmed) return;

    setNpxImportingId(pkgName);
    setError(null);
    try {
      const imported = await invoke<SkillConfig>('import_npx_skill', {
        packageName: pkgName,
        projectId: projectId(),
      });
      setSkills({ ...skills(), [imported.id]: imported });
      // 更新列表中的 alreadyImported 标记
      setNpxSkills((prev) =>
        prev.map((s) => (s.packageName === pkgName ? { ...s, alreadyImported: true } : s)),
      );
    } catch (e) {
      setError(t('skill.importFailed', { error: String(e) }));
    } finally {
      setNpxImportingId(null);
    }
  };

  const refreshNpx = async (id: string) => {
    // 安全确认
    const confirmed = window.confirm(t('skill.refreshConfirm'));
    if (!confirmed) return;

    setNpxRefreshingId(id);
    setError(null);
    try {
      const updated = await invoke<SkillConfig>('refresh_npx_skill', {
        id,
        projectId: projectId(),
      });
      setSkills({ ...skills(), [updated.id]: updated });
    } catch (e) {
      setError(t('skill.refreshFailed') + ': ' + e);
    } finally {
      setNpxRefreshingId(null);
    }
  };

  const loadMarket = async (nextSort = sort(), nextCategory = category(), forceRefresh = false) => {
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
      setError(t('skill.loadShFailed', { error: String(e) }));
    } finally {
      setLoading(false);
    }
  };

  onMount(async () => {
    const cacheIsFresh =
      initialCache &&
      Date.now() - (initialCache.marketUpdatedAt.all ?? 0) < MARKET_CACHE_TTL_MS &&
      Date.now() - initialCache.updatedAt < MARKET_CACHE_TTL_MS;
    setLoading(!initialCache?.markets.all);
    try {
      const localList = await invoke<SkillConfig[]>('list_skills', { projectId: projectId() });
      setSkills(Object.fromEntries(localList.map((skill) => [skill.id, skill])));
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
      setError(t('skill.loadMarketFailed', { error: String(e) }));
    } finally {
      setLoading(false);
    }
  });

  // 监听 scope 切换，自动重新加载对应范围的 Skill 列表
  createEffect(
    on(
      projectId,
      async (pid) => {
        try {
          const list = await invoke<SkillConfig[]>('list_skills', { projectId: pid });
          setSkills(Object.fromEntries(list.map((skill) => [skill.id, skill])));
        } catch (e) {
          console.warn('切换 Skill 范围失败:', e);
        }
      },
      { defer: true },
    ),
  );

  const filteredMarketSkills = createMemo(() => {
    const keyword = query().trim().toLowerCase();
    if (!keyword) return marketSkills();
    return marketSkills().filter((skill) =>
      `${skill.name} ${skill.description} ${skill.owner} ${skill.repo}`
        .toLowerCase()
        .includes(keyword),
    );
  });

  const filteredLocalSkills = createMemo(() => {
    const keyword = query().trim().toLowerCase();
    const list = Object.values(skills()).sort((a, b) => a.name.localeCompare(b.name));
    if (!keyword) return list;
    return list.filter((skill) =>
      `${skill.name} ${skill.description} ${skill.sourceOwner ?? ''} ${skill.sourceRepo ?? ''}`
        .toLowerCase()
        .includes(keyword),
    );
  });

  const categoryOptions = createMemo(() => [
    { value: 'all', label: t('skill.allCategories') },
    ...categories().map((item) => ({ value: item.id, label: `${item.name} (${item.skillCount})` })),
  ]);

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
    setRefreshResult(null);
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
      setRefreshResult({ ok: true, msg: t('skill.updateCount', { count: marketList.length }) });
    } catch {
      setRefreshResult({ ok: false, msg: t('skill.refreshFailed') });
    } finally {
      setRefreshing(false);
      setTimeout(() => setRefreshResult(null), 3000);
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
        projectId: projectId(),
      });
      setSkills({ ...skills(), [downloaded.id]: downloaded });
    } catch (e) {
      setError(t('skill.downloadFailed', { error: String(e) }));
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
      setError(t('skill.nameAndInstructionRequired'));
      return;
    }
    try {
      await invoke('save_skill', { skill, projectId: projectId() });
      setSkills({ ...skills(), [skill.id]: skill });
      setEditing(null);
    } catch (e) {
      setError(t('skill.saveFailed', { error: String(e) }));
    }
  };

  const remove = async (id: string) => {
    if (!confirm(t('skill.removeConfirm'))) return;
    try {
      await invoke('delete_skill', { id, projectId: projectId() });
      const next = { ...skills() };
      delete next[id];
      setSkills(next);

      const affectedAssistantIds = datas.assistants
        .filter((assistant) => assistant.skillIds?.includes(id))
        .map((assistant) => assistant.id);
      for (const assistantId of affectedAssistantIds) {
        setDatas(
          'assistants',
          (assistant) => assistant.id === assistantId,
          'skillIds',
          (ids: string[] | undefined) => (ids ?? []).filter((skillId) => skillId !== id),
        );
      }
      await Promise.all(affectedAssistantIds.map(saveSingleAssistantToBackend));
    } catch (e) {
      setError(t('skill.removeFailed', { error: String(e) }));
    }
  };

  return (
    <div
      class="flex flex-col h-full overflow-hidden p-6 gap-4"
      style={{ color: 'rgba(var(--text-base-rgb),0.88)' }}
    >
      <div class="animate-row-in flex items-start justify-between gap-4">
        <h2 class="text-xl font-semibold">{t('skill.market')}</h2>
      </div>

      <div
        class="animate-row-in flex items-center justify-between gap-3 flex-wrap"
        style={{ 'animation-delay': '30ms' }}
      >
        <div class="flex items-center gap-3 flex-wrap">
          <div
            class="flex items-center gap-1 p-1 rounded-lg"
            style={{ background: 'rgba(var(--text-base-rgb),0.04)' }}
          >
            <button
              class="px-3 py-1.5 rounded-md text-sm"
              classList={{ 'bg-pri-20 text-pri': view() === 'market' }}
              onClick={() => setView('market')}
            >
              {t('skill.tabMarket')}
            </button>
            <button
              class="px-3 py-1.5 rounded-md text-sm"
              classList={{ 'bg-pri-20 text-pri': view() === 'downloaded' }}
              onClick={() => setView('downloaded')}
            >
              {t('skill.tabDownloaded', { count: Object.keys(skills()).length })}
            </button>
            <button
              class="px-3 py-1.5 rounded-md text-sm"
              classList={{ 'bg-pri-20 text-pri': view() === 'npx' }}
              onClick={() => {
                setView('npx');
                if (!npxScanned) discoverNpx();
              }}
            >
              {t('skill.tabNpx')}
            </button>
          </div>
          {/* 存在活跃项目时，显示作用域切换（市场/已下载/npx 均可看到） */}
          {currentProjectId() && (
            <div
              class="flex items-center gap-1 p-1 rounded-lg"
              style={{ background: 'rgba(var(--text-base-rgb),0.04)' }}
            >
              <button
                class="px-2.5 py-1 rounded-md text-xs"
                classList={{ 'bg-pri-20 text-pri': scope() === 'global' }}
                onClick={() => setScope('global')}
              >
                {t('skill.scopeGlobal')}
              </button>
              <button
                class="px-2.5 py-1 rounded-md text-xs"
                classList={{ 'bg-pri-20 text-pri': scope() === 'project' }}
                onClick={() => setScope('project')}
              >
                {t('skill.scopeProject', { name: currentProject()?.name ?? '' })}
              </button>
            </div>
          )}
        </div>
        <div class="relative">
          <Icon
            name="search"
            size={13}
            class="absolute left-2.5 top-1/2 -translate-y-1/2 text-[#666] pointer-events-none"
          />
          <input
            class="w-[280px] max-w-full bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none pl-8 pr-3 py-2 text-sm"
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            placeholder={t('skill.searchPlaceholder')}
          />
        </div>
      </div>

      <Show when={view() === 'market'}>
        <div
          class="animate-row-in flex items-center gap-3 flex-wrap"
          style={{ 'animation-delay': '60ms' }}
        >
          <div class="flex gap-1">
            <For
              each={
                [
                  ['all', t('skill.tabHot')],
                  ['trending', t('skill.tabTrend')],
                  ['hot', 'Hot'],
                ] as const
              }
            >
              {([value, label]) => (
                <button
                  class="px-3 py-1.5 rounded-md text-xs"
                  classList={{ 'bg-pri-20 text-pri': sort() === value }}
                  style={sort() === value ? '' : 'background: rgba(var(--text-base-rgb),0.04);'}
                  onClick={() => selectSort(value)}
                >
                  {label}
                </button>
              )}
            </For>
          </div>
          <Dropdown
            value={category()}
            onChange={(v) => selectCategory(v)}
            options={categoryOptions()}
            class="text-xs"
          />
          <button
            type="button"
            class="flex items-center gap-1 px-2 py-1 text-[10px] rounded-md border border-white/10 text-[#aaa] hover:border-pri-30 hover:text-white transition-all duration-200 active:scale-95 disabled:opacity-50 ml-auto"
            disabled={refreshing()}
            onClick={() => void refreshMarket()}
          >
            <Show when={refreshing()} fallback={<Icon name="refresh" size={11} />}>
              <Icon name="spinner" size={11} class="animate-spin" />
            </Show>
            {refreshing() ? t('skill.refreshing') : t('skill.refresh')}
          </button>
          <Show when={refreshResult()}>
            <span
              class="text-[10px]"
              classList={{
                'text-green-300': refreshResult()!.ok,
                'text-red-300': !refreshResult()!.ok,
              }}
            >
              {refreshResult()!.msg}
            </span>
          </Show>
          <Show when={lastRefreshedAt()}>
            <span class="text-[10px]" style={{ color: 'rgba(var(--text-base-rgb),0.3)' }}>
              {t('skill.updatedAt', { time: lastRefreshedAt()!.toLocaleTimeString() })}
            </span>
          </Show>
        </div>
      </Show>

      <Show when={error()}>
        <div
          class="px-3 py-2 rounded-md text-sm"
          style={{ background: 'rgba(255,77,77,0.1)', color: '#ff8a8a' }}
        >
          {error()}
          <button class="ml-3 underline" onClick={() => setError(null)}>
            {t('skill.close')}
          </button>
        </div>
      </Show>

      <div class="flex-1 overflow-y-auto min-h-0">
        <Show when={loading()}>
          <div
            class="h-full flex items-center justify-center text-sm"
            style={{ color: 'rgba(var(--text-base-rgb),0.45)' }}
          >
            {t('skill.loadingMarket')}
          </div>
        </Show>

        <Show when={!loading() && view() === 'market'}>
          <div class="grid grid-cols-1 lg:grid-cols-2 gap-3">
            <For
              each={filteredMarketSkills()}
              fallback={
                <div
                  class="col-span-full py-12 text-center text-sm"
                  style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                >
                  没有匹配的 Skill。
                </div>
              }
            >
              {(skill, index) => {
                const downloaded = () => Boolean(skills()[skill.id]);
                const weekly = () => skill.weeklyInstalls[skill.weeklyInstalls.length - 1] ?? 0;
                return (
                  <div
                    class="animate-row-in flex flex-col gap-3 rounded-xl p-4"
                    style={{
                      'animation-delay': `${Math.min((index() + 3) * 10, 500)}ms`,
                      background: 'rgba(var(--text-base-rgb),0.035)',
                      border: '1px solid var(--border-dim)',
                    }}
                  >
                    <div class="flex items-start gap-3">
                      <div
                        class="w-8 text-center font-mono text-sm pt-0.5"
                        style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}
                      >
                        #{index() + 1}
                      </div>
                      <div class="flex-1 min-w-0">
                        <div class="font-semibold truncate">{skill.name}</div>
                        <button
                          class="text-xs font-mono truncate hover:underline"
                          style={{ color: 'rgba(var(--primary-rgb),0.8)' }}
                          onClick={() => void openUrl(skill.sourceUrl)}
                        >
                          {skill.owner}/{skill.repo}
                        </button>
                      </div>
                      <div class="text-right shrink-0">
                        <div class="font-mono text-sm">
                          {skill.installsLabel || formatInstalls(skill.installs)}
                        </div>
                        <div
                          class="text-[10px]"
                          style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}
                        >
                          {t('skill.installs')}
                        </div>
                      </div>
                    </div>
                    <p
                      class="text-xs leading-relaxed line-clamp-3 min-h-[3rem]"
                      style={{ color: 'rgba(var(--text-base-rgb),0.55)' }}
                    >
                      {skill.description || t('skill.noDescription')}
                    </p>
                    <div class="flex items-center justify-between gap-3">
                      <span class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}>
                        {weekly() > 0
                          ? t('skill.weeklyInstalls', { count: formatInstalls(weekly()) })
                          : t('skill.communityContent')}
                      </span>
                      <button
                        class="px-3 py-1.5 rounded-md text-xs"
                        disabled={downloaded() || downloadingId() === skill.id}
                        style={
                          downloaded()
                            ? 'background: rgba(124,217,160,0.12); color: #7cd9a0;'
                            : 'background: rgba(var(--primary-rgb),0.2); border: 1px solid rgba(var(--primary-rgb),0.3);'
                        }
                        onClick={() => void download(skill)}
                      >
                        {downloaded()
                          ? t('skill.downloaded')
                          : downloadingId() === skill.id
                            ? t('skill.downloading')
                            : t('skill.download')}
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
            <For
              each={filteredLocalSkills()}
              fallback={
                <div
                  class="py-12 text-center text-sm"
                  style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                >
                  {t('skill.noInstalled')}
                </div>
              }
            >
              {(skill, index) => (
                <div
                  class="animate-row-in flex items-center justify-between px-4 py-3 rounded-lg"
                  style={{
                    'animation-delay': `${(index() + 3) * 30}ms`,
                    background: 'rgba(var(--text-base-rgb),0.03)',
                    border: '1px solid var(--border-dim)',
                  }}
                >
                  <div class="flex-1 min-w-0">
                    <div class="flex items-center gap-2">
                      <span class="font-medium truncate">{skill.name}</span>
                      <Show when={skill.sourceUrl}>
                        <span class="text-[10px] px-1.5 py-0.5 rounded bg-pri-10 text-pri">
                          skills.sh
                        </span>
                      </Show>
                    </div>
                    <div
                      class="text-xs truncate mt-1"
                      style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}
                    >
                      {skill.description || skill.content}
                    </div>
                  </div>
                  <div class="flex gap-2 ml-3">
                    <Show when={skill.sourceUrl}>
                      <button
                        class="px-2 py-1 rounded text-xs"
                        style={{ background: 'rgba(var(--text-base-rgb),0.05)' }}
                        onClick={() => void openUrl(skill.sourceUrl!)}
                      >
                        {t('skill.source')}
                      </button>
                    </Show>
                    <button
                      class="px-2 py-1 rounded text-xs"
                      style={{ background: 'rgba(var(--text-base-rgb),0.05)' }}
                      onClick={() => setEditing({ ...skill })}
                    >
                      {t('skill.edit')}
                    </button>
                    <button
                      class="px-2 py-1 rounded text-xs"
                      style={{ background: 'rgba(255,77,77,0.1)', color: 'rgba(255,107,107,0.9)' }}
                      onClick={() => void remove(skill.id)}
                    >
                      {t('skill.remove')}
                    </button>
                  </div>
                </div>
              )}
            </For>
          </div>
        </Show>

        {/* npx 发现标签页 */}
        <Show when={view() === 'npx'}>
          <div class="animate-row-in flex flex-col gap-3" style={{ 'animation-delay': '30ms' }}>
            <div class="flex items-center justify-between">
              <p class="text-xs" style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}>
                {t('skill.npxHint')}
              </p>
              <button
                class="px-3 py-1.5 rounded-md text-xs"
                style={{ background: 'rgba(var(--text-base-rgb),0.05)' }}
                onClick={() => discoverNpx()}
                disabled={npxLoading()}
              >
                {npxLoading() ? t('skill.scanning') : t('skill.rescan')}
              </button>
            </div>

            <Show
              when={!npxLoading()}
              fallback={
                <div
                  class="py-12 text-center text-sm"
                  style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                >
                  正在扫描系统上的 npx skill...
                </div>
              }
            >
              <div class="flex flex-col gap-2">
                <For
                  each={npxSkills()}
                  fallback={
                    <div
                      class="py-12 text-center text-sm"
                      style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                    >
                      {t('skill.npxEmpty')}
                    </div>
                  }
                >
                  {(item) => (
                    <div
                      class="flex items-center justify-between px-4 py-3 rounded-lg"
                      style={{
                        background: 'rgba(var(--text-base-rgb),0.03)',
                        border: '1px solid var(--border-dim)',
                      }}
                    >
                      <div class="flex-1 min-w-0">
                        <div class="flex items-center gap-2">
                          <span class="font-medium truncate text-sm">{item.packageName}</span>
                          <span class="text-[10px] px-1.5 py-0.5 rounded bg-white/10 text-white/50">
                            v{item.version}
                          </span>
                          <span class="text-[10px] px-1.5 py-0.5 rounded bg-pri-10 text-pri">
                            {item.sourceType === 'claude-skills-dir' ? 'Claude' : 'npm'}
                          </span>
                        </div>
                        <Show when={item.description}>
                          <div
                            class="text-xs truncate mt-1"
                            style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}
                          >
                            {item.description}
                          </div>
                        </Show>
                      </div>
                      <div class="flex gap-2 ml-3">
                        <Show
                          when={item.alreadyImported}
                          fallback={
                            <button
                              class="px-3 py-1 rounded text-xs bg-pri text-black disabled:opacity-50"
                              onClick={() => importNpx(item.packageName)}
                              disabled={npxImportingId() === item.packageName}
                            >
                              {npxImportingId() === item.packageName
                                ? t('skill.importing')
                                : t('skill.import')}
                            </button>
                          }
                        >
                          <span
                            class="text-xs px-2 py-1 rounded"
                            style={{
                              background: 'rgba(var(--text-base-rgb),0.05)',
                              color: 'rgba(var(--text-base-rgb),0.5)',
                            }}
                          >
                            {t('skill.imported')}
                          </span>
                          <button
                            class="px-2 py-1 rounded text-xs"
                            style={{ background: 'rgba(var(--text-base-rgb),0.05)' }}
                            onClick={() => refreshNpx(`npx-${item.packageName}`)}
                            disabled={npxRefreshingId() === `npx-${item.packageName}`}
                          >
                            {npxRefreshingId() === `npx-${item.packageName}`
                              ? t('skill.refreshing')
                              : t('skill.refresh')}
                          </button>
                        </Show>
                      </div>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </div>
        </Show>
      </div>

      <Show when={editing()}>
        <div
          class="fixed inset-0 z-50 flex items-center justify-center p-6"
          style={{ background: 'rgba(0,0,0,0.6)', 'backdrop-filter': 'blur(8px)' }}
          onClick={(e) => e.target === e.currentTarget && setEditing(null)}
        >
          <div
            class="w-[640px] max-w-full max-h-[90vh] overflow-y-auto rounded-xl p-6 flex flex-col gap-4"
            style={{
              background: 'rgba(var(--surface-bg),0.98)',
              border: '1px solid var(--border-dim)',
            }}
          >
            <h3 class="text-base font-semibold">{t('skill.editTitle')}</h3>
            <label class="flex flex-col gap-1 text-xs">
              {t('skill.name')}
              <input
                class="px-3 py-2 rounded text-sm outline-none"
                style={{ background: 'rgba(0,0,0,0.3)', border: '1px solid var(--border-dim)' }}
                value={editing()!.name}
                onInput={(e) => updateField('name', e.currentTarget.value)}
              />
            </label>
            <label class="flex flex-col gap-1 text-xs">
              {t('skill.description')}
              <input
                class="px-3 py-2 rounded text-sm outline-none"
                style={{ background: 'rgba(0,0,0,0.3)', border: '1px solid var(--border-dim)' }}
                value={editing()!.description}
                onInput={(e) => updateField('description', e.currentTarget.value)}
              />
            </label>
            <label class="flex flex-col gap-1 text-xs">
              {t('skill.systemPrompt')}
              <textarea
                class="px-3 py-2 rounded text-sm outline-none min-h-[260px] resize-y font-mono"
                style={{ background: 'rgba(0,0,0,0.3)', border: '1px solid var(--border-dim)' }}
                value={editing()!.content}
                onInput={(e) => updateField('content', e.currentTarget.value)}
              />
            </label>
            <div class="flex justify-end gap-2">
              <button
                class="px-3 py-1.5 rounded text-sm"
                style={{ background: 'rgba(var(--text-base-rgb),0.05)' }}
                onClick={() => setEditing(null)}
              >
                {t('skill.cancel')}
              </button>
              <button
                class="px-3 py-1.5 rounded text-sm bg-pri text-black"
                onClick={() => void save()}
              >
                {t('skill.save')}
              </button>
            </div>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default SkillList;
