/**
 * Provider detail page (LobeHub v2)
 * Route: /settings/provider/:providerId
 *
 * Data flow:
 * - Enabled state / enabled model list / form fields: written directly to providerConfigs + disk, no save button
 * - Toggles (model toggle / provider enabled / orphan remove): auto-save immediately
 * - Text fields (URL/Key/Proxy/displayName): auto-save on input
 */
import { Component, createSignal, createMemo, Show, For, onMount, onCleanup } from 'solid-js';
import { useNavigate, useParams } from '@solidjs/router';
import {
  saveProviderConfigs,
  testProviderConnection,
  fetchProviderModels,
} from '../../core/api/providers';
import { probeEngineHealth, startLocalServer, stopLocalServer } from '../../core/api/engine';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import {
  providerConfigs,
  setProviderConfigs,
  modelsCatalog,
  engineScanResults,
  initEngineScan,
} from '../../core/store/store';
import { loadModelsCatalog } from '../../core/utils/models';
import {
  type ProviderConfig,
  type FetchedModel,
  type ModelMeta,
  type ProviderMeta,
  listProviderModels,
} from '../../core/utils/models';
import { getLocalEngineProvider } from '../../core/utils/models';
import { getProviderLogo } from '../../core/utils/modelLogo';
import ModelRow from '../../shared/components/ModelRow';
import Icon from '../../shared/components/Icon';
import Dropdown from '../../shared/components/Dropdown';
// 阶段4（设置样板收敛）: shared glow switch + toast (was per-page inline copies)
import Switch from '../../shared/components/Switch';
import { ToastBubble, useToast } from '../../shared/components/Toast';
import {
  btnDangerOutline,
  btnIcon,
  btnSecondarySm,
  btnSuccessOutline,
} from '../../shared/components/buttonStyles';
import { formatNumber, reportError, t } from '../../core/i18n';

type SortKey = 'releaseDesc' | 'nameAsc';

/**
 * FE-4: order-insensitive content equality for fetched model lists, used by the
 * Ollama liveness poll to skip no-op disk writes (the poll fires every 5s).
 */
const sameFetchedModels = (a: FetchedModel[] | undefined, b: FetchedModel[]): boolean => {
  if (!a || a.length !== b.length) return false;
  const key = (m: FetchedModel) =>
    `${m.id}|${m.ownedBy ?? ''}|${m.displayName ?? ''}|${m.releasedAt ?? ''}`;
  const sa = new Set(a.map(key));
  return b.every((m) => sa.has(key(m)));
};

