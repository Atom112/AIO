// Models catalog Tauri command wrappers (mirrors commands/catalog.rs).
// FE-25: the invoke calls previously lived inline in core/utils/models.ts —
// utils keeps the cache/lookup logic only, and this api module owns the IPC.
import { invoke } from '@tauri-apps/api/core';

export type CatalogSourceTag = 'appdata' | 'bundled' | 'dev_fallback' | 'empty';

export interface CatalogResponse {
  source: CatalogSourceTag;
  json: string;
  path: string | null;
  version: string | null;
  generatedAt: string | null;
}

export interface UpdateResult {
  success: boolean;
  modelCount: number;
  providerCount: number;
  version: string;
  cachedPath: string;
  error: string | null;
  bytes: number;
  elapsedMs: number;
}

/** Loads the catalog JSON + metadata from the backend (bundled resource or AppData copy). */
export const loadCatalogFull = () => invoke<CatalogResponse>('load_models_catalog_full');

/** Triggers a catalog update from the remote registry; persists to AppData on success. */
export const updateCatalog = (url: string | null) =>
  invoke<UpdateResult>('update_models_catalog', { url });
