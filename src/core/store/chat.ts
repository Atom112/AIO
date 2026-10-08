// 对话核心域：datas（助手/话题/消息）+ 应用配置 + MCP 状态 + Skills + 标题重命名 —— 从 store.ts 拆出。

import { createStore } from 'solid-js/store';
import { createEffect, createSignal } from 'solid-js';
import {
  deleteAssistant as deleteAssistantApi,
  saveAssistant as saveAssistantApi,
} from '../api/chat';
import { listen } from '@tauri-apps/api/event';
import { listSkills } from '../api/skills';
import { listMcpServerStatuses, listMcpServers, listMcpTools, startMcpServer } from '../api/mcp';
import { EVENTS } from '../api/events';
import type {
  McpServerConfig,
  McpServerStatusInfo,
  ToolSpec,
  LlmToolCallPayload,
} from '../types/mcp';
import type { SkillConfig } from '../types/skill';
import type { ActivatedModel, AppConfig, Assistant, Topic } from './types';
import {
  activeProviderModels,
  currentAssistantId,
  isLocalModel,
  modelKey,
  selectedModel,
} from './models';
import { currentProjectId, projects } from './project';
import { isDarkMode, themeColor } from './ui-prefs';
export interface DatasState {
  /** 所有助手（含其话题与消息树） */
  assistants: Assistant[];
  /** 当前激活的模型列表，用于模型选择器 */
  activatedModels: ActivatedModel[];
}

export const [datas, setDatas] = createStore<DatasState>({
  assistants: [],
  activatedModels: [],
});

// ====== MCP 状态 ======

/** MCP 服务器配置表（id → config） */
export const [mcpServers, setMcpServers] = createSignal<Record<string, McpServerConfig>>({});
/** MCP 服务器运行时状态（id → status） */
export const [mcpServerStatus, setMcpServerStatus] = createSignal<
  Record<string, McpServerStatusInfo>
>({});
/** 当前对话可用的工具列表（合并所有已连接 server 的工具，供全局状态展示） */
export const [mcpToolsCache, setMcpToolsCache] = createSignal<ToolSpec[]>([]);
/** 全局 Skill 配置表（id → config）。 */
export const [skills, setSkills] = createSignal<Record<string, SkillConfig>>({});

/** LLM 工具调用事件总线（ChatPage 监听） */
export const [pendingToolCall, setPendingToolCall] = createSignal<LlmToolCallPayload | null>(null);

/**
 * 一次性安装 MCP 后台事件监听器（mcp-server-status / mcp-server-stderr）。
 * 监听器进程级唯一：`initMcpServers` 被多处并发调用时不会重复注册，
 * 避免旧实现里 `listen()` 未 await、监听器堆叠、事件丢失等竞态。
 */
let mcpListenersInstalled = false;
const installMcpListeners = async () => {
  if (mcpListenersInstalled) return;
  mcpListenersInstalled = true;
  // 监听器进程级唯一、随应用生命周期存活，无需显式 unlisten。
  await listen<McpServerStatusInfo>(EVENTS.mcpServerStatus, (event) => {
    const info = event.payload;
    setMcpServerStatus((prev) => ({ ...prev, [info.id]: info }));
  });
  await listen<{ id: string; line: string }>(EVENTS.mcpServerStderr, (event) => {
    const { id, line } = event.payload;
    if (import.meta.env.DEV) {
      console.debug(`[mcp:${id}] ${line}`);
    }
  });
  // 工具列表变化（notifications/tools/list_changed）：刷新状态与工具缓存
  await listen<{ id: string }>(EVENTS.mcpToolsChanged, async (event) => {
    const { id } = event.payload;
    try {
      const statusMap = await listMcpServerStatuses();
      setMcpServerStatus((prev) => ({ ...prev, ...statusMap }));
      await refreshMcpToolsCache();
      if (import.meta.env.DEV) {
        console.debug(`[mcp:${id}] tools/list_changed → 已刷新工具缓存`);
      }
    } catch {
      // 刷新失败不致命：下次打开设置页会重新拉取
    }
  });
};

/**
 * 加载并初始化 MCP 服务器列表 + 同步后端已连接状态 + 自动启动标记为 autoStart 的 server
 */
