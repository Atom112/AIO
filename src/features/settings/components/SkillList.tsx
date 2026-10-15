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
import {
  deleteSkill,
  discoverNpxSkills,
  downloadMarketSkill,
  importNpxSkill,
  listSkillMarket,
  listSkills,
  refreshNpxSkill,
  saveSkill,
  searchSkillMarket,
} from '../../../core/api/skills';
import { openSafeUrl } from '../../../core/utils/safeUrl';
import {
  datas,
  saveSingleAssistantToBackend,
  setDatas,
  setSkills,
  skills,
  currentProjectId,
  currentProject,
} from '../../../core/store/store';
import Icon from '../../../shared/components/Icon';
import {
  btnBadge,
  btnBadgeBar,
  btnDanger,
  btnPrimarySm,
  btnSecondarySm,
  btnSuccess,
  btnTabXs,
  tabActive,
} from '../../../shared/components/buttonStyles';
import { createJsonCache } from '../../../core/utils/cache';
import { t } from '../../../core/i18n';
import type { MarketSkill, SkillConfig, DiscoveredNpxSkill } from '../../../core/types/skill';

type MarketSort = 'all' | 'trending' | 'hot';
type ViewMode = 'market' | 'downloaded' | 'npx';
type ScopeMode = 'global' | 'project';

const MARKET_CACHE_KEY = 'aio-skill-market-cache-v1';
const MARKET_CACHE_TTL_MS = 6 * 60 * 60 * 1000;

interface SkillMarketCache {
  updatedAt: number;
  markets: Partial<Record<MarketSort, MarketSkill[]>>;
  marketUpdatedAt: Partial<Record<MarketSort, number>>;
}

// 阶段4（设置样板收敛）: localStorage JSON cache via the shared factory
// (core/utils/cache) — read is parse-memoized, write swallows quota errors.
const marketCache = createJsonCache<SkillMarketCache>(MARKET_CACHE_KEY, (raw) => {
  if (!raw || typeof raw !== 'object') return null;
  const cache = raw as Partial<SkillMarketCache>;
  if (!cache.updatedAt || !cache.markets) return null;
  // Schema-evolution backfill: pre-bucket caches stored one global timestamp.
  cache.marketUpdatedAt ??= { all: cache.updatedAt };
  return cache as SkillMarketCache;
});

const formatInstalls = (value: number): string => {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return String(value);
};

// ====== npx discovery cache (module-level signal: survives leaving the page or switching settings tabs, avoids rescanning on every visit) ======
// Session-only; the first npx tab visit after an app restart still scans. Click "rescan" to force a refresh.
const [cachedNpxSkills, setCachedNpxSkills] = createSignal<DiscoveredNpxSkill[] | null>(null);

