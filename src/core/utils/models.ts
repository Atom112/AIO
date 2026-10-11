/**
 * Model catalog loading and query utilities
 * @description Wraps the @aio/models-data package: catalog loading, model lookup, capability completion
 * @data-source D:/base/aio-models-data (auto-synced every Sunday 04:00 UTC)
 */

import type { ModelMeta, ProviderMeta, Catalog } from '@aio/models-data';
// FE-25: the Tauri invoke wrappers moved to core/api/catalog.ts (utils keeps
// cache + pure lookup logic only)
import { loadCatalogFull, updateCatalog } from '../api/catalog';
import type { CatalogResponse, CatalogSourceTag, UpdateResult } from '../api/catalog';
import {
  formatDateTime as formatLocaleDateTime,
  formatRelativeTime as formatLocaleRelativeTime,
  t,
} from '../i18n';

export type { ModelMeta, ProviderMeta, Catalog } from '@aio/models-data';

/** Model entries fetched from a provider API and persisted (LobeHub-shaped list rows) */
export interface FetchedModel {
  id: string;
  ownedBy: string;
  /** Vendor-provided display name (Google `displayName` / Anthropic `display_name`); absent for OpenAI/Ollama */
  displayName?: string;
  /** Release date YYYY-MM-DD; OpenAI `created` converted, Anthropic `created_at` passed through */
  releasedAt?: string;
}

export interface ProviderConfig {
  id: string;
  enabled: boolean;
  displayName: string;
  apiUrl: string;
  apiKey: string;
  enabledModels: string[];
  isCustom: boolean;
  customModelIds: string[];
  /** per-provider HTTP/HTTPS proxy (e.g. `http://127.0.0.1:7890`), for regions where OpenAI/Google are hard to reach */
  proxyUrl?: string;
  /** Model list fetched and persisted from the API, for the LobeHub-style two-section toggle list */
  fetchedModels?: FetchedModel[];
  /** Local-engine only: model file path mapping { modelId -> localPath } */
  localModelPaths?: Record<string, string>;
}

export interface ProviderConfigFile {
  version: number;
  updatedAt: string;
  providers: Record<string, ProviderConfig>;
}

// ==================== Local inference engine provider definitions ====================

/** Built-in local inference engine provider definition */
export interface LocalEngineProvider {
  id: string; // provider identifier, e.g. 'local-llamacpp'
  name: string; // display name, e.g. 'llama.cpp'
  defaultApiUrl: string; // pre-filled URL, e.g. 'http://127.0.0.1:8080/v1'
  engineType: string; // 'llama_cpp' | 'vllm' | 'ollama'
  fileExtensions: string[]; // ['gguf'] for file picker, [] for no file picker
  allowModelFileSelection: boolean;
  engineInstallUrl?: string; // docs URL for manual install (not AIO auto-install)
  /** Whether the model list can be fetched from the API (Ollama's /api/tags; llama.cpp/vLLM fix the model at startup, no fetch needed) */
  supportsFetchModels?: boolean;
}

export const LOCAL_ENGINE_PROVIDERS: LocalEngineProvider[] = [
  {
    id: 'local-llamacpp',
    name: 'llama.cpp',
    defaultApiUrl: 'http://127.0.0.1:8080/v1',
    engineType: 'llama_cpp',
    fileExtensions: ['gguf'],
    allowModelFileSelection: true,
    engineInstallUrl: 'https://github.com/ggml-org/llama.cpp',
    supportsFetchModels: false,
  },
  {
    id: 'local-vllm',
    name: 'vLLM',
    defaultApiUrl: 'http://127.0.0.1:8000/v1',
    engineType: 'vllm',
    fileExtensions: [],
    allowModelFileSelection: false,
    engineInstallUrl: 'https://docs.vllm.ai',
    supportsFetchModels: false,
  },
  {
    id: 'ollama',
    name: 'Ollama',
    defaultApiUrl: 'http://localhost:11434',
    engineType: 'ollama',
    fileExtensions: ['gguf'],
    allowModelFileSelection: true,
    engineInstallUrl: 'https://ollama.ai',
    supportsFetchModels: true,
  },
];

/** Whether a provider id is a local engine provider */
export const isLocalEngineProvider = (id: string): boolean =>
  LOCAL_ENGINE_PROVIDERS.some((p) => p.id === id);

/** Get the LocalEngineProvider definition for a provider id */
export const getLocalEngineProvider = (id: string): LocalEngineProvider | undefined =>
  LOCAL_ENGINE_PROVIDERS.find((p) => p.id === id);