export const initMcpServers = async (projectId?: string | null) => {
  try {
    const list = await listMcpServers(projectId);
    const map: Record<string, McpServerConfig> = {};
    for (const cfg of list) map[cfg.id] = cfg;
    setMcpServers(map);

    const statusMap = await listMcpServerStatuses();
    setMcpServerStatus((prev) => ({ ...prev, ...statusMap }));

    // 先安装进程级唯一的后台事件监听器（await 成功后再触发 autoStart，避免事件丢失）
    await installMcpListeners();

    // 自动启动标记为 autoStart 的 server（跳过已连接的，避免重复启动）
    // （是否被某助手使用由 Assistant.mcpServerIds 在 list_mcp_tools_for_assistant 时过滤）
    const autoStartIds = Object.values(map)
      .filter((cfg) => cfg.autoStart && statusMap[cfg.id]?.status !== 'connected')
      .map((cfg) => cfg.id);
    if (autoStartIds.length > 0) {
      const pid = projectId ?? null;
      await Promise.allSettled(autoStartIds.map((id) => startMcpServerAndRefresh(id, pid)));
    }
  } catch (e) {
    console.warn('加载 MCP server 列表失败:', e);
  }
};

/**
 * 启动一个 MCP server 并刷新工具缓存
 */
export const startMcpServerAndRefresh = async (
  id: string,
  projectId?: string | null,
): Promise<ToolSpec[]> => {
  setMcpServerStatus((prev) => ({
    ...prev,
    [id]: { id, status: 'connecting', toolCount: 0, resourceCount: 0, promptCount: 0 },
  }));
  try {
    const tools = await startMcpServer(id, projectId);
    setMcpServerStatus((prev) => ({
      ...prev,
      [id]: { id, status: 'connected', toolCount: tools.length, resourceCount: 0, promptCount: 0 },
    }));
    await refreshMcpToolsCache();
    return tools;
  } catch (e) {
    setMcpServerStatus((prev) => ({
      ...prev,
      [id]: {
        id,
        status: 'error',
        message: String(e),
        toolCount: 0,
        resourceCount: 0,
        promptCount: 0,
      },
    }));
    throw e;
  }
};

/** 重新拉取所有已连接 server 的工具，更新 mcpToolsCache（全局状态展示用） */
export const refreshMcpToolsCache = async () => {
  try {
    const tools = await listMcpTools(currentProjectId());
    setMcpToolsCache(tools);
  } catch (e) {
    console.warn('刷新 MCP 工具缓存失败:', e);
  }
};

// 配置初始化与管理
// H5 加固：API Key 改存 Rust 侧 keyring；前端 localStorage 仅保留非敏感字段

/**
 * 前端配置的默认骨架（与后端 AppConfig::default() 对齐）。
 * 仅用于启动前的本地快照；权威值以 load_app_config 返回为准。
 */
const defaultAppConfig = (): AppConfig => ({
  apiUrl: '',
  apiKey: '',
  defaultModel: '',
  localModelPath: '',
  auto_retry_enabled: true,
  auto_retry_count: 2,
  auto_retry_delay_ms: 500,
  memoryEnabled: false,
  memoryAutoInject: true,
  memoryAutoExtract: true,
  memoryMaxFacts: 2000,
  memoryInjectionBudgetTokens: 3000,
  memoryExtractDebounceSecs: 60,
  memoryEmbedding: {
    provider: 'local',
    model: '',
    apiUrl: '',
    dimensions: 0,
    enabled: false,
  },
  embeddingModelsDir: '',
  embeddingModels: [],
  memoryRerank: { enabled: false, provider: 'local', model: '', apiUrl: '' },
  memoryDedupThreshold: 0.92,
  memoryQueryRewrite: false,
  memoryConsolidateBudget: 20,
  autoStartEnabled: false,
  maxConcurrentSubagents: undefined,
  maxToolRounds: undefined,
});

const initialConfig: AppConfig = (() => {
  try {
    const raw = localStorage.getItem('app_config');
    if (!raw) return defaultAppConfig();
    const parsed = JSON.parse(raw);
    // 兼容旧数据：保留非敏感字段，清空 apiKey（由 Rust 重新加载）
    return {
      ...defaultAppConfig(),
      apiUrl: parsed.apiUrl || '',
      apiKey: '',
      defaultModel: parsed.defaultModel || '',
    };
  } catch {
    return defaultAppConfig();
  }
})();

/** 应用配置信号，存储 API URL 和 Key 等全局设置 */
export const [config, setConfig] = createSignal<AppConfig>(initialConfig);

// 后端持久化逻辑
/**
 * 将指定助手数据持久化保存到后端
 * @param id - 要保存的助手唯一标识符
 */
export const saveSingleAssistantToBackend = async (id: string) => {
  const asst = datas.assistants.find((a) => a.id === id);
  if (!asst) return;

  try {
    // 深度克隆对象以解除 SolidJS 响应式代理，确保可序列化
    await saveAssistantApi(JSON.parse(JSON.stringify(asst)));
  } catch (err) {
    console.error('保存助手失败:', err);
  }
};

