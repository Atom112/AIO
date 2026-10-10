/**
 * LocalModelMarket - the standalone "local model market" settings sub-page
 * (route /settings/local-models) for downloading GGUF chat models without a browser.
 *
 * - Discovery: HuggingFace + ModelScope merged rankings (download counts), search-first,
 *   classified by parameter-size segments; hardware-fit models are floated to the top
 *   - Download: one quant-family (all shards) at a time with pause / resume / cancel and
 *   progress events; finished models are registered into the selected local engine
 *   (llama.cpp / Ollama) via providerConfigs, exactly like the manual file import
 * - Icons: rows reuse the lobehub provider logos (same source as the online model list)
 * - Persistence: market query results cached in localStorage (6h TTL, stale-while-revalidate,
 *   same pattern as the skill market); hardware profile cached at module level
 */
import {
  Component,
  createEffect,
  createMemo,
  createSignal,
  For,
  Show,
  onMount,
  onCleanup,
} from 'solid-js';
import { listen } from '@tauri-apps/api/event';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import Icon from '../../../shared/components/Icon';
import {
  btnBadge,
  btnBadgeBar,
  btnDangerBadge,
  btnSecondarySm,
  btnTabXs,
  tabActive,
} from '../../../shared/components/buttonStyles';
import { t, formatNumber } from '../../../core/i18n';
import { formatSize } from '../../../core/utils/format';
import { openSafeUrl } from '../../../core/utils/safeUrl';
import { getLogo } from '../../../core/utils/modelLogo';
import { LOCAL_ENGINE_PROVIDERS, type ProviderConfig } from '../../../core/utils/models';
import { providerConfigs, setProviderConfigs } from '../../../core/store/store';
import { saveProviderConfigs } from '../../../core/api/providers';
import { EVENTS } from '../../../core/api/events';
import {
  localModelCancelDownload,
  localModelDelete,
  localModelDownload,
  localModelHardwareProfile,
  localModelHfTokenGet,
  localModelHfTokenSet,
  localModelInstalledList,
  localModelMarketFiles,
  localModelMarketList,
  localModelPauseDownload,
  localModelsGetDir,
  localModelsSetDir,
} from '../../../core/api/localModel';
import type {
  HardwareProfile,
  InstalledLocalModel,
  MarketFile,
  MarketModel,
} from '../../../core/api/localModel';

const MARKET_CACHE_KEY = 'aio-local-model-market-cache-v1';
const MARKET_CACHE_TTL_MS = 6 * 60 * 60 * 1000;

interface MarketCacheEntry {
  models: MarketModel[];
  failedSources: string[];
  cachedAt: number;
}
interface MarketCache {
  updatedAt: number;
  queries: Record<string, MarketCacheEntry>;
}

function readMarketCache(): MarketCache | null {
  try {
    const raw = localStorage.getItem(MARKET_CACHE_KEY);
    if (!raw) return null;
    const cache = JSON.parse(raw) as MarketCache;
    if (!cache.updatedAt || !cache.queries) return null;
    return cache;
  } catch {
    return null;
  }
}

function writeMarketCache(queryKey: string, entry: MarketCacheEntry) {
  try {
    const cache = readMarketCache() ?? { updatedAt: 0, queries: {} };
    cache.queries[queryKey] = entry;
    cache.updatedAt = Date.now();
    localStorage.setItem(MARKET_CACHE_KEY, JSON.stringify(cache));
  } catch {
    /* storage full / disabled: cache is best-effort */
  }
}

// Hardware profile never changes at runtime: cache at module level so switching
// settings tabs / providers never re-probes.
const [cachedHardware, setCachedHardware] = createSignal<HardwareProfile | null>(null);

/** One downloadable quant-family (a single file, or all shards of a split GGUF). */
interface FileGroup {
  /** Install folder / modelId (family stem, e.g. "Qwen3-14B-Q4_K_M") */
  id: string;
  display: string;
  files: MarketFile[];
  totalSize: number;
  recommended: boolean;
  complete: boolean;
}

const SHARD_RE = /^(.*)-(\d{5})-of-(\d{5})\.gguf$/i;
const RECOMMENDED_QUANT_RE = /(^|[-_])(ud-)?(q4_k_m|q4_k_s|q5_k_m|iq4_xs|ud-q4_k_xl)$/i;

