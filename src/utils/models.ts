/**
 * 模型目录加载与查询工具
 * @description 包装 @aio/models-data 包，提供 catalog 加载、模型查询、能力补全
 * @data-source D:/base/aio-models-data (每周日 04:00 UTC 自动同步)
 */

import { invoke } from '@tauri-apps/api/core'
import type { ModelMeta, ProviderMeta, Catalog } from '@aio/models-data'

export type { ModelMeta, ProviderMeta, Catalog } from '@aio/models-data'

let cachedCatalog: Catalog | null = null
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

async function loadCatalogFromBackend(): Promise<Catalog> {
  try {
    const json = await invoke<string>('load_models_catalog')
    return JSON.parse(json) as Catalog
  } catch (e) {
    console.warn('[models] 从后端加载 catalog 失败:', e)
    return EMPTY_CATALOG
  }
}

export async function loadModelsCatalog(): Promise<Catalog> {
  if (cachedCatalog) return cachedCatalog
  if (loadPromise) return loadPromise
  loadPromise = loadCatalogFromBackend().then(c => {
    cachedCatalog = c
    loadPromise = null
    return c
  })
  return loadPromise
}

export function getCachedCatalog(): Catalog | null {
  return cachedCatalog
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
