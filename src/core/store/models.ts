// Model domain: selected model / models catalog / provider configs / model key and available-model resolution — split out of store.ts.
// Leaf domain: depends only on types and core/utils/models, not on other store domains.

import { createSignal } from 'solid-js';
import type { ActivatedModel } from './types';
import type { Catalog, CatalogSourceTag, ProviderConfig } from '../utils/models';
import { isLocalEngineProvider, getLocalEngineProvider } from '../utils/models';
export const [selectedModel, setSelectedModel] = createSignal<ActivatedModel | null>(null);
/** Currently selected assistant ID signal, used for sidebar assistant switching */
export const [currentAssistantId, setCurrentAssistantId] = createSignal<string | null>(null);
/** Currently selected topic ID signal, used by the Chat page to track the active conversation */
export const [currentTopicId, setCurrentTopicId] = createSignal<string | null>(null);

/** Models catalog signal (from @aio/models-data synced data, loaded by Layout at startup) */
export const [modelsCatalog, setModelsCatalog] = createSignal<Catalog | null>(null);
/** Models catalog load status: 'idle' | 'loading' | 'ready' | 'failed' */
export const [modelsCatalogStatus, setModelsCatalogStatus] = createSignal<
  'idle' | 'loading' | 'ready' | 'failed'
>('idle');
/** Current catalog source tag */
export const [modelsCatalogSource, setModelsCatalogSource] =
  createSignal<CatalogSourceTag>('empty');
/** Current catalog path on disk (only set for AppData/Bundled sources) */
export const [modelsCatalogPath, setModelsCatalogPath] = createSignal<string | null>(null);
/** Current catalog version (from models.dev) */
export const [modelsCatalogVersion, setModelsCatalogVersion] = createSignal<string | null>(null);
/** Current catalog generation time (ISO string) */
export const [modelsCatalogGeneratedAt, setModelsCatalogGeneratedAt] = createSignal<string | null>(
  null,
);

/** Provider configs (LobeHub-shaped), key = provider id */
export const [providerConfigs, setProviderConfigs] = createSignal<Record<string, ProviderConfig>>(
  {},
);

/** Available models derived from providerConfigs (enabled models of enabled providers) */
export interface ActiveModelEntry {
  provider: string;
  providerName: string;
  modelId: string;
  apiUrl: string;
  apiKey: string;
  isCustom: boolean;
  /** Local engine model file path */
  local_path?: string;
  /** Local engine type */
  engine_type?: string;
}

export const activeProviderModels = (): ActiveModelEntry[] => {
  const cfgs = providerConfigs();
  const out: ActiveModelEntry[] = [];
  for (const cfg of Object.values(cfgs)) {
    if (!cfg.enabled) continue;
    for (const mid of cfg.enabledModels) {
      const entry: ActiveModelEntry = {
        provider: cfg.id,
        providerName: cfg.displayName,
        modelId: mid,
        apiUrl: cfg.apiUrl,
        apiKey: cfg.apiKey,
        isCustom: cfg.isCustom,
      };
      // Local engine: attach engine_type (always set) + local_path (when a path exists)
      if (isLocalEngineProvider(cfg.id)) {
        entry.engine_type = getLocalEngineProvider(cfg.id)?.engineType;
        const localPath = cfg.localModelPaths?.[mid];
        if (localPath) {
          entry.local_path = localPath;
        }
      }
      out.push(entry);
    }
  }
  return out;
};

/** Whether the model is local (has local_path or engine_type) */
export const isLocalModel = (m: ActivatedModel): boolean => !!(m.local_path || m.engine_type);

/**
 * Build the unique key of a model for assistant bindings.
 * Cloud models use the `model_id@api_url` composite key to distinguish same-named models from
 * different providers (e.g. OpenAI and OpenRouter both offer gpt-4o; model_id alone cannot tell them apart and breaks highlighting).
 * Local models have no stable api_url, so they fall back to the plain model_id.
 */
export const modelKey = (m: ActivatedModel): string => {
  if (isLocalModel(m) || !m.api_url) return m.model_id;
  return `${m.model_id}@${m.api_url}`;
};