const SkillList: Component = () => {
  const initialCache = marketCache.read();
  const [view, setView] = createSignal<ViewMode>('market');
  const [scope, setScope] = createSignal<ScopeMode>(currentProjectId() ? 'project' : 'global');
  const projectId = () => (scope() === 'project' ? currentProjectId() : null);
  const [marketSkills, setMarketSkills] = createSignal<MarketSkill[]>(
    initialCache?.markets.all ?? [],
  );
  const [sort, setSort] = createSignal<MarketSort>('all');
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

  // ====== npx discovery state ======
  const [npxSkills, setNpxSkills] = createSignal<DiscoveredNpxSkill[]>(cachedNpxSkills() ?? []);
  const [npxLoading, setNpxLoading] = createSignal(false);
  const [npxImportingId, setNpxImportingId] = createSignal<string | null>(null);
  const [npxRefreshingId, setNpxRefreshingId] = createSignal<string | null>(null);
  let npxScanned = cachedNpxSkills() !== null; // auto-scan on first npx tab visit, then reuse the cached result

  const discoverNpx = async () => {
    setNpxLoading(true);
    setError(null);
    try {
      const list = await discoverNpxSkills();
      setNpxSkills(list);
      setCachedNpxSkills(list);
      npxScanned = true;
    } catch (e) {
      setError(t('skill.scanFailed', { error: String(e) }));
    } finally {
      setNpxLoading(false);
    }
  };

  const importNpx = async (pkgName: string, sourcePath?: string) => {
    // Safety confirmation: npx import executes unreviewed code from npm
    const confirmed = window.confirm(t('skill.importNpxConfirm', { name: pkgName }));
    if (!confirmed) return;

    setNpxImportingId(pkgName);
    setError(null);
    try {
      const imported = await importNpxSkill(pkgName, projectId(), sourcePath ?? null);
      setSkills({ ...skills(), [imported.id]: imported });
      // Update the alreadyImported flag in the list (keep local signal and module-level cache in sync)
      const next = npxSkills().map((s) =>
        s.packageName === pkgName ? { ...s, alreadyImported: true } : s,
      );
      setNpxSkills(next);
      setCachedNpxSkills(next);
    } catch (e) {
      setError(t('skill.importFailed', { error: String(e) }));
    } finally {
      setNpxImportingId(null);
    }
  };

  const refreshNpx = async (id: string) => {
    // Safety confirmation
    const confirmed = window.confirm(t('skill.refreshConfirm'));
    if (!confirmed) return;

    setNpxRefreshingId(id);
    setError(null);
    try {
      const updated = await refreshNpxSkill(id, projectId());
      setSkills({ ...skills(), [updated.id]: updated });
    } catch (e) {
      setError(t('skill.refreshFailed') + ': ' + e);
    } finally {
      setNpxRefreshingId(null);
    }
  };

  const loadMarket = async (nextSort = sort(), forceRefresh = false) => {
    setLoading(true);
    setError(null);
    try {
      const cache = marketCache.read();
      if (!forceRefresh) {
        const cachedList = cache?.markets[nextSort];
        const cachedAt = cache?.marketUpdatedAt[nextSort] ?? 0;
        const cacheIsFresh = marketCache.isFresh(cachedAt, MARKET_CACHE_TTL_MS);
        if (cachedList && cacheIsFresh) {
          setMarketSkills(cachedList);
          setLastRefreshedAt(new Date(cachedAt));
          return;
        }
      }
      const list = await listSkillMarket({
        sort: nextSort,
        forceRefresh,
      });
      setMarketSkills(list);
      const updatedAt = Date.now();
      marketCache.write({
        updatedAt,
        markets: { ...(cache?.markets ?? {}), [nextSort]: list },
        marketUpdatedAt: { ...(cache?.marketUpdatedAt ?? {}), [nextSort]: updatedAt },
      });
      setLastRefreshedAt(new Date(updatedAt));
    } catch (e) {
      setError(t('skill.loadShFailed', { error: String(e) }));
    } finally {
      setLoading(false);
    }
  };

  onMount(async () => {
    const cacheIsFresh =
      initialCache &&
      marketCache.isFresh(initialCache.marketUpdatedAt.all ?? 0, MARKET_CACHE_TTL_MS) &&
      marketCache.isFresh(initialCache.updatedAt, MARKET_CACHE_TTL_MS);
    setLoading(!initialCache?.markets.all);
    try {
      const localList = await listSkills(projectId());
      setSkills(Object.fromEntries(localList.map((skill) => [skill.id, skill])));
      if (cacheIsFresh) return;

      const marketList = await listSkillMarket({ sort: 'all', forceRefresh: false });
      setMarketSkills(marketList);
      const updatedAt = Date.now();
      marketCache.write({
        updatedAt,
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

  // Watch scope switches and reload the skill list for the new scope
  createEffect(
    on(
      projectId,
      async (pid) => {
        try {
          const list = await listSkills(pid);
          setSkills(Object.fromEntries(list.map((skill) => [skill.id, skill])));
        } catch (e) {
          console.warn('Failed to switch skill scope:', e);
        }
      },
      { defer: true },
    ),
  );

  // Market search: the sort pages only ship the top slice of skills.sh, so a
  // non-empty query ALSO hits the remote search API (debounced) — results cover
  // the full catalog, not just the downloaded page.
  const [searchResults, setSearchResults] = createSignal<MarketSkill[] | null>(null);
  const [searching, setSearching] = createSignal(false);
  let searchTimer: ReturnType<typeof setTimeout> | undefined;

  const runMarketSearch = async (keyword: string) => {
    setSearching(true);
    try {
      setSearchResults(await searchSkillMarket(keyword));
    } catch (e) {
      console.warn('Skill market search failed:', e);
      setSearchResults([]);
    } finally {
      setSearching(false);
    }
  };

  const onQueryInput = (value: string) => {
    setQuery(value);
    if (searchTimer) clearTimeout(searchTimer);
    const keyword = value.trim();
    // Remote search only applies to the market view; downloaded/npx views filter locally
    if (!keyword || view() !== 'market') {
      setSearchResults(null);
      setSearching(false);
      return;
    }
    searchTimer = setTimeout(() => void runMarketSearch(keyword), 300);
  };

  const selectView = (v: ViewMode) => {
    setView(v);
    if (searchTimer) clearTimeout(searchTimer);
    const keyword = query().trim();
    if (v === 'market' && keyword) {
      void runMarketSearch(keyword);
    } else {
      setSearchResults(null);
      setSearching(false);
    }
  };

  const filteredMarketSkills = createMemo(() => {
    const keyword = query().trim().toLowerCase();
    if (!keyword) return marketSkills();
    return marketSkills().filter((skill) =>
      `${skill.name} ${skill.description} ${skill.owner} ${skill.repo}`
        .toLowerCase()
        .includes(keyword),
    );
  });

  // What the market grid shows: remote full-catalog results while searching,
  // otherwise the locally filtered browse slice
  const displayedMarketSkills = createMemo(() => searchResults() ?? filteredMarketSkills());

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

  const selectSort = (value: MarketSort) => {
    setSort(value);
    // Leaving search mode: a sort tab click returns to the browse slice
    setSearchResults(null);
    setQuery('');
    void loadMarket(value);
  };

  const refreshMarket = async () => {
    setRefreshing(true);
    setError(null);
    setRefreshResult(null);
    try {
      const marketList = await listSkillMarket({
        sort: sort(),
        forceRefresh: true,
      });
      setMarketSkills(marketList);
      const updatedAt = Date.now();
      const cache = marketCache.read();
      marketCache.write({
        updatedAt,
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
      const downloaded = await downloadMarketSkill(skill, projectId());
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
      await saveSkill(skill, projectId());
      setSkills({ ...skills(), [skill.id]: skill });
      setEditing(null);
    } catch (e) {
      setError(t('skill.saveFailed', { error: String(e) }));
    }
  };

  const remove = async (id: string) => {
    if (!confirm(t('skill.removeConfirm'))) return;
    try {
      await deleteSkill(id, projectId());
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
              class={btnTabXs}
              classList={{ [tabActive]: view() === 'market' }}
              onClick={() => selectView('market')}
            >
              {t('skill.tabMarket')}
            </button>
            <button
              class={btnTabXs}
              classList={{ [tabActive]: view() === 'downloaded' }}
              onClick={() => selectView('downloaded')}
            >
              {t('skill.tabDownloaded', { count: Object.keys(skills()).length })}
            </button>
            <button
              class={btnTabXs}
              classList={{ [tabActive]: view() === 'npx' }}
              onClick={() => {
                selectView('npx');
                if (!npxScanned) discoverNpx();
              }}
            >
              {t('skill.tabNpx')}
            </button>
          </div>
          {/* Show scope switcher when an active project exists (visible in market/downloaded/npx views) */}
          {currentProjectId() && (
            <div
              class="flex items-center gap-1 p-1 rounded-lg"
              style={{ background: 'rgba(var(--text-base-rgb),0.04)' }}
            >
              <button
                class={btnTabXs}
                classList={{ [tabActive]: scope() === 'global' }}
                onClick={() => setScope('global')}
              >
                {t('skill.scopeGlobal')}
              </button>
              <button
                class={btnTabXs}
                classList={{ [tabActive]: scope() === 'project' }}
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
            class="absolute left-2.5 top-1/2 -translate-y-1/2 text-[rgba(var(--text-base-rgb),0.4)] pointer-events-none"
          />
          <input
            class="w-[280px] max-w-full bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none pl-8 pr-3 py-2 text-sm"
            value={query()}
            onInput={(e) => onQueryInput(e.currentTarget.value)}
            placeholder={t('skill.searchPlaceholder')}
          />
          <Show when={searching()}>
            <Icon
              name="spinner"
              size={13}
              class="absolute right-2.5 top-1/2 -translate-y-1/2 text-[rgba(var(--text-base-rgb),0.4)] animate-spin pointer-events-none"
            />
          </Show>
        </div>
      </div>

      <Show when={view() === 'market'}>
        <div
          class="animate-row-in flex items-center gap-3 flex-wrap"
          style={{ 'animation-delay': '60ms' }}
        >
          <div
            class="flex items-center gap-1 p-1 rounded-lg"
            style={{ background: 'rgba(var(--text-base-rgb),0.04)' }}
          >
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
                  class={btnTabXs}
                  classList={{ [tabActive]: sort() === value }}
                  onClick={() => selectSort(value)}
                >
                  {label}
                </button>
              )}
            </For>
          </div>
          <button
            type="button"
            class={btnBadgeBar + ' ml-auto'}
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
          style={{ background: 'rgba(255,77,77,0.1)', color: 'var(--color-danger)' }}
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
              each={displayedMarketSkills()}
              fallback={
                <div
                  class="col-span-full py-12 text-center text-sm"
                  style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                >
                  {t('skill.noMatch')}
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
                          onClick={() => void openSafeUrl(skill.sourceUrl)}
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
                        class={downloaded() ? btnSuccess : btnSecondarySm}
                        disabled={downloaded() || downloadingId() === skill.id}
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
                      <button class={btnBadge} onClick={() => void openSafeUrl(skill.sourceUrl!)}>
                        {t('skill.source')}
                      </button>
                    </Show>
                    <button class={btnBadge} onClick={() => setEditing({ ...skill })}>
                      {t('skill.edit')}
                    </button>
                    <button class={btnDanger} onClick={() => void remove(skill.id)}>
                      {t('skill.remove')}
                    </button>
                  </div>
                </div>
              )}
            </For>
          </div>
        </Show>

        {/* npx discovery tab */}
        <Show when={view() === 'npx'}>
          <div class="animate-row-in flex flex-col gap-3" style={{ 'animation-delay': '30ms' }}>
            <div class="flex items-center justify-between">
              <button class={btnSecondarySm} onClick={() => discoverNpx()} disabled={npxLoading()}>
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
                  {t('skill.scanningNpx')}
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
                              class={btnSecondarySm}
                              onClick={() => importNpx(item.packageName, item.sourcePath)}
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
                            class={btnBadge}
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
              <button class={btnSecondarySm} onClick={() => setEditing(null)}>
                {t('skill.cancel')}
              </button>
              <button class={btnPrimarySm} onClick={() => void save()}>
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
