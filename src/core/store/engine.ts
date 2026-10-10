// Local inference engine domain: start / progress / connection state / engine scan cache.
// Dependency direction: engine → models (model selection), engine → chat (persisting assistant bindings); one-way, acyclic.

import { createSignal } from 'solid-js';
import {
  isLocalServerRunning,
  probeEngineHealth,
  scanInstalledEngines,
  startLocalServer,
} from '../api/engine';
import { t } from '../i18n';
import type { ActivatedModel } from './types';
import { isLocalModel, modelKey, setSelectedModel } from './models';
import { saveSingleAssistantToBackend, setDatas } from './chat';
import { isLocalAutoStartConfirmed, setLocalAutoStartConfirmed } from './ui-prefs';
/** engine_type → display name */
const engineDisplayName = (engineType?: string): string => {
  switch (engineType) {
    case 'llama_cpp':
      return 'llama.cpp';
    case 'vllm':
      return 'vLLM';
    case 'ollama':
      return 'Ollama';
    default:
      return engineType || '';
  }
};
/**
 * Check whether the local inference engine service is ready (2s timeout).
 */
const checkServerHealth = async (baseUrl: string): Promise<boolean> => {
  try {
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), 2000);
    const rootUrl = baseUrl.replace('/v1', '');
    const resp = await fetch(`${rootUrl}/health`, { signal: controller.signal });
    clearTimeout(timeoutId);
    return resp.ok;
  } catch {
    return false;
  }
};

/**
 * Start the local inference engine for the selected model.
 * Engine connection state is exposed through localEngineConnected / localEngineName signals;
 * @param model - The local model (needs local_path or engine_type for Ollama)
 */
export const startLocalEngineForAssistant = async (
  model: ActivatedModel,
  _asstId: string,
): Promise<void> => {
  void _asstId;
  const isExternalEngine = !model.local_path && !!model.engine_type;
  if (!model.local_path && !isExternalEngine) return;

  const engineType = model.engine_type || 'llama_cpp';

  if (!isExternalEngine) {
    if (!isLocalAutoStartConfirmed()) {
      const ok = confirm(
        t('provider.autoStartConfirm', { name: model.model_id, path: model.local_path! }),
      );
      if (!ok) return;
      setLocalAutoStartConfirmed();
    }
    const engineName = engineDisplayName(model.engine_type);
    // Ollama: always call start_local_server to ensure the model is imported (the plugin handles the already-running case)
    // Other engines: skip if already running
    if (engineType !== 'ollama') {
      const isRunning = await isLocalServerRunning(engineType);
      if (isRunning) {
        setLocalEngineName(engineName);
        setLocalEngineConnected(true);
        return;
      }
    }
  }

  setLocalEngineName(engineDisplayName(model.engine_type));

  try {
    setIsStartingLocalModel(true);
    setLocalModelStartProgress(0);

    if (!isExternalEngine) {
      const port = engineType === 'ollama' ? 11434 : engineType === 'vllm' ? 8000 : 8080;
      await startLocalServer({
        modelPath: model.local_path!,
        port,
        gpuLayers: 99,
        engineType,
        trustRemoteCode: engineType === 'vllm' ? window.confirm(t('provider.vllmWarning')) : false,
      });
    }

    const healthBaseUrl = model.api_url.replace(/\/+$/, '');
    let attempts = 0;
    const maxAttempts = isExternalEngine ? 5 : 60;
    const poll = setInterval(async () => {
      attempts++;
      const isReady =
        isExternalEngine || engineType === 'ollama'
          ? await probeEngineHealth(healthBaseUrl, model.engine_type ?? '').catch(() => false)
          : await checkServerHealth(healthBaseUrl);
      if (isReady) {
        clearInterval(poll);
        setLocalModelStartProgress(100);
        setLocalEngineConnected(true);
        setTimeout(() => {
          setIsStartingLocalModel(false);
          setLocalModelStartProgress(0);
        }, 1000);
      } else if (attempts >= maxAttempts) {
        clearInterval(poll);
        setLocalModelStartProgress(100);
        setLocalEngineConnected(false);
        setTimeout(() => {
          setIsStartingLocalModel(false);
          setLocalModelStartProgress(0);
        }, 1000);
      }
    }, 500);
  } catch {
    setLocalModelStartProgress(100);
    setLocalEngineConnected(false);
    setTimeout(() => {
      setIsStartingLocalModel(false);
      setLocalModelStartProgress(0);
    }, 1000);
  }
};

/**
 * Set the bound model for an assistant, sync the global selectedModel immediately, and persist.
 * If the model is local, also start the inference engine. Reused by the assistant settings modal.
 */
export const setAssistantModel = async (asstId: string, model: ActivatedModel): Promise<void> => {
  // Store the composite modelKey (model_id@api_url or plain model_id) so same-named models from different sources stay distinguishable.
  setDatas('assistants', (a: any) => a.id === asstId, 'modelId', modelKey(model));
  setSelectedModel(model);
  await saveSingleAssistantToBackend(asstId);
  if (isLocalModel(model) && model.local_path) {
    void startLocalEngineForAssistant(model, asstId);
  }
};

/** Whether a local model is starting. */
export const [isStartingLocalModel, setIsStartingLocalModel] = createSignal(false);
/** Local model start progress percentage. */
export const [localModelStartProgress, setLocalModelStartProgress] = createSignal(0);

/** Whether the local inference engine is connected (health check passed). */
export const [localEngineConnected, setLocalEngineConnected] = createSignal(false);
/** Display name of the connected local inference engine (mapped from engine_type). */
export const [localEngineName, setLocalEngineName] = createSignal('');

// ====== Engine scan cache (scanned once at app startup) ======

export interface EngineScanEntry {
  id: string;
  name: string;
  installed: boolean;
  version: string | null;
  defaultPort: number;
  defaultApiUrl: string;
}

/** Engine scan results cache; null = not scanned yet. */
export const [engineScanResults, setEngineScanResults] = createSignal<EngineScanEntry[] | null>(
  null,
);

/** Scan installed inference engines once at app startup. */
export const initEngineScan = async () => {
  try {
    const results = await scanInstalledEngines();
    setEngineScanResults(results);
  } catch (e) {
    console.error('[engine] init scan failed:', e);
    setEngineScanResults([]);
  }
};