export interface TestConnectionResult {
  success: boolean;
  modelCount: number;
  sampleModelIds: string[];
  error: string | null;
  elapsedMs: number;
}

export interface FetchLiveModelsResult {
  success: boolean;
  models: Array<{ id: string; owned_by: string; display_name?: string; released_at?: string }>;
  error: string | null;
  elapsedMs: number;
}

export type { CatalogResponse, CatalogSourceTag, UpdateResult } from '../api/catalog';
let cachedCatalog: Catalog | null = null;
let cachedSource: CatalogSourceTag = 'empty';
let cachedPath: string | null = null;
let cachedVersion: string | null = null;
let cachedGeneratedAt: string | null = null;
let loadPromise: Promise<Catalog> | null = null;

const EMPTY_CATALOG: Catalog = {
  version: '0.0.0',
  generatedAt: '1970-01-01T00:00:00.000Z',
  source: '',
  providerCount: 0,
  modelCount: 0,
  providers: [],
  models: [],
};

function applyResponse(resp: CatalogResponse): Catalog {
  const cat = JSON.parse(resp.json) as Catalog;
  cachedCatalog = cat;
  cachedSource = resp.source;
  cachedPath = resp.path;
  cachedVersion = resp.version;
  cachedGeneratedAt = resp.generatedAt;
  return cat;
}

async function loadCatalogFromBackend(): Promise<Catalog> {
  try {
    const resp = await loadCatalogFull();
    return applyResponse(resp);
  } catch (e) {
    console.warn('[models] 从后端加载 catalog 失败:', e);
    return EMPTY_CATALOG;
  }
}

export async function loadModelsCatalog(): Promise<Catalog> {
  if (cachedCatalog) return cachedCatalog;
  if (loadPromise) return loadPromise;
  loadPromise = loadCatalogFromBackend().then((c) => {
    loadPromise = null;
    return c;
  });
  return loadPromise;
}

export function getCachedCatalog(): Catalog | null {
  return cachedCatalog;
}

export function getCatalogMeta(): {
  source: CatalogSourceTag;
  path: string | null;
  version: string | null;
  generatedAt: string | null;
} {
  return {
    source: cachedSource,
    path: cachedPath,
    version: cachedVersion,
    generatedAt: cachedGeneratedAt,
  };
}

export async function refreshModelsCatalog(): Promise<Catalog> {
  loadPromise = null;
  return loadModelsCatalog();
}

export async function updateModelsCatalog(url?: string): Promise<UpdateResult> {
  const result = await updateCatalog(url ?? null);
  if (result.success) {
    await refreshModelsCatalog();
  }
  return result;
}

export function findModel(catalog: Catalog, provider: string, id: string): ModelMeta | null {
  const prov = (provider || '').toLowerCase();
  const target = (id || '').toLowerCase();

  const direct = catalog.models.find(
    (m: ModelMeta) => m.provider.toLowerCase() === prov && m.id.toLowerCase() === target,
  );
  if (direct) return direct;

  return (
    catalog.models.find(
      (m: ModelMeta) =>
        m.provider.toLowerCase() === prov &&
        m.aliases.some((a: string) => a.toLowerCase() === target),
    ) ?? null
  );
}

