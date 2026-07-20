/**
 * 模型目录加载与查询工具
 * @description 包装 @aio/models-data 包，提供 catalog 加载、模型查询、能力补全
 * @data-source D:/base/aio-models-data (每周日 04:00 UTC 自动同步)
 */

import { invoke } from '@tauri-apps/api/core'
import type { ModelMeta, ProviderMeta, Catalog } from '@aio/models-data'

export type { ModelMeta, ProviderMeta, Catalog } from '@aio/models-data'

/** 从 provider API 拉取并持久化的模型条目（仿 LobeHub 列表行） */
export interface FetchedModel {
  id: string
  ownedBy: string
  /** 厂商返回的展示名（Google `displayName` / Anthropic `display_name`），OpenAI/Ollama 无 */
  displayName?: string
  /** 发布日期 YYYY-MM-DD，OpenAI `created` 转 / Anthropic `created_at` 原样 */
  releasedAt?: string
}

export interface ProviderConfig {
  id: string
  enabled: boolean
  displayName: string
  apiUrl: string
  apiKey: string
  enabledModels: string[]
  isCustom: boolean
  customModelIds: string[]
  /** per-provider HTTP/HTTPS 代理 (例如 `http://127.0.0.1:7890`)，用于解决国内访问 OpenAI/Google 的网络问题 */
  proxyUrl?: string
  /** 从 API 持久化拉取的模型列表，用于仿 LobeHub 风格的双段 toggle 列表 */
  fetchedModels?: FetchedModel[]
}

export interface ProviderConfigFile {
  version: number
  updatedAt: string
  providers: Record<string, ProviderConfig>
}

export interface TestConnectionResult {
  success: boolean
  modelCount: number
  sampleModelIds: string[]
  error: string | null
  elapsedMs: number
}

export interface FetchLiveModelsResult {
  success: boolean
  models: Array<{ id: string; owned_by: string; display_name?: string; released_at?: string }>
  error: string | null
  elapsedMs: number
}

export type CatalogSourceTag = 'appdata' | 'bundled' | 'dev_fallback' | 'empty'

export interface CatalogResponse {
  source: CatalogSourceTag
  json: string
  path: string | null
  version: string | null
  generatedAt: string | null
}

export interface UpdateResult {
  success: boolean
  modelCount: number
  providerCount: number
  version: string
  cachedPath: string
  error: string | null
  bytes: number
  elapsedMs: number
}

let cachedCatalog: Catalog | null = null
let cachedSource: CatalogSourceTag = 'empty'
let cachedPath: string | null = null
let cachedVersion: string | null = null
let cachedGeneratedAt: string | null = null
let loadPromise: Promise<Catalog> | null = null

const EMPTY_CATALOG: Catalog = {
  version: '0.0.0',
  generatedAt: '1970-01-01T00:00:00.000Z',
  source: '',
  providerCount: 0,
  modelCount: 0,
  providers: [],
  models: [],
}

function applyResponse(resp: CatalogResponse): Catalog {
  const cat = JSON.parse(resp.json) as Catalog
  cachedCatalog = cat
  cachedSource = resp.source
  cachedPath = resp.path
  cachedVersion = resp.version
  cachedGeneratedAt = resp.generatedAt
  return cat
}

async function loadCatalogFromBackend(): Promise<Catalog> {
  try {
    const resp = await invoke<CatalogResponse>('load_models_catalog_full')
    return applyResponse(resp)
  } catch (e) {
    console.warn('[models] 从后端加载 catalog 失败:', e)
    return EMPTY_CATALOG
  }
}

export async function loadModelsCatalog(): Promise<Catalog> {
  if (cachedCatalog) return cachedCatalog
  if (loadPromise) return loadPromise
  loadPromise = loadCatalogFromBackend().then(c => {
    loadPromise = null
    return c
  })
  return loadPromise
}

export function getCachedCatalog(): Catalog | null {
  return cachedCatalog
}

export function getCatalogMeta(): {
  source: CatalogSourceTag
  path: string | null
  version: string | null
  generatedAt: string | null
} {
  return {
    source: cachedSource,
    path: cachedPath,
    version: cachedVersion,
    generatedAt: cachedGeneratedAt,
  }
}

export async function refreshModelsCatalog(): Promise<Catalog> {
  loadPromise = null
  return loadModelsCatalog()
}

export async function updateModelsCatalog(
  url?: string
): Promise<UpdateResult> {
  const result = await invoke<UpdateResult>('update_models_catalog', { url: url ?? null })
  if (result.success) {
    await refreshModelsCatalog()
  }
  return result
}

export async function getCatalogUrl(): Promise<string> {
  return await invoke<string>('get_catalog_url')
}

export function findModel(
  catalog: Catalog,
  provider: string,
  id: string
): ModelMeta | null {
  const prov = (provider || '').toLowerCase()
  const target = (id || '').toLowerCase()

  const direct = catalog.models.find(
    m => m.provider.toLowerCase() === prov && m.id.toLowerCase() === target
  )
  if (direct) return direct

  return (
    catalog.models.find(
      m =>
        m.provider.toLowerCase() === prov &&
        m.aliases.some(a => a.toLowerCase() === target)
    ) ?? null
  )
}