const ProviderDetail: Component = () => {
  const params = useParams<{ providerId: string }>();
  const navigate = useNavigate();

  const providerId = () => decodeURIComponent(params.providerId);
  const isCustom = () => providerId().startsWith('custom-');
  const localEngineDef = () => getLocalEngineProvider(providerId());
  const isLocalEngine = () => localEngineDef() !== undefined;

  // ===== catalog / state =====
  const [catalogReady, setCatalogReady] = createSignal(false);
  // 阶段4（设置样板收敛）: shared toast state (timer auto-cleared on replace + dispose)
  const { toast, showToast } = useToast();

  const [testState, setTestState] = createSignal<{
    status: 'idle' | 'testing' | 'ok' | 'fail';
    msg?: string;
    sampleModels?: string[];
  }>({ status: 'idle' });
  const [fetchState, setFetchState] = createSignal<{
    status: 'idle' | 'fetching' | 'ok' | 'fail';
    msg?: string;
    models?: FetchedModel[];
  }>({ status: 'idle' });

  const [search, setSearch] = createSignal('');
  const [sortKey, setSortKey] = createSignal<SortKey>('releaseDesc');
  const [isServerAlive, setIsServerAlive] = createSignal<'checking' | 'alive' | 'dead'>('checking');
  // ===== engine install state (scanned once at startup + refreshed on page entry) =====
  const engineInstallInfo = createMemo(() => {
    if (!isLocalEngine()) return null;
    const results = engineScanResults();
    if (!results) return null; // scan not finished yet
    const match = results.find((r) => r.id === localEngineDef()!.engineType);
    return match ? { installed: match.installed, version: match.version } : null;
  });
  const [engineActionLoading, setEngineActionLoading] = createSignal(false);

  onMount(async () => {
    if (!modelsCatalog()) {
      await loadModelsCatalog();
    }
    setCatalogReady(true);
    // Refresh install state once when opening the local engine page (the startup scan may be stale, e.g. an engine installed since)
    if (isLocalEngine()) {
      await initEngineScan();
    }
  });

  // ===== derived =====
  const cat = () => modelsCatalog();
  const isCatalogProvider = () =>
    !isCustom() && !!cat()?.providers.find((p: ProviderMeta) => p.id === providerId());

  /** The user's current config for this provider (from the store) */
  const userCfg = () => providerConfigs()[providerId()] ?? null;

  // FE-4: persistence serialization + typing-burst debounce state
  const DEBOUNCE_MS = 400;
  let persistChain: Promise<void> = Promise.resolve();
  let pendingOverrides: Partial<ProviderConfig> | null = null;
  let persistTimer: number | undefined;

  /** Provider metadata from the catalog; falls back (e.g. local engine) when not found */
  const providerMeta = createMemo(() => {
    const c = cat();
    if (!c) return null;
    if (isCustom()) {
      const cfg = providerConfigs()[providerId()];
      return cfg
        ? {
            id: providerId(),
            name: cfg.displayName,
            modelCount: c.models.filter((m: ModelMeta) => m.provider === providerId()).length,
            isCustom: true,
          }
        : null;
    }
    const catMeta = c.providers.find((p: ProviderMeta) => p.id === providerId());
    if (catMeta) return catMeta;
    // local engine fallback
    if (isLocalEngine()) {
      const def = localEngineDef()!;
      return {
        id: def.id,
        name: def.name,
        modelCount: userCfg()?.enabledModels?.length ?? 0,
        isLocalEngine: true,
      };
    }
    return null;
  });

  /**
   * Persists to disk and updates the store. `overrides` supports partial updates
   * (e.g. toggling enabledModels).
   *
   * FE-4: writes are SERIALIZED (one in-flight save; each queued job re-reads the
   * current config at execution time, so two quick toggles can no longer race the
   * read-modify-write and silently drop one of them). Callers that pass
   * `debounce: true` (text-field keystrokes) coalesce through a 400ms window.
   */
  const persist = (overrides: Partial<ProviderConfig> = {}, opts?: { debounce?: boolean }) => {
    // The write job re-reads the store at execution time (FE-4 serialization); the
    // reads intentionally happen outside tracked scopes (async chain + setTimeout).
    const doWrite = (merged: Partial<ProviderConfig>) => {
      persistChain = persistChain
        // eslint-disable-next-line solid/reactivity -- deliberate: the queued write re-resolves the current config when it actually runs
        .then(async () => {
          const cur = providerConfigs()[providerId()] ?? null;
          const meta = providerMeta();
          const localDef = localEngineDef();
          const next: ProviderConfig = {
            id: providerId(),
            enabled: cur?.enabled ?? false,
            displayName: cur?.displayName ?? localDef?.name ?? meta?.name ?? providerId(),
            apiUrl: cur?.apiUrl ?? localDef?.defaultApiUrl ?? defaultApiUrl(providerId()),
            apiKey: cur?.apiKey ?? '',
            proxyUrl: cur?.proxyUrl,
            enabledModels: cur?.enabledModels ?? [],
            isCustom: isCustom() || isLocalEngine(),
            customModelIds: cur?.customModelIds ?? [],
            fetchedModels: cur?.fetchedModels,
            localModelPaths: cur?.localModelPaths,
            ...merged,
          };
          const map = { ...providerConfigs() };
          if (next.enabled || next.apiKey || next.enabledModels.length > 0 || next.isCustom) {
            map[next.id] = next;
          } else {
            delete map[next.id];
          }
          const file = { version: 2, updatedAt: String(Date.now()), providers: map };
          await saveProviderConfigs(file);
          setProviderConfigs(map);
        })
        .catch((e) => {
          showToast(reportError('error.save', e), false);
        });
      return persistChain;
    };

    if (!opts?.debounce) {
      return doWrite(overrides);
    }
    // Debounced path: merge pending overrides and flush once the typing settles
    pendingOverrides = { ...(pendingOverrides ?? {}), ...overrides };
    if (persistTimer !== undefined) {
      clearTimeout(persistTimer);
    }
    persistTimer = window.setTimeout(() => {
      persistTimer = undefined;
      const merged = pendingOverrides;
      pendingOverrides = null;
      if (merged) void doWrite(merged);
    }, DEBOUNCE_MS);
    return persistChain;
  };

  /** Form field onInput: FE-4 — debounced save (one disk write per typing burst, not per keystroke) */
  const updateField = <K extends keyof ProviderConfig>(key: K, value: ProviderConfig[K]) => {
    persist({ [key]: value } as Partial<ProviderConfig>, { debounce: true });
  };

  /** Model enable toggle: persists immediately */
  const toggleModel = (modelId: string) => {
    const cur = userCfg()?.enabledModels ?? [];
    const next = cur.includes(modelId) ? cur.filter((m) => m !== modelId) : [...cur, modelId];
    persist({ enabledModels: next });
  };

  /** Orphan model removal */
  const removeOrphan = (modelId: string) => {
    const cur = userCfg()?.enabledModels ?? [];
    persist({ enabledModels: cur.filter((m) => m !== modelId) });
  };

  /** Provider enable toggle */
  const toggleProviderEnabled = (enabled: boolean) => {
    persist({ enabled });
  };

  /** Pick a local model file (GGUF, etc.) */
  const pickLocalModelFile = async () => {
    if (!localEngineDef()) return;
    try {
      const file = await openDialog({
        multiple: false,
        filters: [{ name: localEngineDef()!.name, extensions: localEngineDef()!.fileExtensions }],
      });
      if (file && typeof file === 'string') {
        const fileName = file.split(/[\\/]/).pop() || 'model';
        const modelId = fileName.replace(/\.[^.]+$/, '');
        const current = userCfg()?.localModelPaths ?? {};
        const next = { ...current, [modelId]: file };
        const enabled = userCfg()?.enabledModels ?? [];
        const nextEnabled = enabled.includes(modelId) ? enabled : [...enabled, modelId];
        persist({ localModelPaths: next, enabledModels: nextEnabled, enabled: true });
      }
    } catch {
      /* user cancelled dialog */
    }
  };

  /** Remove a local model file */
  const removeLocalModelFile = (modelId: string) => {
    const current = userCfg()?.localModelPaths ?? {};
    const next = { ...current };
    delete next[modelId];
    persist({
      localModelPaths: Object.keys(next).length > 0 ? next : undefined,
      enabledModels: (userCfg()?.enabledModels ?? []).filter((m) => m !== modelId),
    });
  };

  /** Start the local engine */
  const handleStartEngine = async () => {
    const def = localEngineDef();
    if (!def) return;
    setEngineActionLoading(true);
    try {
      const u = userCfg();
      const paths = u?.localModelPaths ?? {};
      const modelPath = Object.values(paths)[0];
      if (!modelPath) {
        showToast(t('provider.modelRequired'), false);
        return;
      }
      const port = def.engineType === 'vllm' ? 8000 : def.engineType === 'ollama' ? 11434 : 8080;
      await startLocalServer({
        modelPath,
        port,
        gpuLayers: 99,
        engineType: def.engineType,
        trustRemoteCode: false,
      });
      setIsServerAlive('alive');
      showToast(t('provider.localStarted', { name: modelPath, engine: def.name }), true);
    } catch (e) {
      showToast(t('provider.localFailed', { error: String(e) }), false);
    } finally {
      setEngineActionLoading(false);
    }
  };
  /** Stop the local engine */
  const handleStopEngine = async () => {
    const def = localEngineDef();
    if (!def) return;
    setEngineActionLoading(true);
    try {
      await stopLocalServer(def.engineType);
      setIsServerAlive('dead');
      showToast(t('provider.localStopped'), true);
    } catch (e) {
      // FE-9: raw String(e) echoed provider/transport internals; wrap in i18n
      showToast(t('provider.stopFailed', { error: String(e) }), false);
    } finally {
      setEngineActionLoading(false);
    }
  };

  // ===== model grouping (createMemo is required to keep reactivity) =====
  const modelGroups = createMemo(() => {
    const c = cat();
    if (!c || isCustom()) return { enabled: [], available: [], orphans: [] as string[] };
    return listProviderModels(c, providerId(), userCfg()?.enabledModels ?? []);
  });

  const visibleAvailable = createMemo(() => {
    const q = search().trim().toLowerCase();
    let list = modelGroups().available;
    if (q) {
      list = list.filter(
        (m) =>
          m.id.toLowerCase().includes(q) ||
          m.displayName.toLowerCase().includes(q) ||
          (m.family ?? '').toLowerCase().includes(q),
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
    return list.filter(
      (m) => m.id.toLowerCase().includes(q) || m.displayName.toLowerCase().includes(q),
    );
  });

  const visibleOrphans = createMemo(() => {
    const q = search().trim().toLowerCase();
    const list = modelGroups().orphans;
    if (!q) return list;
    return list.filter((id) => id.toLowerCase().includes(q));
  });

  const handleTestConnection = async () => {
    const u = userCfg();
    setTestState({ status: 'testing' });
    try {
      const r = await testProviderConnection({
        apiUrl: u?.apiUrl ?? defaultApiUrl(providerId()),
        apiKey: u?.apiKey ?? '',
        proxyUrl: u?.proxyUrl ?? null,
        engineType: isLocalEngine() ? (localEngineDef()?.engineType ?? null) : null,
      });
      if (r.success) {
        setTestState({
          status: 'ok',
          msg: t('provider.connectionResult', {
            count: formatNumber(r.modelCount),
            elapsed: formatNumber(r.elapsedMs),
          }),
          sampleModels: r.sampleModelIds,
        });
      } else {
        console.error('[provider] connection test failed:', r.error);
        setTestState({ status: 'fail', msg: t('error.connection') });
      }
    } catch (e) {
      setTestState({ status: 'fail', msg: reportError('error.connection', e) });
    }
  };

  // ===== local engine liveness probe =====
  let alivePoll: number | undefined;
  onMount(() => {
    if (!isLocalEngine()) return;
    const probe = async () => {
      // Skip polling while a start/stop operation is in flight to avoid races overwriting the state
      if (engineActionLoading()) return;
      const u = userCfg();
      if (!u?.apiUrl) {
        setIsServerAlive('dead');
        return;
      }
      try {
        const engineType = localEngineDef()?.engineType;
        if (engineType === 'llama_cpp' || engineType === 'vllm') {
          const alive = await probeEngineHealth(u.apiUrl, null);
          setIsServerAlive(alive ? 'alive' : 'dead');
        } else if (engineType === 'ollama') {
          const alive = await probeEngineHealth(u.apiUrl, 'ollama');
          setIsServerAlive(alive ? 'alive' : 'dead');
          if (alive) {
            try {
              const r = await fetchProviderModels({
                apiUrl: u.apiUrl,
                apiKey: '',
                proxyUrl: null,
                engineType: 'ollama',
              });
              if (r.success && r.models.length > 0) {
                const incoming: FetchedModel[] = r.models.map((m) => ({
                  id: m.id,
                  ownedBy: m.owned_by,
                  displayName: m.display_name,
                  releasedAt: m.released_at,
                }));
                // FE-4: the 5s poll used to persist unconditionally — one disk write
                // every tick even when nothing changed. Only write on a real change.
                if (!sameFetchedModels(userCfg()?.fetchedModels, incoming)) {
                  persist({ fetchedModels: incoming });
                }
                setFetchState({
                  status: 'ok',
                  msg: t('provider.fetchResult', { count: formatNumber(r.models.length) }),
                  models: incoming,
                });
              }
            } catch {
              /* silence auto-fetch failures; next tick will retry */
            }
          } else {
            // FE-4: same diff-check on the dead path (clear only when it held data)
            if ((userCfg()?.fetchedModels?.length ?? 0) > 0) {
              persist({ fetchedModels: [] });
            }
          }
        } else {
          const r = await testProviderConnection({
            apiUrl: u.apiUrl,
            apiKey: '',
            proxyUrl: null,
            engineType: engineType ?? null,
          });
          setIsServerAlive(r.success ? 'alive' : 'dead');
        }
      } catch {
        setIsServerAlive('dead');
      }
    };
    probe();
    alivePoll = window.setInterval(probe, 5000);
  });
  onCleanup(() => {
    clearInterval(alivePoll);
    // FE-4: flush any debounced (still unsaved) field edits so leaving the page
    // cannot lose the last keystroke burst
    if (persistTimer !== undefined) {
      clearTimeout(persistTimer);
      persistTimer = undefined;
      const merged = pendingOverrides;
      pendingOverrides = null;
      if (merged) void persist(merged);
    }
  });

  const handleFetchModels = async () => {
    const u = userCfg();
    setFetchState({ status: 'fetching' });
    try {
      const r = await fetchProviderModels({
        apiUrl: u?.apiUrl ?? '',
        apiKey: u?.apiKey ?? '',
        proxyUrl: u?.proxyUrl ?? null,
        engineType: isLocalEngine() ? (localEngineDef()?.engineType ?? null) : null,
      });
      if (r.success) {
        const incoming: FetchedModel[] = r.models.map((m) => ({
          id: m.id,
          ownedBy: m.owned_by,
          displayName: m.display_name,
          releasedAt: m.released_at,
        }));
        setFetchState({
          status: 'ok',
          msg: t('provider.fetchResult', { count: formatNumber(r.models.length) }),
          models: incoming,
        });
        if (isCustom() || localEngineDef()?.engineType === 'ollama') {
          persist({ fetchedModels: incoming });
        }
      } else {
        console.error('[provider] fetch models failed:', r.error);
        setFetchState({ status: 'fail', msg: t('error.connection') });
      }
    } catch (e) {
      setFetchState({ status: 'fail', msg: reportError('error.connection', e) });
    }
  };

  return (
    <div class="h-full overflow-y-auto">
      <div class="max-w-5xl mx-auto p-4 sm:p-6">
        {/* Top: back button + title */}
        <div class="flex items-center gap-3 mb-5 animate-row-in">
          <button type="button" class={btnSecondarySm} onClick={() => navigate('/settings')}>
            <Icon name="arrow-left" size={14} /> {t('provider.back')}
          </button>
          <Show when={!catalogReady()}>
            <span class="text-xs text-[rgba(var(--text-base-rgb),0.55)] flex items-center gap-1.5">
              <span class="w-1.5 h-1.5 rounded-full bg-pri animate-pulse" />
              {t('provider.loadingCatalog')}
            </span>
          </Show>
        </div>

        {/* Provider header card */}
        <div
          class="glass-card mb-4 flex items-center gap-4 animate-row-in"
          style={{ 'animation-delay': '30ms' }}
        >
          <div class="flex items-center justify-center w-12 h-12 rounded-lg bg-white border border-white/85 shadow-[0_1px_3px_rgba(0,0,0,0.15)] overflow-hidden shrink-0 text-[var(--color-dark-700)]">
            {getProviderLogo(providerId()) ? (
              <img
                src={getProviderLogo(providerId())!}
                alt={userCfg()?.displayName ?? providerId()}
                class="w-7 h-7 object-contain"
              />
            ) : (
              <span class="font-bold">
                {(userCfg()?.displayName ?? providerId()).charAt(0).toUpperCase()}
              </span>
            )}
          </div>
          <div class="grow min-w-0">
            <h1 class="text-xl font-bold text-white truncate tracking-tight">
              {userCfg()?.displayName ?? providerMeta()?.name ?? providerId()}
            </h1>
            <div class="text-xs text-[rgba(var(--text-base-rgb),0.55)] font-mono mt-1 flex items-center gap-2 flex-wrap">
              <Show when={isCustom()}>
                <span
                  class="inline-flex items-center px-[7px] py-px rounded-full text-[9px] font-semibold tracking-[0.5px] uppercase leading-[1.6]"
                  style={{
                    background: 'rgba(var(--primary-rgb), 0.15)',
                    color: 'rgba(var(--primary-rgb), 1)',
                    border: '1px solid rgba(var(--primary-rgb), 0.25)',
                  }}
                >
                  {t('provider.custom')}
                </span>
              </Show>
              <Show when={!isCustom()}>
                <span>
                  {t('provider.enabledModels', {
                    count: `${formatNumber(modelGroups().enabled.length)} / ${formatNumber(modelGroups().enabled.length + modelGroups().available.length)}`,
                  })}
                </span>
                <Show when={modelGroups().enabled.length > 0}>
                  <span class="text-[rgba(var(--text-base-rgb),0.4)]">·</span>
                  <span class="inline-flex items-center px-[7px] py-px rounded-full text-[9px] font-semibold tracking-[0.5px] uppercase leading-[1.6] bg-green-400/15 text-green-300 border border-green-400/20">
                    {t('common.enabled')}
                  </span>
                </Show>
              </Show>
              <Show when={isLocalEngine()}>
                <span
                  class="ml-2 inline-flex items-center gap-1 text-[10px]"
                  classList={{
                    'text-green-400': isServerAlive() === 'alive',
                    'text-yellow-400': isServerAlive() === 'checking',
                    'text-red-400': isServerAlive() === 'dead',
                  }}
                >
                  <span
                    class="w-1.5 h-1.5 rounded-full"
                    classList={{
                      'bg-green-400': isServerAlive() === 'alive',
                      'bg-yellow-400': isServerAlive() === 'checking',
                      'bg-red-400': isServerAlive() === 'dead',
                    }}
                  />
                  {isServerAlive() === 'alive'
                    ? t('provider.localRunning')
                    : isServerAlive() === 'checking'
                      ? t('engine.checking')
                      : t('provider.notRunning')}
                </span>
              </Show>
            </div>
          </div>
          <Show when={!isCustom() && providerMeta()}>
            <Show
              when={
                (providerMeta() as any).doc && /^https?:\/\//i.test((providerMeta() as any).doc)
              }
            >
              <a
                href={(providerMeta() as any).doc}
                target="_blank"
                rel="noopener noreferrer"
                class="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-white/10 text-[rgba(var(--text-base-rgb),0.65)] hover:border-pri-30 hover:text-pri transition-all duration-200"
              >
                <Icon name="book" size={12} /> {t('provider.docs')}
              </a>
            </Show>
          </Show>
        </div>

        {/* Local engine install guide */}
        <Show when={isLocalEngine() && !userCfg()}>
          <div
            class="glass-card mb-4 p-4 text-xs text-[rgba(var(--text-base-rgb),0.65)] leading-relaxed animate-row-in"
            style={{ 'animation-delay': '45ms' }}
          >
            <p class="mb-2">{t('provider.localEngineGuide', { name: localEngineDef()!.name })}</p>
            <Show when={localEngineDef()!.engineInstallUrl}>
              <a
                href={localEngineDef()!.engineInstallUrl}
                target="_blank"
                rel="noopener noreferrer"
                class="text-pri hover:underline"
              >
                {t('provider.localEngineDoc')}
              </a>
            </Show>
          </div>
        </Show>
        {/* Form + action buttons combined card */}
        <div class="glass-card mb-4 animate-row-in" style={{ 'animation-delay': '60ms' }}>
          <div class="section-label mb-3">{t('provider.connectionConfig')}</div>
          <div class="grid grid-cols-1 md:grid-cols-2 gap-3 mb-4">
            <div>
              <label class="block section-label mb-1.5" style={{ 'font-size': '9px' }}>
                {t('provider.displayName')}
              </label>
              <input
                type="text"
                class="bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none w-full px-3 py-2 text-sm"
                value={userCfg()?.displayName ?? ''}
                onInput={(e) => updateField('displayName', e.currentTarget.value)}
              />
            </div>
            <div>
              <label class="block section-label mb-1.5" style={{ 'font-size': '9px' }}>
                {t('provider.apiUrlLabel')}
              </label>
              <input
                type="text"
                class="bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none w-full px-3 py-2 text-sm font-mono"
                value={userCfg()?.apiUrl ?? ''}
                placeholder={localEngineDef()?.defaultApiUrl ?? ''}
                onInput={(e) => updateField('apiUrl', e.currentTarget.value)}
              />
            </div>
            <Show when={!isLocalEngine()}>
              <div>
                <label class="block section-label mb-1.5" style={{ 'font-size': '9px' }}>
                  API Key
                </label>
                <input
                  type="password"
                  placeholder="sk-..."
                  class="bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none w-full px-3 py-2 text-sm font-mono"
                  value={userCfg()?.apiKey ?? ''}
                  onInput={(e) => updateField('apiKey', e.currentTarget.value)}
                />
              </div>
            </Show>
            <Show when={!isLocalEngine()}>
              <div>
                <label class="block section-label mb-1.5" style={{ 'font-size': '9px' }}>
                  {t('provider.proxyUrl')}{' '}
                  <span class="text-[rgba(var(--text-base-rgb),0.4)] normal-case tracking-normal font-normal ml-1">
                    ({t('provider.proxyOptional')})
                  </span>
                </label>
                <input
                  type="text"
                  placeholder={t('provider.proxyPlaceholder')}
                  class="bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none w-full px-3 py-2 text-sm font-mono"
                  value={userCfg()?.proxyUrl ?? ''}
                  onInput={(e) => updateField('proxyUrl', e.currentTarget.value || undefined)}
                />
              </div>
            </Show>
          </div>

          {/* Enable toggle + action buttons */}
          <div class="flex items-center gap-3 flex-wrap pt-3 border-t border-white/5">
            {/* 阶段4（设置样板收敛）: glow switch is the shared component; the label stays a
                plain button (switch-in-button is invalid HTML) */}
            <div class="flex items-center gap-2.5">
              <Switch
                variant="glow"
                checked={userCfg()?.enabled ?? false}
                onChange={(v) => toggleProviderEnabled(v)}
              />
              <button
                type="button"
                class="text-sm text-white bg-transparent border-0 p-0 cursor-pointer"
                onClick={() => toggleProviderEnabled(!(userCfg()?.enabled ?? false))}
              >
                {t('provider.enableProvider')}
              </button>
            </div>
            <button
              type="button"
              class={btnSecondarySm}
              disabled={testState().status === 'testing'}
              onClick={handleTestConnection}
            >
              <Show
                when={testState().status === 'testing'}
                fallback={<Icon name="beaker" size={13} />}
              >
                <Icon name="spinner" size={13} class="animate-spin" />
              </Show>
              {testState().status === 'testing'
                ? t('provider.testing')
                : t('provider.testConnection')}
            </button>
            <Show when={!isLocalEngine()}>
              <button
                type="button"
                class={btnSecondarySm}
                disabled={fetchState().status === 'fetching'}
                onClick={handleFetchModels}
              >
                <Show
                  when={fetchState().status === 'fetching'}
                  fallback={<Icon name="download" size={13} />}
                >
                  <Icon name="spinner" size={13} class="animate-spin" />
                </Show>
                {fetchState().status === 'fetching'
                  ? t('provider.fetching')
                  : t('provider.fetchModels')}
              </button>
            </Show>
          </div>
        </div>

        {/* vLLM Windows platform warning */}
        <Show
          when={
            isLocalEngine() &&
            localEngineDef()!.engineType === 'vllm' &&
            /Windows|Win32|Win64/i.test(navigator.userAgent)
          }
        >
          <div
            class="glass-card mb-4 p-4 text-xs text-red-300 leading-relaxed animate-row-in border border-red-400/20"
            style={{ 'animation-delay': '75ms' }}
          >
            <div class="flex items-center gap-2">
              <Icon name="alert-triangle" size={14} />
              <span>{t('provider.vllmWindowsWarning')}</span>
            </div>
          </div>
        </Show>

        {/* Engine status panel */}
        <Show when={isLocalEngine()}>
          <div class="glass-card mb-4 animate-row-in" style={{ 'animation-delay': '80ms' }}>
            {/* Install status */}
            <div class="flex items-center gap-3 mb-3 text-sm">
              <span
                class="w-2 h-2 rounded-full"
                classList={{
                  'bg-green-400': engineInstallInfo()?.installed,
                  'bg-gray-500': !engineInstallInfo()?.installed,
                }}
              />
              <span class="text-white/80">
                {engineInstallInfo()?.installed
                  ? t('engine.installed')
                  : engineInstallInfo() === null
                    ? t('engine.scanning')
                    : t('engine.notInstalled')}
              </span>
              <Show when={engineInstallInfo()?.version}>
                <span class="text-white/40 text-xs font-mono">{engineInstallInfo()!.version}</span>
              </Show>
            </div>

            {/* Running state + action buttons */}
            <div class="flex items-center gap-3 flex-wrap">
              <span
                class="w-2 h-2 rounded-full"
                classList={{
                  'bg-green-400': isServerAlive() === 'alive',
                  'bg-yellow-400': isServerAlive() === 'checking',
                  'bg-red-400': isServerAlive() === 'dead',
                }}
              />
              <span class="text-white/80 text-sm">
                {isServerAlive() === 'alive'
                  ? t('engine.running')
                  : isServerAlive() === 'checking'
                    ? t('engine.checking')
                    : t('engine.stopped')}
              </span>

              <Show when={isServerAlive() !== 'alive'}>
                <button
                  type="button"
                  class={btnSuccessOutline}
                  disabled={engineActionLoading()}
                  onClick={handleStartEngine}
                >
                  <Show
                    when={!engineActionLoading()}
                    fallback={<Icon name="spinner" size={13} class="animate-spin" />}
                  >
                    <Icon name="play" size={13} />
                  </Show>
                  {t('engine.start')}
                </button>
              </Show>
              <Show when={isServerAlive() === 'alive'}>
                <button
                  type="button"
                  class={btnDangerOutline}
                  disabled={engineActionLoading()}
                  onClick={handleStopEngine}
                >
                  <Show
                    when={!engineActionLoading()}
                    fallback={<Icon name="spinner" size={13} class="animate-spin" />}
                  >
                    <Icon name="stop" size={13} />
                  </Show>
                  {t('engine.stop')}
                </button>
              </Show>
            </div>
          </div>
        </Show>

        {/* Test/fetch feedback */}
        <Show when={testState().status !== 'idle'}>
          <div
            class="flex items-center gap-1.5 px-3 py-2 rounded-lg text-xs border backdrop-blur-[20px] mb-4 animate-row-in"
            classList={{
              'bg-green-400/[0.08] border-green-400/25 text-green-300': testState().status === 'ok',
              'bg-red-400/[0.08] border-red-400/25 text-red-300': testState().status === 'fail',
              'bg-pri/5 border-pri text-white': testState().status === 'testing',
            }}
          >
            <Show
              when={testState().status === 'testing'}
              fallback={
                <Show when={testState().status === 'ok'} fallback={<Icon name="x" size={14} />}>
                  <Icon name="check" size={14} />
                </Show>
              }
            >
              <Icon name="spinner" size={14} class="animate-spin" />
            </Show>
            <span class="flex-1">{testState().msg}</span>
            <Show when={testState().sampleModels && testState().sampleModels!.length > 0}>
              <span class="text-[rgba(var(--text-base-rgb),0.55)] font-mono">
                ({testState().sampleModels!.slice(0, 3).join(', ')}...)
              </span>
            </Show>
          </div>
        </Show>
        <Show
          when={
            (!isLocalEngine() ||
              ((localEngineDef()?.supportsFetchModels ?? false) &&
                localEngineDef()!.engineType !== 'ollama')) &&
            fetchState().status !== 'idle'
          }
        >
          <div
            class="flex items-center gap-1.5 px-3 py-2 rounded-lg text-xs border backdrop-blur-[20px] mb-4 animate-row-in"
            classList={{
              ok: fetchState().status === 'ok',
              fail: fetchState().status === 'fail',
              test: fetchState().status === 'fetching',
            }}
          >
            <Show
              when={fetchState().status === 'fetching'}
              fallback={
                <Show when={fetchState().status === 'ok'} fallback={<Icon name="x" size={14} />}>
                  <Icon name="check" size={14} />
                </Show>
              }
            >
              <Icon name="spinner" size={14} class="animate-spin" />
            </Show>
            <span class="flex-1">{fetchState().msg}</span>
          </div>
        </Show>

        {/* Local engine active models info card */}
        <Show
          when={
            isLocalEngine() &&
            testState().status === 'ok' &&
            testState().sampleModels &&
            testState().sampleModels!.length > 0
          }
        >
          <div class="glass-card mt-4 animate-row-in" style={{ 'animation-delay': '105ms' }}>
            <div class="section-label mb-3">{t('provider.activeModel')}</div>
            <div class="space-y-1.5">
              <For each={testState().sampleModels}>
                {(modelId) => (
                  <div class="flex items-center gap-2 px-3 py-2 rounded-lg bg-white/5 border border-white/10">
                    <Icon name="model" size={14} />
                    <span class="text-xs text-white font-mono">{modelId}</span>
                  </div>
                )}
              </For>
            </div>
          </div>
        </Show>

        {/* Local model file picker (only for engines with allowModelFileSelection) */}
        <Show when={isLocalEngine() && localEngineDef()!.allowModelFileSelection}>
          <div class="glass-card mt-4 animate-row-in" style={{ 'animation-delay': '75ms' }}>
            <div class="section-label mb-3">{t('provider.localModels')}</div>
            <div class="text-xs text-[rgba(var(--text-base-rgb),0.55)] mb-3">
              {t('provider.localModelsHint')}
            </div>
            <Show
              when={
                (userCfg()?.localModelPaths &&
                  Object.keys(userCfg()!.localModelPaths!).length > 0) ||
                false
              }
            >
              <div class="space-y-1.5 mb-3">
                <For each={Object.entries(userCfg()?.localModelPaths ?? {})}>
                  {([modelId, path]) => (
                    <div class="flex items-center gap-2 px-3 py-2 rounded-lg bg-white/5 border border-white/10">
                      <Icon name="file" size={14} />
                      <span class="flex-1 text-xs text-white font-mono truncate">{path}</span>
                      <button
                        type="button"
                        class={
                          'w-5 h-5 text-[rgba(var(--text-base-rgb),0.55)] hover:text-red-400 ' +
                          btnIcon
                        }
                        onClick={() => removeLocalModelFile(modelId)}
                      >
                        <Icon name="x" size={12} />
                      </button>
                    </div>
                  )}
                </For>
              </div>
            </Show>
            <button
              type="button"
              class={`${btnSecondarySm} border-dashed`}
              onClick={pickLocalModelFile}
            >
              <Icon name="plus" size={12} />
              {t('provider.addLocalModel')}
            </button>
          </div>
        </Show>
        {/* ===== model list section ===== */}
        <Show when={isCustom() && !isLocalEngine()}>
          <div class="glass-card mt-4 animate-row-in" style={{ 'animation-delay': '90ms' }}>
            <div class="flex items-center justify-between mb-3">
              <div class="section-label">
                {t('provider.customModels', {
                  count: formatNumber(userCfg()?.fetchedModels?.length ?? 0),
                })}
              </div>
            </div>
            <div class="text-xs text-[rgba(var(--text-base-rgb),0.55)] italic mb-3 px-1">
              {t('provider.customModelsHint')}
            </div>
            <Show when={(userCfg()?.fetchedModels?.length ?? 0) > 0}>
              <div class="space-y-1.5">
                <For each={userCfg()?.fetchedModels ?? []}>
                  {(m, i) => (
                    <div class="animate-row-in" style={{ 'animation-delay': `${i() * 30}ms` }}>
                      <ModelRow
                        meta={
                          {
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
                          } as any
                        }
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
          <div class="glass-card mt-4 animate-row-in" style={{ 'animation-delay': '90ms' }}>
            <div class="flex items-center gap-3 mb-3 flex-wrap">
              <div class="section-label">
                {t('provider.modelList', {
                  count: formatNumber(
                    modelGroups().enabled.length + modelGroups().available.length,
                  ),
                })}
              </div>
              <div class="flex items-center gap-2 ml-auto flex-wrap">
                <div class="relative">
                  <Icon
                    name="search"
                    size={12}
                    class="absolute left-2.5 top-1/2 -translate-y-1/2 text-[rgba(var(--text-base-rgb),0.4)] pointer-events-none"
                  />
                  <input
                    type="text"
                    placeholder={t('provider.searchModels')}
                    class="bg-black/25 border border-white/[0.08] rounded-lg text-white transition-[border-color,background,box-shadow] duration-200 placeholder:text-white/30 hover:border-white/[0.14] focus:outline-none pl-7 pr-3 py-1 text-xs"
                    style={{ width: '180px' }}
                    value={search()}
                    onInput={(e) => setSearch(e.currentTarget.value)}
                  />
                </div>
                <Dropdown
                  value={sortKey()}
                  onChange={(v) => setSortKey(v as SortKey)}
                  options={[
                    { value: 'releaseDesc', label: t('provider.releaseDate') },
                    { value: 'nameAsc', label: t('provider.nameSort') },
                  ]}
                  class="text-xs"
                />
              </div>
            </div>

            {/* Enabled */}
            <Show when={visibleEnabled().length > 0}>
              <div class="section-label mt-2 mb-2 flex items-center gap-2">
                <span class="w-1.5 h-1.5 rounded-full bg-green-400" />
                {t('provider.enabledCount', { count: formatNumber(visibleEnabled().length) })}
              </div>
              <div class="space-y-1.5">
                <For each={visibleEnabled()}>
                  {(m, i) => (
                    <div class="animate-row-in" style={{ 'animation-delay': `${i() * 25}ms` }}>
                      <ModelRow meta={m} enabled={true} onToggle={() => toggleModel(m.id)} />
                    </div>
                  )}
                </For>
              </div>
            </Show>

            {/* Not enabled */}
            <Show when={visibleAvailable().length > 0}>
              <div class="section-label mt-4 mb-2 flex items-center gap-2">
                <span class="w-1.5 h-1.5 rounded-full bg-[rgba(var(--text-base-rgb),0.4)]" />
                {t('provider.notEnabledCount', { count: formatNumber(visibleAvailable().length) })}
              </div>
              <div class="space-y-1.5">
                <For each={visibleAvailable()}>
                  {(m, i) => (
                    <div class="animate-row-in" style={{ 'animation-delay': `${i() * 25}ms` }}>
                      <ModelRow meta={m} enabled={false} onToggle={() => toggleModel(m.id)} />
                    </div>
                  )}
                </For>
              </div>
            </Show>

            {/* Orphans (not in the catalog but previously enabled by the user) */}
            <Show when={visibleOrphans().length > 0}>
              <div class="section-label mt-4 mb-2 flex items-center gap-2">
                <span class="w-1.5 h-1.5 rounded-full bg-yellow-400/60" />
                {t('provider.orphanCount', { count: formatNumber(visibleOrphans().length) })}
              </div>
              <div class="flex flex-wrap gap-1.5">
                <For each={visibleOrphans()}>
                  {(mid, i) => (
                    <span
                      class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md inline-flex items-center px-[7px] py-px rounded-full text-[9px] font-semibold tracking-[0.5px] uppercase leading-[1.6] bg-yellow-400/[0.12] text-yellow-200 border border-yellow-400/20 font-mono animate-row-in"
                      style={{ 'animation-delay': `${i() * 30}ms` }}
                    >
                      <span class="truncate max-w-[200px]">{mid}</span>
                      <button
                        type="button"
                        class="w-4 h-4 flex items-center justify-center rounded-full text-[12px] leading-none transition-colors hover:bg-white/15"
                        title={t('common.delete')}
                        onClick={() => removeOrphan(mid)}
                      >
                        <Icon name="x" size={10} />
                      </button>
                    </span>
                  )}
                </For>
              </div>
            </Show>

            {/* Empty state */}
            <Show
              when={
                modelGroups().enabled.length === 0 &&
                modelGroups().available.length === 0 &&
                !search()
              }
            >
              <div class="text-xs text-[rgba(var(--text-base-rgb),0.4)] italic py-6 text-center">
                {t('provider.noCatalogModels')}
              </div>
            </Show>
            <Show
              when={
                search() &&
                visibleEnabled().length === 0 &&
                visibleAvailable().length === 0 &&
                visibleOrphans().length === 0
              }
            >
              <div class="text-xs text-[rgba(var(--text-base-rgb),0.4)] italic py-6 text-center">
                {t('provider.noModelMatch', { query: search() })}
              </div>
            </Show>
          </div>
        </Show>

        {/* 阶段4（设置样板收敛）: shared toast bubble */}
        <ToastBubble toast={toast()} />
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