/** Format a context window size (1234567 -> "1.2M") */
export function formatContextWindow(tokens: number): string {
  if (tokens >= 1_000_000)
    return `${(tokens / 1_000_000).toFixed(tokens % 1_000_000 === 0 ? 0 : 1)}M`;
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(0)}K`;
  return String(tokens);
}

/** Format an ISO timestamp as a relative description like "X days ago" */
export function formatRelativeTime(iso: string | null): string {
  if (!iso) return t('common.neverUpdated');
  try {
    const d = new Date(iso);
    const now = Date.now();
    const diff = now - d.getTime();
    const minutes = Math.floor(diff / 60_000);
    if (minutes < 1) return formatLocaleRelativeTime(0, 'minute');
    if (minutes < 60) return formatLocaleRelativeTime(-minutes, 'minute');
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return formatLocaleRelativeTime(-hours, 'hour');
    const days = Math.floor(hours / 24);
    if (days < 30) return formatLocaleRelativeTime(-days, 'day');
    const months = Math.floor(days / 30);
    if (months < 12) return formatLocaleRelativeTime(-months, 'month');
    return formatLocaleRelativeTime(-Math.floor(months / 12), 'year');
  } catch {
    return iso;
  }
}

/** Format a releaseDate ("2025-01-15" -> "2025-01"), year + month only */
export function formatReleaseDate(iso: string | null | undefined): string {
  if (!iso) return '';
  return formatLocaleDateTime(`${iso.slice(0, 10)}T00:00:00`, { year: 'numeric', month: 'short' });
}

// ==================== v2 utility functions ====================

/**
 * Merge catalog model metadata + user-enabled state, split into three sections
 * - enabled: enabled models (in the original enabledIds order)
 * - available: in catalog but not enabled (releaseDate desc, then id alphabetical)
 * - orphans: in enabledModels but not in the catalog (legacy custom / old config)
 */
export function listProviderModels(
  catalog: Catalog,
  providerId: string,
  enabledIds: readonly string[],
): {
  enabled: ModelMeta[];
  available: ModelMeta[];
  orphans: string[];
} {
  const catalogModels = catalog.models.filter((m: ModelMeta) => m.provider === providerId);
  const enabledSet = new Set(enabledIds);
  const enabledOrder = new Map(enabledIds.map((id, i) => [id, i]));

  const enabled: ModelMeta[] = [];
  const available: ModelMeta[] = [];
  const seen = new Set<string>();

  for (const m of catalogModels) {
    if (enabledSet.has(m.id)) {
      enabled.push(m);
      seen.add(m.id);
    }
  }
  // Sort enabled by enabledIds order
  enabled.sort((a, b) => (enabledOrder.get(a.id) ?? 0) - (enabledOrder.get(b.id) ?? 0));

  for (const m of catalogModels) {
    if (!seen.has(m.id)) {
      available.push(m);
    }
  }
  // releaseDate desc, then id alphabetical for the same date
  available.sort((a, b) => {
    const ra = a.releaseDate ?? '';
    const rb = b.releaseDate ?? '';
    if (ra !== rb) return rb.localeCompare(ra); // desc
    return a.id.localeCompare(b.id);
  });

  // orphans: in enabledIds but not in the catalog
  const catalogIds = new Set(catalogModels.map((m: ModelMeta) => m.id));
  const orphans = enabledIds.filter((id) => !catalogIds.has(id));

  return { enabled, available, orphans };
}

/**
 * Reverse-lookup a provider id by host (single entry point)
 * Prefers the api field of ProviderMeta in the catalog, falls back to 9 common host mappings
 */
const KNOWN_HOSTS: ReadonlyArray<readonly [string, string]> = [
  ['api.openai.com', 'openai'],
  ['api.anthropic.com', 'anthropic'],
  ['generativelanguage.googleapis.com', 'google'],
  ['api.deepseek.com', 'deepseek'],
  ['api.groq.com', 'groq'],
  ['api.mistral.ai', 'mistral'],
  ['api.x.ai', 'xai'],
  ['api.cohere.ai', 'cohere'],
  ['openrouter.ai', 'openrouter'],
];

export function detectProviderByUrl(url: string): string | null {
  if (!url) return null;
  const u = url.toLowerCase();
  for (const [host, pid] of KNOWN_HOSTS) {
    if (u.includes(host)) return pid;
  }
  return null;
}

/**
 * Whether to use the dedicated /images/generations endpoint (otherwise images are inline in chat/completions).
 *
 * - `dall-e-2`/`dall-e-3`: no chat mode, dedicated endpoint only.
 * - `gpt-image-*`: chat mode preferred (streamed array chunks).
 * - Others: check whether catalog `modalities.output` includes `image`.
 */
export function isImageGenModel(modelId: string, apiUrl?: string): boolean {
  const lower = (modelId || '').toLowerCase();
  if (/^dall-e-[23]$/.test(lower)) return true; // no chat mode, dedicated endpoint only
  if (/^gpt-image-/.test(lower)) return false; // chat mode (streamed array chunks) is better
  const provider = apiUrl ? detectProviderByUrl(apiUrl) : null;
  const catalog = getCachedCatalog();
  const meta = provider && catalog ? findModel(catalog, provider, modelId) : null;
  return !!meta?.modalities?.output?.includes('image');
}

/** List-page search: match provider id / name only, not model names */
export function searchProviders(catalog: Catalog, query: string): ProviderMeta[] {
  if (!query.trim()) return catalog.providers;
  const q = query.toLowerCase();
  return catalog.providers.filter(
    (p: ProviderMeta) => p.id.includes(q) || p.name.toLowerCase().includes(q),
  );
}
