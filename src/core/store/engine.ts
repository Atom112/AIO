// 本地推理引擎域：启动 / 进度 / 连接状态 / 引擎扫描缓存 —— 从 store.ts 拆出。
// 依赖方向：engine → models（选择模型）、engine → chat（持久化助手绑定）；单向无环。

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
/** engine_type → 展示名称 */
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
 * 检查本地推理引擎服务是否就绪（2s 超时）
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
    // Ollama：始终调用 start_local_server 以确保模型已导入（插件内部处理"已运行"情况）
    // 其他引擎：若已在运行则跳过
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
 * 为某助手设置绑定模型并立即同步全局 selectedModel + 持久化。
 * 若为本地模型则顺带拉起推理引擎。供助手设置弹窗复用。
 */
export const setAssistantModel = async (asstId: string, model: ActivatedModel): Promise<void> => {
  // 存复合键 modelKey（model_id@api_url 或纯 model_id），确保同名异源模型可区分
  setDatas('assistants', (a: any) => a.id === asstId, 'modelId', modelKey(model));
  setSelectedModel(model);
  await saveSingleAssistantToBackend(asstId);
  if (isLocalModel(model) && model.local_path) {
    void startLocalEngineForAssistant(model, asstId);
  }
};

/** 是否正在启动本地模型 */
export const [isStartingLocalModel, setIsStartingLocalModel] = createSignal(false);
/** 本地模型启动进度百分比 */
export const [localModelStartProgress, setLocalModelStartProgress] = createSignal(0);

/** 本地推理引擎是否已连接（健康检查通过） */
export const [localEngineConnected, setLocalEngineConnected] = createSignal(false);
/** 已连接的本地推理引擎展示名（由 engine_type 映射） */
export const [localEngineName, setLocalEngineName] = createSignal('');

// ====== 引擎扫描缓存（应用启动时扫一次） ======

export interface EngineScanEntry {
  id: string;
  name: string;
  installed: boolean;
  version: string | null;
  defaultPort: number;
  defaultApiUrl: string;
}

/** 引擎扫描结果缓存，null = 尚未扫描 */
export const [engineScanResults, setEngineScanResults] = createSignal<EngineScanEntry[] | null>(
  null,
);

/** 应用启动时扫描一次已安装的推理引擎 */
export const initEngineScan = async () => {
  try {
    const results = await scanInstalledEngines();
    setEngineScanResults(results);
  } catch (e) {
    console.error('[engine] init scan failed:', e);
    setEngineScanResults([]);
  }
};
