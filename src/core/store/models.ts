// 模型域：当前选中模型 / 模型目录 / 供应商配置 / 模型键与可用模型解析 —— 从 store.ts 拆出。
// 叶域：仅依赖 types 与 core/utils/models，不依赖其他 store 域。

import { createSignal } from 'solid-js';
import type { ActivatedModel } from './types';
import type { Catalog, CatalogSourceTag, ProviderConfig } from '../utils/models';
import { isLocalEngineProvider, getLocalEngineProvider } from '../utils/models';
export const [selectedModel, setSelectedModel] = createSignal<ActivatedModel | null>(null);
/** 当前选中的助手 ID 信号，用于侧边栏助手切换 */
export const [currentAssistantId, setCurrentAssistantId] = createSignal<string | null>(null);
/** 当前选中的话题 ID 信号，用于 Chat 页面跟踪当前对话 */
export const [currentTopicId, setCurrentTopicId] = createSignal<string | null>(null);

/** 模型目录信号（来自 @aio/models-data 同步数据，启动时由 Layout 加载） */
export const [modelsCatalog, setModelsCatalog] = createSignal<Catalog | null>(null);
/** 模型目录加载状态：'idle' | 'loading' | 'ready' | 'failed' */
export const [modelsCatalogStatus, setModelsCatalogStatus] = createSignal<
  'idle' | 'loading' | 'ready' | 'failed'
>('idle');
/** 当前 catalog 来源标签 */
export const [modelsCatalogSource, setModelsCatalogSource] =
  createSignal<CatalogSourceTag>('empty');
/** 当前 catalog 在磁盘上的路径（仅 AppData/Bundled 来源有值） */
export const [modelsCatalogPath, setModelsCatalogPath] = createSignal<string | null>(null);
/** 当前 catalog 版本号（来自 models.dev） */
export const [modelsCatalogVersion, setModelsCatalogVersion] = createSignal<string | null>(null);
/** 当前 catalog 生成时间（ISO 字符串） */
export const [modelsCatalogGeneratedAt, setModelsCatalogGeneratedAt] = createSignal<string | null>(
  null,
);

/** Provider 配置 (lobehub 形态)，key = provider id */
export const [providerConfigs, setProviderConfigs] = createSignal<Record<string, ProviderConfig>>(
  {},
);

/** 从 providerConfigs 派生的可用模型列表（启用 provider 的启用模型） */
export interface ActiveModelEntry {
  provider: string;
  providerName: string;
  modelId: string;
  apiUrl: string;
  apiKey: string;
  isCustom: boolean;
  /** 本地引擎模型文件路径 */
  local_path?: string;
  /** 本地引擎类型 */
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
      // 本地引擎：附加 engine_type（始终设置）+ local_path（有路径时）
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

/** 判断是否为本地模型（有 local_path 或 engine_type） */
export const isLocalModel = (m: ActivatedModel): boolean => !!(m.local_path || m.engine_type);

/**
 * 生成模型在助手绑定中的唯一标识键。
 * 云端模型用 `model_id@api_url` 复合键，以区分「同名模型来自不同 provider」的情况
 * （例如 OpenAI 和 OpenRouter 都提供 gpt-4o，仅靠 model_id 无法区分，会导致高亮错乱）。
 * 本地模型没有稳定的 api_url，退化为纯 model_id。
 */
export const modelKey = (m: ActivatedModel): string => {
  if (isLocalModel(m) || !m.api_url) return m.model_id;
  return `${m.model_id}@${m.api_url}`;
};