function buildFileGroups(files: MarketFile[]): FileGroup[] {
  const families = new Map<string, MarketFile[]>();
  for (const f of files) {
    const base = f.name.split('/').pop() ?? f.name;
    const m = base.match(SHARD_RE);
    if (m) {
      const list = families.get(m[1]) ?? [];
      list.push(f);
      families.set(m[1], list);
    } else {
      const key = base.replace(/\.gguf$/i, '');
      const list = families.get(key) ?? [];
      list.push(f);
      families.set(key, list);
    }
  }
  const groups: FileGroup[] = [];
  for (const [id, list] of families) {
    list.sort((a, b) => a.name.localeCompare(b.name));
    const first = list[0];
    const base = first.name.split('/').pop() ?? first.name;
    const m = base.match(SHARD_RE);
    // Split GGUF: complete only when every shard 1..N is present
    const complete = m ? list.length >= parseInt(m[3], 10) : true;
    groups.push({
      id,
      display: m ? `${id}.gguf` : base,
      files: list,
      totalSize: list.reduce((s, f) => s + f.size, 0),
      recommended: RECOMMENDED_QUANT_RE.test(id),
      complete,
    });
  }
  groups.sort((a, b) => {
    if (a.recommended !== b.recommended) return a.recommended ? -1 : 1;
    return a.id.localeCompare(b.id);
  });
  return groups;
}

/** Parameter-size segments (total params, memory-fit oriented); labels are literal i18n keys. */
const SEGMENTS: ReadonlyArray<{
  id: string;
  label:
    | 'provider.localModelMarket.segAll'
    | 'provider.localModelMarket.segU1'
    | 'provider.localModelMarket.seg1to3'
    | 'provider.localModelMarket.seg3to7'
    | 'provider.localModelMarket.seg7to14'
    | 'provider.localModelMarket.seg14to32'
    | 'provider.localModelMarket.seg32plus';
  test: (b: number | null) => boolean;
}> = [
  { id: 'all', label: 'provider.localModelMarket.segAll', test: () => true },
  { id: 'u1', label: 'provider.localModelMarket.segU1', test: (b) => b !== null && b <= 1 },
  {
    id: '1-3',
    label: 'provider.localModelMarket.seg1to3',
    test: (b) => b !== null && b > 1 && b <= 3,
  },
  {
    id: '3-7',
    label: 'provider.localModelMarket.seg3to7',
    test: (b) => b !== null && b > 3 && b <= 7,
  },
  {
    id: '7-14',
    label: 'provider.localModelMarket.seg7to14',
    test: (b) => b !== null && b > 7 && b <= 14,
  },
  {
    id: '14-32',
    label: 'provider.localModelMarket.seg14to32',
    test: (b) => b !== null && b > 14 && b <= 32,
  },
  { id: '32plus', label: 'provider.localModelMarket.seg32plus', test: (b) => b !== null && b > 32 },
];

/** Engines that can load GGUF files (the market registers finished models into one of them). */
const GGUF_ENGINES = LOCAL_ENGINE_PROVIDERS.filter((p) => p.allowModelFileSelection);