// ====== 话题标题自动重命名 ======

/**
 * 待处理的重命名请求信号。
 * ChatPage 组件通过 createEffect 监听此信号，触发实际的 LLM 调用。
 * 消费后立即清空，避免重复触发。
 */
export const [pendingRenameRequest, setPendingRenameRequest] = createSignal<{
  asstId: string;
  topicId: string;
} | null>(null);

/**
 * 触发话题标题的重新生成（手动入口）。
 * 用于右键菜单的"重新生成标题"按钮。
 * 行为：
 *   1. 校验话题存在且不是默认话题（默认话题永远不重命名）
 *   2. 重置 `renamed = false`，让后续的 checkAndRename 能正常工作
 *   3. 持久化到后端
 *   4. 通过 pendingRenameRequest 信号通知 ChatPage 执行实际的 LLM 调用
 * @returns 是否成功发起请求（默认话题 / 不存在则返回 false）
 */
export const requestRenameTopic = (asstId: string, topicId: string): boolean => {
  const asst = datas.assistants.find((a: any) => a.id === asstId);
  const topic = asst?.topics.find((t: Topic) => t.id === topicId);
  if (!asst || !topic) return false;
  // 默认话题（每个助手的话题列表第一项）永远不参与自动重命名
  if (asst.topics[0]?.id === topic.id) return false;

  // 重置标志位，使 checkAndRename 能继续工作
  setDatas(
    'assistants',
    (a: any) => a.id === asstId,
    'topics',
    (t: Topic) => t.id === topicId,
    'renamed',
    false,
  );
  // 异步持久化（不阻塞 UI）
  void saveSingleAssistantToBackend(asstId);

  // 通过信号通知 ChatPage
  setPendingRenameRequest({ asstId, topicId });
  return true;
};

/**
 * 从后端物理删除助手配置文件
 * @param id - 要删除的助手唯一标识符
 */
export const deleteAssistantFile = async (id: string) => {
  try {
    await deleteAssistantApi(id);
  } catch (err) {
    console.error('物理删除失败:', err);
  }
};

// 监听主题颜色变化并同步到 CSS 变量和本地存储
createEffect(() => {
  const color = themeColor();
  // 打标记全局禁用过渡, 确保主题色切换即时无动画
  document.documentElement.setAttribute('data-theme-changing', '');
  document.documentElement.style.setProperty('--primary-color', color);
  // 同步 RGB 分量供 rgba(var(--primary-rgb), 0.xx) 使用
  const r = parseInt(color.slice(1, 3), 16);
  const g = parseInt(color.slice(3, 5), 16);
  const b = parseInt(color.slice(5, 7), 16);
  if (!isNaN(r) && !isNaN(g) && !isNaN(b)) {
    document.documentElement.style.setProperty('--primary-rgb', `${r}, ${g}, ${b}`);
  }
  localStorage.setItem('theme-color', color);
  // 下一帧移除标记, 恢复正常的交互过渡动画
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      document.documentElement.removeAttribute('data-theme-changing');
    });
  });
});

// 同步深色模式到 HTML data-theme 属性，并注入边框亮度修复
createEffect(() => {
  const dark = isDarkMode();
  document.documentElement.setAttribute('data-theme-changing', '');
  document.documentElement.setAttribute('data-theme', dark ? 'dark' : 'light');
  localStorage.setItem('dark-mode', String(dark));

  // 动态注入/移除暗色模式边框亮度样式（运行时注入，确保优先级最高）
  const STYLE_ID = 'aio-dark-border-fix';
  if (dark) {
    if (!document.getElementById(STYLE_ID)) {
      const el = document.createElement('style');
      el.id = STYLE_ID;
      el.textContent = [
        '[data-theme="dark"] .border-white\\/5{--tw-border-opacity:1;border-color:rgba(240,240,240,0.3)!important}',
        '[data-theme="dark"] .border-white\\/10{--tw-border-opacity:1;border-color:rgba(240,240,240,0.3)!important}',
        '[data-theme="dark"] .border-white\\/15{--tw-border-opacity:1;border-color:rgba(240,240,240,0.3)!important}',
        '[data-theme="dark"] .border-white\\/20{--tw-border-opacity:1;border-color:rgba(240,240,240,0.3)!important}',
        '[data-theme="dark"] .border-white\\/\\[0\\.04\\]{border-color:rgba(240,240,240,0.3)!important}',
        '[data-theme="dark"] .border-white\\/\\[0\\.05\\]{border-color:rgba(240,240,240,0.3)!important}',
        '[data-theme="dark"] .border-white\\/\\[0\\.06\\]{border-color:rgba(240,240,240,0.3)!important}',
        '[data-theme="dark"] .border-white\\/\\[0\\.08\\]{border-color:rgba(240,240,240,0.3)!important}',
        '[data-theme="dark"] .border-white\\/\\[0\\.15\\]{border-color:rgba(240,240,240,0.3)!important}',
      ].join('');
      document.head.appendChild(el);
    }
  } else {
    const el = document.getElementById(STYLE_ID);
    if (el) el.remove();
  }

  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      document.documentElement.removeAttribute('data-theme-changing');
    });
  });
});