export function listProvidersForApiUrl(
  catalog: Catalog,
  apiUrl: string
): ProviderMeta[] {
  if (!apiUrl) return catalog.providers.slice(0, 50)
  const u = apiUrl.toLowerCase()
  const knownHosts: Record<string, string> = {
    'api.openai.com': 'openai',
    'api.anthropic.com': 'anthropic',
    'generativelanguage.googleapis.com': 'google',
    'api.deepseek.com': 'deepseek',
    'api.groq.com': 'groq',
    'api.mistral.ai': 'mistral',
    'api.x.ai': 'xai',
    'openrouter.ai': 'openrouter',
    'api.cohere.ai': 'cohere',
  }
  for (const [host, prov] of Object.entries(knownHosts)) {
    if (u.includes(host)) {
      return catalog.providers.filter(p => p.id === prov)
    }
  }
  return catalog.providers.filter(p => !p.isAggregator).slice(0, 50)
}

export function listCatalogModels(
  catalog: Catalog,
  providerId?: string
): ModelMeta[] {
  if (!providerId || providerId === 'all') return catalog.models
  return catalog.models.filter(m => m.provider === providerId)
}

/** 格式化上下文窗口（1234567 → "1.2M"） */
export function formatContextWindow(tokens: number): string {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(tokens % 1_000_000 === 0 ? 0 : 1)}M`
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(0)}K`
  return String(tokens)
}

/** 格式化单条价格（per 1M tokens） */
export function formatPricing(input: number, output: number): string {
  if (input === 0 && output === 0) return '免费'
  return `$${input}/${output}/1M`
}

/** 把 ISO 时间格式化为「X 天前」之类的相对描述 */
export function formatRelativeTime(iso: string | null): string {
  if (!iso) return '从未更新'
  try {
    const d = new Date(iso)
    const now = Date.now()
    const diff = now - d.getTime()
    const minutes = Math.floor(diff / 60_000)
    if (minutes < 1) return '刚刚'
    if (minutes < 60) return `${minutes} 分钟前`
    const hours = Math.floor(minutes / 60)
    if (hours < 24) return `${hours} 小时前`
    const days = Math.floor(hours / 24)
    if (days < 30) return `${days} 天前`
    const months = Math.floor(days / 30)
    if (months < 12) return `${months} 个月前`
    return `${Math.floor(months / 12)} 年前`
  } catch {
    return iso
  }
}

/** 格式化 releaseDate（"2025-01-15" → "2025-01"）只取年月 */
export function formatReleaseDate(iso: string | null | undefined): string {
  if (!iso) return ''
  return iso.slice(0, 7) // YYYY-MM
}

// ==================== v2 工具函数 ====================

/**
 * 合并 catalog 模型元数据 + 用户启用状态, 拆分为三段
 * - enabled: 已启用的 (按 enabledIds 原顺序)
 * - available: catalog 中有但未启用的 (按 releaseDate desc, 同日期按 id 字母序)
 * - orphans: enabledModels 中有但 catalog 没有的 (遗留 custom / 老 config)
 */
export function listProviderModels(
  catalog: Catalog,
  providerId: string,
  enabledIds: readonly string[]
): {
  enabled: ModelMeta[]
  available: ModelMeta[]
  orphans: string[]
} {
  const catalogModels = catalog.models.filter(m => m.provider === providerId)
  const enabledSet = new Set(enabledIds)
  const enabledOrder = new Map(enabledIds.map((id, i) => [id, i]))

  const enabled: ModelMeta[] = []
  const available: ModelMeta[] = []
  const seen = new Set<string>()

  for (const m of catalogModels) {
    if (enabledSet.has(m.id)) {
      enabled.push(m)
      seen.add(m.id)
    }
  }
  // 按 enabledIds 顺序排序 enabled
  enabled.sort((a, b) => (enabledOrder.get(a.id) ?? 0) - (enabledOrder.get(b.id) ?? 0))

  for (const m of catalogModels) {
    if (!seen.has(m.id)) {
      available.push(m)
    }
  }
  // 按 releaseDate desc, 同日期按 id 字母序
  available.sort((a, b) => {
    const ra = a.releaseDate ?? ''
    const rb = b.releaseDate ?? ''
    if (ra !== rb) return rb.localeCompare(ra) // desc
    return a.id.localeCompare(b.id)
  })

  // orphans: enabledIds 中有但 catalog 没有的
  const catalogIds = new Set(catalogModels.map(m => m.id))
  const orphans = enabledIds.filter(id => !catalogIds.has(id))

  return { enabled, available, orphans }
}

/**
 * 通过 host 反查 provider id (统一入口, 替代三处重复实现)
 * 优先匹配 catalog 中 ProviderMeta 的 api 字段, 兜底用 9 个常见 host 映射
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
]

export function detectProviderByUrl(url: string): string | null {
  if (!url) return null
  const u = url.toLowerCase()
  for (const [host, pid] of KNOWN_HOSTS) {
    if (u.includes(host)) return pid
  }
  return null
}

/** 列表页搜索: 仅匹配 provider id / name, 不含模型名 */
export function searchProviders(catalog: Catalog, query: string): ProviderMeta[] {
  if (!query.trim()) return catalog.providers
  const q = query.toLowerCase()
  return catalog.providers.filter(p => p.id.includes(q) || p.name.toLowerCase().includes(q))
}