const LocalModelMarket: Component = () => {
  /** Target engine for registration (llama.cpp / Ollama); download proxy follows its provider config */
  const [engineId, setEngineId] = createSignal<string>(GGUF_ENGINES[0]?.id ?? 'local-llamacpp');
  const engineDef = () => GGUF_ENGINES.find((p) => p.id === engineId()) ?? GGUF_ENGINES[0];
  const [query, setQuery] = createSignal('');
  const [debouncedQuery, setDebouncedQuery] = createSignal('');
  const [segment, setSegment] = createSignal('all');
  const [models, setModels] = createSignal<MarketModel[]>([]);
  const [failedSources, setFailedSources] = createSignal<string[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal('');
  const [expandedId, setExpandedId] = createSignal<string | null>(null);
  const [fileGroups, setFileGroups] = createSignal<Record<string, FileGroup[]>>({});
  const [filesLoading, setFilesLoading] = createSignal<Record<string, boolean>>({});
  const [filesError, setFilesError] = createSignal<Record<string, string>>({});
  const [progress, setProgress] = createSignal<
    Record<string, { file: string; downloaded: number; total: number; source: string }>
  >({});
  const [downloading, setDownloading] = createSignal<Record<string, boolean>>({});
  const [pausedIds, setPausedIds] = createSignal<Record<string, boolean>>({});
  const [installed, setInstalled] = createSignal<InstalledLocalModel[]>([]);
  const [installDir, setInstallDir] = createSignal('');
  const [tokenInput, setTokenInput] = createSignal('');
  const [tokenSaved, setTokenSaved] = createSignal(false);
  const [notice, setNotice] = createSignal('');

  /** Provider logo (lobehub) for a market row: org table first, then model-name rules */
  const rowLogo = (m: MarketModel) => getLogo(m.org, m.name);

  const engineProxyUrl = () => providerConfigs()[engineId()]?.proxyUrl;

  /**
   * Registers a downloaded model into the selected engine provider (the same shape as the
   * manual GGUF import on the provider detail page: localModelPaths + enabledModels).
   */
  const registerModel = async (modelId: string, entryPath: string) => {
    const pid = engineId();
    const def = engineDef();
    const cfgs = providerConfigs();
    const cur = cfgs[pid];
    const next: ProviderConfig = {
      ...(cur ?? {
        id: pid,
        enabled: false,
        displayName: def?.name ?? pid,
        apiUrl: def?.defaultApiUrl ?? '',
        apiKey: '',
        enabledModels: [],
        isCustom: true,
        customModelIds: [],
      }),
      id: pid,
      enabled: true,
      enabledModels: (cur?.enabledModels ?? []).includes(modelId)
        ? cur!.enabledModels
        : [...(cur?.enabledModels ?? []), modelId],
      localModelPaths: { ...(cur?.localModelPaths ?? {}), [modelId]: entryPath },
    };
    const map = { ...cfgs, [pid]: next };
    const file = { version: 2, updatedAt: String(Date.now()), providers: map };
    await saveProviderConfigs(file);
    setProviderConfigs(map);
  };

  const hardware = () => cachedHardware();
  /** Whether a model's parameter size fits the hardware recommendation. */
  const fitsHardware = (m: MarketModel) => {
    const h = hardware();
    if (!h || m.paramsB === null) return false;
    return m.paramsB <= h.recommendedParamsB && m.paramsB >= h.recommendedParamsB / 3;
  };

  const queryKey = () => debouncedQuery().trim().toLowerCase() || 'default';

  const loadMarket = async (queryText: string, silent: boolean) => {
    if (!silent) setLoading(true);
    setError('');
    try {
      const key = queryText.trim().toLowerCase() || 'default';
      const resp = await localModelMarketList(queryText.trim(), 50);
      setModels(resp.models);
      setFailedSources(resp.failedSources);
      writeMarketCache(key, {
        models: resp.models,
        failedSources: resp.failedSources,
        cachedAt: Date.now(),
      });
    } catch (e) {
      if (!silent) setError(String(e));
    } finally {
      if (!silent) setLoading(false);
    }
  };

  /** Cache-first render: stale-while-revalidate (fresh cache skips the network). */
  const applyCache = () => {
    const cache = readMarketCache();
    const entry = cache?.queries[queryKey()];
    if (entry) {
      setModels(entry.models);
      setFailedSources(entry.failedSources);
      if (Date.now() - entry.cachedAt < MARKET_CACHE_TTL_MS) return;
    }
    void loadMarket(debouncedQuery(), true);
  };

  // Debounced search: typing re-queries after 500ms
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  const onQueryInput = (value: string) => {
    setQuery(value);
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => setDebouncedQuery(value), 500);
  };
  onCleanup(() => {
    if (searchTimer) clearTimeout(searchTimer);
  });

  // Query change re-applies the cache (segment filtering is client-side)
  createEffect(() => {
    void queryKey();
    applyCache();
  });

  const refreshInstalled = async () => {
    try {
      setInstalled(await localModelInstalledList());
    } catch {
      /* non-fatal */
    }
  };

  onMount(() => {
    if (!cachedHardware()) {
      localModelHardwareProfile()
        .then(setCachedHardware)
        .catch(() => {});
    }
    void localModelsGetDir()
      .then(setInstallDir)
      .catch(() => {});
    void localModelHfTokenGet()
      .then((tok) => setTokenInput(tok ?? ''))
      .catch(() => {});
    void refreshInstalled();
  });

  const visibleModels = createMemo(() => {
    const segId = segment();
    const seg = SEGMENTS.find((s) => s.id === segId);
    const list = seg ? models().filter((m) => seg.test(m.paramsB)) : models();
    // Hardware-fit models float to the top (the recommendation row zone); the profile is
    // hoisted out of the comparator so no signal read happens inside the sort callback.
    const h = cachedHardware();
    const fit = (m: MarketModel): number =>
      h &&
      m.paramsB !== null &&
      m.paramsB <= h.recommendedParamsB &&
      m.paramsB >= h.recommendedParamsB / 3
        ? 0
        : 1;
    return [...list].sort((a, b) => fit(a) - fit(b));
  });

  const toggleExpand = async (m: MarketModel) => {
    if (expandedId() === m.id) {
      setExpandedId(null);
      return;
    }
    setExpandedId(m.id);
    if (fileGroups()[m.id]) return;
    setFilesLoading((prev) => ({ ...prev, [m.id]: true }));
    setFilesError((prev) => ({ ...prev, [m.id]: '' }));
    try {
      const sources: Array<{ source: string; repo: string | null }> = [
        { source: 'modelscope', repo: m.msRepo },
        { source: 'huggingface', repo: m.hfRepo },
      ];
      const results = await Promise.all(
        sources
          .filter((s) => s.repo)
          .map((s) => localModelMarketFiles(s.source, s.repo!).catch(() => null)),
      );
      const all = results.flatMap((r) => r ?? []);
      if (all.length === 0) {
        setFilesError((prev) => ({
          ...prev,
          [m.id]: t('provider.localModelMarket.noFiles'),
        }));
        return;
      }
      // Merge by lowercase file name (same file may exist on both platforms)
      const byName = new Map<string, MarketFile>();
      for (const f of all) {
        const key = f.name.toLowerCase();
        const existing = byName.get(key);
        if (!existing || f.size > existing.size) byName.set(key, f);
      }
      setFileGroups((prev) => ({
        ...prev,
        [m.id]: buildFileGroups([...byName.values()]),
      }));
    } finally {
      setFilesLoading((prev) => ({ ...prev, [m.id]: false }));
    }
  };

  const pct = (id: string) => {
    const p = progress()[id];
    if (!p || p.total <= 0) return 0;
    return Math.min(100, Math.round((p.downloaded / p.total) * 100));
  };

  const downloadGroup = async (m: MarketModel, g: FileGroup) => {
    if (!g.complete) return;
    const id = g.id;
    setDownloading((prev) => ({ ...prev, [id]: true }));
    setPausedIds((prev) => ({ ...prev, [id]: false }));
    setProgress((prev) => ({ ...prev, [id]: { file: '', downloaded: 0, total: 0, source: '' } }));
    let unlisten: (() => void) | undefined;
    try {
      unlisten = await listen(EVENTS.localModelDownloadProgress, (ev) => {
        const p = ev.payload as {
          id?: string;
          file?: string;
          downloaded?: number;
          total?: number;
          source?: string;
          done?: boolean;
        };
        if (!p || p.id !== id) return;
        if (p.done) {
          setProgress((prev) => {
            const next = { ...prev };
            delete next[id];
            return next;
          });
          return;
        }
        setProgress((prev) => ({
          ...prev,
          [id]: {
            file: p.file ?? '',
            downloaded: p.downloaded ?? 0,
            total: p.total ?? 0,
            source: p.source ?? '',
          },
        }));
      });
      const outcome = await localModelDownload({
        id,
        hfRepo: m.hfRepo,
        msRepo: m.msRepo,
        files: g.files.map((f) => ({ name: f.name, size: f.size })),
        proxyUrl: engineProxyUrl(),
      });
      await refreshInstalled();
      if (outcome.status === 'ok' && outcome.entryPath) {
        await registerModel(id, outcome.entryPath);
        setNotice(t('provider.localModelMarket.downloaded', { name: id }));
      } else if (outcome.status === 'paused') {
        setPausedIds((prev) => ({ ...prev, [id]: true }));
        setNotice(t('provider.localModelMarket.downloadPaused', { name: id }));
      } else {
        setNotice(t('provider.localModelMarket.downloadCancelled', { name: id }));
      }
    } catch (e) {
      setNotice(t('provider.localModelMarket.downloadFail', { error: String(e) }));
    } finally {
      unlisten?.();
      setDownloading((prev) => ({ ...prev, [id]: false }));
      setProgress((prev) => {
        const next = { ...prev };
        delete next[id];
        return next;
      });
    }
  };

  const pauseDownload = (id: string) => {
    void localModelPauseDownload(id);
  };

  const cancelDownload = async (id: string) => {
    const wasActive = downloading()[id];
    try {
      await localModelCancelDownload(id);
    } catch {
      /* ignore */
    }
    if (!wasActive) {
      setPausedIds((prev) => ({ ...prev, [id]: false }));
      setNotice(t('provider.localModelMarket.downloadCancelled', { name: id }));
    }
  };

  const removeInstalled = async (m: InstalledLocalModel) => {
    if (!window.confirm(t('provider.localModelMarket.deleteConfirm', { name: m.id }))) return;
    try {
      await localModelDelete(m.id);
      await refreshInstalled();
      setNotice(t('provider.localModelMarket.deleted', { name: m.id }));
    } catch (e) {
      setNotice(t('provider.localModelMarket.deleteFail', { error: String(e) }));
    }
  };

  const pickDir = async () => {
    try {
      const dir = await openDialog({ directory: true, multiple: false });
      if (dir && typeof dir === 'string') {
        const effective = await localModelsSetDir(dir);
        setInstallDir(effective);
        setNotice(t('provider.localModelMarket.dirSaved'));
        await refreshInstalled();
      }
    } catch {
      /* user cancelled */
    }
  };

  const resetDir = async () => {
    try {
      const effective = await localModelsSetDir('');
      setInstallDir(effective);
      setNotice(t('provider.localModelMarket.dirSaved'));
      await refreshInstalled();
    } catch {
      /* ignore */
    }
  };

  const saveToken = async () => {
    try {
      await localModelHfTokenSet(tokenInput().trim());
      setTokenSaved(true);
      setNotice(t('provider.localModelMarket.tokenSaved'));
      setTimeout(() => setTokenSaved(false), 2000);
    } catch (e) {
      setNotice(t('provider.localModelMarket.tokenSaveFail', { error: String(e) }));
    }
  };

  const renderModelRow = (m: MarketModel, index: () => number) => {
    const groups = () => fileGroups()[m.id] ?? [];
    const isExpanded = () => expandedId() === m.id;
    const isLoadingFiles = () => filesLoading()[m.id];
    const filesErr = () => filesError()[m.id];
    return (
      <div
        class="rounded-[10px] border border-white/[0.05] bg-white/[0.02] animate-row-in"
        style={{ 'animation-delay': (index() + 1) * 30 + 'ms' }}
      >
        <button
          type="button"
          class="w-full flex items-center gap-3 px-3 py-2.5 text-left"
          onClick={() => void toggleExpand(m)}
        >
          <div class="flex items-center justify-center w-9 h-9 rounded-lg bg-white border border-white/85 shadow-[0_1px_3px_rgba(0,0,0,0.15)] overflow-hidden shrink-0">
            {rowLogo(m) ? (
              <img src={rowLogo(m)!} alt={m.org} class="w-5 h-5 object-contain" />
            ) : (
              <span class="font-bold text-[15px]" style={{ color: 'var(--color-dark-700)' }}>
                {(m.org || m.name).charAt(0).toUpperCase()}
              </span>
            )}
          </div>
          <div class="grow min-w-0">
            <div class="flex items-center gap-2 flex-wrap">
              <span class="text-[13px] text-white/85 font-medium truncate">{m.name}</span>
              <Show when={m.paramsB !== null}>
                <span class="px-1 py-0.5 rounded bg-white/10 text-[10px] shrink-0 font-mono">
                  {m.paramsB}B
                </span>
              </Show>
              <Show when={fitsHardware(m)}>
                <span class="px-1 py-0.5 rounded bg-pri-20 text-pri text-[10px] shrink-0">
                  {t('provider.localModelMarket.hwFit')}
                </span>
              </Show>
              <Show when={m.license}>
                <span
                  class="px-1 py-0.5 rounded text-[10px] shrink-0"
                  style={{ background: 'rgba(var(--text-base-rgb),0.08)' }}
                >
                  {m.license}
                </span>
              </Show>
            </div>
            <div class="text-[10px] mt-0.5" style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}>
              {m.org}
              <Show when={m.chineseName}>
                <span class="ml-1.5">{m.chineseName}</span>
              </Show>
              <span class="ml-1.5 opacity-70">
                {formatNumber(m.downloads)} {t('provider.localModelMarket.downloads')}
              </span>
              <For each={m.sources}>
                {(s) => (
                  <span class="ml-1.5 opacity-70">
                    {s === 'huggingface' ? 'HuggingFace' : 'ModelScope'}
                  </span>
                )}
              </For>
            </div>
          </div>
          <Show
            when={!isLoadingFiles()}
            fallback={<Icon name="spinner" size={12} class="animate-spin shrink-0" />}
          >
            <Icon
              name={isExpanded() ? 'caret-up' : 'caret-down'}
              size={12}
              class="shrink-0 text-[rgba(var(--text-base-rgb),0.4)]"
            />
          </Show>
        </button>
        <Show when={isExpanded()}>
          <div class="px-3 pb-3 space-y-1.5">
            <Show when={filesErr()}>
              <div class="text-[11px] text-red-300/80">{filesErr()}</div>
            </Show>
            <For each={groups()}>
              {(g) => {
                const isInstalled = () => installed().some((i) => i.id === g.id);
                const isDownloading = () => downloading()[g.id];
                const isPaused = () => pausedIds()[g.id];
                const dlProgress = () => progress()[g.id];
                return (
                  <div class="flex items-center gap-2 px-3 py-2 rounded-lg bg-white/[0.03] border border-white/[0.06] flex-wrap">
                    <span class="text-[11px] text-white/75 font-mono truncate grow min-w-[120px] max-w-full">
                      {g.display}
                      <Show when={!g.complete}>
                        <span class="ml-1.5 text-red-300/80">
                          {t('provider.localModelMarket.incompleteShards')}
                        </span>
                      </Show>
                    </span>
                    <Show when={g.recommended}>
                      <span class="px-1 py-0.5 rounded bg-pri-20 text-pri text-[10px] shrink-0">
                        {t('provider.localModelMarket.recommendedQuant')}
                      </span>
                    </Show>
                    <span
                      class="text-[10px] shrink-0"
                      style={{ color: 'rgba(var(--text-base-rgb),0.45)' }}
                    >
                      {formatSize(g.totalSize)}
                    </span>
                    <Show when={dlProgress()}>
                      <span
                        class="text-[10px] shrink-0"
                        style={{ color: 'rgba(var(--text-base-rgb),0.45)' }}
                      >
                        {pct(g.id)}%
                      </span>
                    </Show>
                    <div class="flex items-center gap-1.5 shrink-0">
                      <Show
                        when={!isInstalled()}
                        fallback={
                          <span class="text-[11px] text-green-400/70">
                            {t('provider.localModelMarket.installed')}
                          </span>
                        }
                      >
                        <Show
                          when={!isDownloading()}
                          fallback={
                            <>
                              <button
                                type="button"
                                class={btnSecondarySm}
                                onClick={() => pauseDownload(g.id)}
                              >
                                <Icon name="pause" size={12} />
                                {t('provider.localModelMarket.pause')}
                              </button>
                              <button
                                type="button"
                                class={btnSecondarySm}
                                onClick={() => void cancelDownload(g.id)}
                              >
                                <Icon name="x" size={12} />
                                {t('common.cancel')}
                              </button>
                            </>
                          }
                        >
                          <button
                            type="button"
                            class={btnSecondarySm}
                            disabled={!g.complete}
                            onClick={() => void downloadGroup(m, g)}
                          >
                            <Icon name={isPaused() ? 'play' : 'download'} size={12} />
                            {isPaused()
                              ? t('provider.localModelMarket.resume')
                              : t('provider.localModelMarket.download')}
                          </button>
                          <Show when={isPaused()}>
                            <button
                              type="button"
                              class={btnSecondarySm}
                              onClick={() => void cancelDownload(g.id)}
                            >
                              <Icon name="x" size={12} />
                              {t('common.cancel')}
                            </button>
                          </Show>
                        </Show>
                      </Show>
                    </div>
                    <Show when={dlProgress()}>
                      <div class="w-full">
                        <div class="h-1 w-full overflow-hidden rounded bg-white/10">
                          <div
                            class="h-1 rounded bg-pri transition-[width] duration-200"
                            style={{ width: pct(g.id) + '%' }}
                          />
                        </div>
                        <div
                          class="text-right text-[10px]"
                          style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                        >
                          {t('provider.localModelMarket.downloadingPct', {
                            file: dlProgress()!.file.split('/').pop() ?? '',
                            pct: String(pct(g.id)),
                            source: dlProgress()!.source,
                          })}
                        </div>
                      </div>
                    </Show>
                  </div>
                );
              }}
            </For>
          </div>
        </Show>
      </div>
    );
  };

  return (
    <div
      class="flex flex-col h-full overflow-hidden p-6 gap-4"
      style={{ color: 'rgba(var(--text-base-rgb),0.88)' }}
    >
      <div class="animate-row-in flex items-start justify-between gap-4 flex-wrap">
        <h2 class="text-xl font-semibold">{t('provider.localModelMarket.title')}</h2>
        <div class="flex items-center gap-3 flex-wrap">
          {/* Engine target for registration */}
          <div class="flex items-center gap-2">
            <span class="text-xs" style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}>
              {t('provider.localModelMarket.engineLabel')}
            </span>
            <div
              class="flex items-center gap-1 p-1 rounded-lg"
              style={{ background: 'rgba(var(--text-base-rgb),0.04)' }}
            >
              <For each={GGUF_ENGINES}>
                {(e) => (
                  <button
                    class={btnTabXs}
                    classList={{ [tabActive]: engineId() === e.id }}
                    onClick={() => setEngineId(e.id)}
                  >
                    {e.name}
                  </button>
                )}
              </For>
            </div>
          </div>
          <button
            type="button"
            class={btnBadgeBar}
            disabled={loading()}
            onClick={() => void loadMarket(debouncedQuery(), false)}
          >
            <Show when={loading()} fallback={<Icon name="refresh" size={11} />}>
              <Icon name="spinner" size={11} class="animate-spin" />
            </Show>
            {loading() ? t('provider.localModelMarket.refreshing') : t('common.refresh')}
          </button>
        </div>
      </div>
      <div class="text-xs text-[rgba(var(--text-base-rgb),0.55)] animate-row-in">
        {t('provider.localModelMarket.hint')}
      </div>

      {/* Hardware recommendation banner */}
      <Show when={hardware()}>
        <div
          class="flex items-center gap-2 px-3 py-2 rounded-lg border animate-row-in shrink-0"
          style={{
            background: 'rgba(var(--primary-rgb),0.08)',
            'border-color': 'rgba(var(--primary-rgb),0.25)',
          }}
        >
          <Icon name="cpu" size={14} class="text-pri shrink-0" />
          <span class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.75)' }}>
            {t('provider.localModelMarket.hwBanner', {
              params: String(hardware()!.recommendedParamsB),
              cpu: hardware()!.cpuBrand,
              gpu: hardware()!.gpuName ?? t('provider.localModelMarket.noDiscreteGpu'),
              ram: formatSize(hardware()!.totalRamBytes),
            })}
          </span>
        </div>
      </Show>

      {/* Search + parameter segments */}
      <div class="flex items-center gap-3 flex-wrap shrink-0">
        <div class="relative">
          <Icon
            name="search"
            size={13}
            class="absolute left-2.5 top-1/2 -translate-y-1/2 text-[rgba(var(--text-base-rgb),0.4)] pointer-events-none"
          />
          <input
            class="w-[240px] max-w-full bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none pl-8 pr-3 py-2 text-sm"
            value={query()}
            onInput={(e) => onQueryInput(e.currentTarget.value)}
            placeholder={t('provider.localModelMarket.searchPlaceholder')}
          />
        </div>
        <div
          class="flex items-center gap-1 p-1 rounded-lg"
          style={{ background: 'rgba(var(--text-base-rgb),0.04)' }}
        >
          <For each={SEGMENTS}>
            {(s) => (
              <button
                class={btnTabXs}
                classList={{ [tabActive]: segment() === s.id }}
                onClick={() => setSegment(s.id)}
              >
                {t(s.label)}
              </button>
            )}
          </For>
        </div>
      </div>

      <Show when={failedSources().length > 0}>
        <div class="text-[10px] text-amber-300/70">
          {t('provider.localModelMarket.partialSources', {
            sources: failedSources().join(', '),
          })}
        </div>
      </Show>
      <Show when={error()}>
        <div class="text-[11px] text-red-300/80">{error()}</div>
      </Show>
      <Show when={notice()}>
        <div class="text-[11px] text-green-400/70">{notice()}</div>
      </Show>

      {/* Model list (fills the remaining height; rows expand inline) */}
      <Show
        when={visibleModels().length > 0}
        fallback={
          <div class="text-xs text-[rgba(var(--text-base-rgb),0.45)] py-4 text-center">
            {loading()
              ? t('provider.localModelMarket.loading')
              : t('provider.localModelMarket.empty')}
          </div>
        }
      >
        <div class="flex-1 min-h-0 overflow-y-auto space-y-1.5 pr-1">
          <For each={visibleModels()}>{renderModelRow}</For>
        </div>
      </Show>

      {/* Installed models */}
      <Show when={installed().length > 0}>
        <div class="shrink-0">
          <div class="section-label mb-2">{t('provider.localModelMarket.installedTitle')}</div>
          <div class="space-y-1.5 max-h-[160px] overflow-y-auto pr-1">
            <For each={installed()}>
              {(m) => (
                <div class="flex items-center gap-2 px-3 py-2 rounded-lg bg-white/[0.03] border border-white/[0.06]">
                  <Icon name="file" size={13} />
                  <span class="flex-1 text-[11px] text-white/75 font-mono truncate">{m.id}</span>
                  <span
                    class="text-[10px] shrink-0"
                    style={{ color: 'rgba(var(--text-base-rgb),0.45)' }}
                  >
                    {formatSize(m.sizeBytes)}
                  </span>
                  <button
                    type="button"
                    class={btnDangerBadge}
                    onClick={() => void removeInstalled(m)}
                  >
                    {t('common.delete')}
                  </button>
                </div>
              )}
            </For>
          </div>
        </div>
      </Show>

      {/* Install directory + optional HF token */}
      <div
        class="shrink-0 space-y-2 text-[11px]"
        style={{ color: 'rgba(var(--text-base-rgb),0.55)' }}
      >
        <div class="flex items-center gap-2 flex-wrap">
          <span class="shrink-0">{t('provider.localModelMarket.dirLabel')}</span>
          <span class="font-mono truncate grow min-w-[120px]" title={installDir()}>
            {installDir()}
          </span>
          <button type="button" class={btnSecondarySm} onClick={() => void pickDir()}>
            {t('provider.localModelMarket.changeDir')}
          </button>
          <button type="button" class={btnBadge} onClick={() => void resetDir()}>
            {t('provider.localModelMarket.resetDir')}
          </button>
        </div>
        <div class="flex items-center gap-2 flex-wrap">
          <span class="shrink-0">{t('provider.localModelMarket.tokenLabel')}</span>
          <input
            type="password"
            class="grow min-w-[120px] bg-black/25 border border-white/[0.08] rounded-lg text-white placeholder:text-white/30 focus:outline-none px-2 py-1.5 text-[11px] font-mono"
            value={tokenInput()}
            onInput={(e) => setTokenInput(e.currentTarget.value)}
            placeholder={t('provider.localModelMarket.tokenPlaceholder')}
          />
          <button type="button" class={btnSecondarySm} onClick={() => void saveToken()}>
            <Show when={tokenSaved()}>
              <Icon name="check-circle" size={12} />
            </Show>
            {t('common.save')}
          </button>
          <button
            type="button"
            class={btnBadge}
            onClick={() => void openSafeUrl('https://huggingface.co/settings/tokens')}
          >
            {t('provider.localModelMarket.getToken')}
          </button>
        </div>
        <div class="text-[10px]" style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}>
          {t('provider.localModelMarket.tokenHint')}
        </div>
      </div>
    </div>
  );
};

export default LocalModelMarket;