/** 从后端加载 Skill 配置（支持项目级合并）。 */
export const initSkills = async (projectId?: string | null) => {
  try {
    const list = await listSkills(projectId);
    setSkills(Object.fromEntries(list.map((skill) => [skill.id, skill])));
  } catch (e) {
    console.warn('加载 Skill 列表失败:', e);
  }
};

/** 返回某个助手启用且仍存在的 Skill，顺序与助手配置一致。 */
export const resolveAssistantSkills = (assistant: Assistant | undefined | null): SkillConfig[] => {
  if (!assistant) return [];
  const skillMap = skills();
  return (assistant.skillIds ?? [])
    .map((id) => skillMap[id])
    .filter((skill): skill is SkillConfig => Boolean(skill));
};

/**
 * 当前可用的全部模型列表（云端 + 本地合并）。
 * 云端模型来自 providerConfigs，本地引擎模型通过 providerConfigs 的 localModelPaths 注入。
 * 供助手设置弹窗的模型选择器与 resolveAssistantModel 解析使用。
 */
export const allAvailableModels = (): ActivatedModel[] => {
  const cloud = activeProviderModels().map(
    (m) =>
      ({
        model_id: m.modelId,
        owned_by: m.providerName,
        api_url: m.apiUrl,
        api_key: m.apiKey,
        ...(m.engine_type
          ? { engine_type: m.engine_type, ...(m.local_path ? { local_path: m.local_path } : {}) }
          : {}),
      }) as ActivatedModel & { provider_id: string },
  );
  // 旧版本地模型：仅保留与 providerConfigs 不重叠的（向后兼容）
  const providerModelIds = new Set(cloud.map((m) => m.model_id + '@' + m.api_url));
  const local = datas.activatedModels.filter(
    (m) => isLocalModel(m) && !providerModelIds.has(m.model_id + '@' + (m.api_url ?? '')),
  );
  return [...cloud, ...local];
};

/**
 * 解析某助手实际应使用的模型。
 * 优先按复合键 modelKey 匹配助手绑定的 modelId；若绑定的是旧式纯 model_id（无 @），
 * 则退化为按 model_id 匹配首个命中项，保证向后兼容。
 * 未绑定或匹配不到时回退到当前全局 selectedModel，再不行则取首个可用模型。
 * 返回 null 表示当前没有任何可用模型。
 */
export const resolveAssistantModel = (
  asst: Assistant | undefined | null,
): ActivatedModel | null => {
  if (!asst) return null;
  const all = allAvailableModels();
  if (asst.modelId) {
    // 1. 优先按复合键精确匹配
    const exact = all.find((m) => modelKey(m) === asst.modelId);
    if (exact) return exact;
    // 2. 兼容旧数据：绑定的 modelId 不含 @ 时，按 model_id 兜底匹配
    if (!asst.modelId.includes('@')) {
      const legacy = all.find((m) => m.model_id === asst.modelId);
      if (legacy) return legacy;
    }
  }
  const cur = selectedModel();
  if (cur) return cur;
  return all[0] ?? null;
};

/** 判断当前是否是聊天模式（纯对话，无工具调用） */
export const isChatMode = (): boolean => {
  const asst = datas.assistants.find((a) => a.id === currentAssistantId());
  return asst?.agentMode === 'off' && !asst?.projectId;
};

/** 获取或创建项目对应的助理 ID */
export const ensureProjectAssistant = async (projectId: string): Promise<string> => {
  let asst = datas.assistants.find((a) => a.projectId === projectId);
  if (!asst) {
    const project = projects().find((p) => p.id === projectId);
    if (!project) throw new Error('Project not found');
    asst = {
      id: `asst-${projectId}`,
      name: project.name,
      prompt: '',
      agentMode: 'normal',
      assistantType: 'project',
      projectId,
      mcpServerIds: ['__aio-filesystem__'],
      skillIds: [],
      topics: [{ id: Date.now().toString(), name: '默认话题', history: [], summary: '' }],
    };
    setDatas('assistants', (prev) => [...prev, asst!]);
    await saveSingleAssistantToBackend(asst.id);
  }
  return asst.id;
};
