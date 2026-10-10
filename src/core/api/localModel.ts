// Local GGUF model market & downloader API (mirrors src-tauri/src/commands/local_model_market.rs
// and local_model_download.rs; components must not invoke these commands directly).

import { invoke } from '@tauri-apps/api/core';

/** One merged market entry (GGUF repo), deduplicated across HuggingFace / ModelScope */
export interface MarketModel {
  /** Canonical id (prefers the HF repo id, e.g. "unsloth/Qwen3-14B-GGUF") */
  id: string;
  name: string;
  org: string;
  hfRepo: string | null;
  msRepo: string | null;
  /** Total parameter count in billions parsed from the name (null = unknown) */
  paramsB: number | null;
  license: string | null;
  downloads: number;
  likes: number;
  stars: number;
  chineseName: string | null;
  /** "huggingface" and/or "modelscope" */
  sources: string[];
}

export interface MarketListResponse {
  models: MarketModel[];
  failedSources: string[];
}

/** One downloadable GGUF file inside a repo (repo-relative path + byte size) */
export interface MarketFile {
  name: string;
  size: number;
}

export interface HardwareProfile {
  cpuBrand: string;
  cpuCores: number;
  totalRamBytes: number;
  gpuName: string | null;
  gpuVramBytes: number | null;
  /** Recommended total parameter count (billions) fitting RAM/VRAM at Q4_K_M */
  recommendedParamsB: number;
}

export interface LocalModelDownloadRequest {
  id: string;
  hfRepo: string | null;
  msRepo: string | null;
  files: Array<{ name: string; size: number }>;
  proxyUrl?: string;
}

export interface LocalModelDownloadOutcome {
  status: 'ok' | 'paused' | 'cancelled';
  id: string;
  entryPath: string | null;
}

export interface InstalledLocalModel {
  id: string;
  files: MarketFile[];
  entryPath: string;
  sizeBytes: number;
}

/** Searches the local model market (empty query = the sites' default GGUF rankings) */
export const localModelMarketList = (query: string, limit?: number) =>
  invoke<MarketListResponse>('local_model_market_list', { query, limit: limit ?? null });

/** Lists the GGUF files of one repo (source: "huggingface" | "modelscope") */
export const localModelMarketFiles = (source: string, repo: string) =>
  invoke<MarketFile[]>('local_model_market_files', { source, repo });

/** Host hardware profile + recommended parameter size */
export const localModelHardwareProfile = () =>
  invoke<HardwareProfile>('local_model_hardware_profile');

/** Reads the optional HuggingFace access token (null when never configured) */
export const localModelHfTokenGet = () => invoke<string | null>('local_model_hf_token_get');

/** Saves (non-empty) or clears (empty) the optional HuggingFace token (gated repos) */
export const localModelHfTokenSet = (token: string) =>
  invoke<void>('local_model_hf_token_set', { token });

/** Downloads one quant-family (pause/resume via HTTP Range; progress events: local-model-download-progress) */
export const localModelDownload = (request: LocalModelDownloadRequest) =>
  invoke<LocalModelDownloadOutcome>('local_model_download', { request });

/** Pauses an in-flight download (checkpoint kept); returns whether one was in flight */
export const localModelPauseDownload = (id: string) =>
  invoke<boolean>('local_model_pause_download', { id });

/** Cancels a download (deletes the checkpoint; cleans stale .part files when idle) */
export const localModelCancelDownload = (id: string) =>
  invoke<boolean>('local_model_cancel_download', { id });

/** Deletes an installed local model (removes its directory) */
export const localModelDelete = (id: string) => invoke<void>('local_model_delete', { id });

/** Lists installed local models (scans the install directory) */
export const localModelInstalledList = () =>
  invoke<InstalledLocalModel[]>('local_model_installed_list');

/** Current local model install directory */
export const localModelsGetDir = () => invoke<string>('local_models_get_dir');

/**
 * Sets the install directory (empty restores the default). Downloaded models are
 * migrated and provider localModelPaths entries are rewritten automatically.
 */
export const localModelsSetDir = (dir: string) => invoke<string>('local_models_set_dir', { dir });
