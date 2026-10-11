// Chat core domain: datas (assistants/topics/messages) + app config + MCP state + skills + title renaming — split out of store.ts.

import { createStore } from 'solid-js/store';
import { createEffect, createSignal } from 'solid-js';
import {
  deleteAssistant as deleteAssistantApi,
  saveAssistant as saveAssistantApi,
} from '../api/chat';
import { listen } from '@tauri-apps/api/event';
import { listSkills } from '../api/skills';
import {
  listMcpServerStatuses,
  listMcpServers,
  listMcpTools,
  listUntrustedProjectMcpServers,
  startMcpServer,
  trustProjectMcpServer,
} from '../api/mcp';
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
import { selectAutoStartIds } from '../utils/mcp';
export interface DatasState {
  /** All assistants, including their topics and message trees */
  assistants: Assistant[];
  /** Currently activated models, used by the model selector */
  activatedModels: ActivatedModel[];
}

export const [datas, setDatas] = createStore<DatasState>({
  assistants: [],
  activatedModels: [],
});

// ====== MCP state ======

/** MCP server config map (id → config) */
export const [mcpServers, setMcpServers] = createSignal<Record<string, McpServerConfig>>({});
/** MCP server runtime status map (id → status) */
export const [mcpServerStatus, setMcpServerStatus] = createSignal<
  Record<string, McpServerStatusInfo>
>({});
/** Tools available in the current conversation (merged from all connected servers, for global status display) */
export const [mcpToolsCache, setMcpToolsCache] = createSignal<ToolSpec[]>([]);
/** Global skill config map (id → config). */
export const [skills, setSkills] = createSignal<Record<string, SkillConfig>>({});

/**
 * 项目级 MCP 待确认队列（P0-1 信任门的前端侧）。
 * 打开/切换项目时填充来自 `.aio/mcp-servers.json` 且尚未信任的服务器；
 * autoStart 对它们一律跳过，直到用户确认（落盘信任标记）或忽略（仅本次会话）。
 */
export const [pendingProjectMcpServers, setPendingProjectMcpServers] = createSignal<
  McpServerConfig[]
>([]);
/** 待确认队列归属的项目 ID。 */
export const [pendingProjectMcpProjectId, setPendingProjectMcpProjectId] = createSignal<
  string | null
>(null);

/** LLM tool call event bus (listened to by ChatPage) */
export const [pendingToolCall, setPendingToolCall] = createSignal<LlmToolCallPayload | null>(null);

/**
 * Install the MCP background event listeners (mcp-server-status / mcp-server-stderr) once.
 * The listeners are process-level singletons: concurrent `initMcpServers` calls never double-register,
 * which prevents listener stacking and lost events.
 */
let mcpListenersInstalled = false;
const installMcpListeners = async () => {
  if (mcpListenersInstalled) return;
  mcpListenersInstalled = true;
  // The listeners are process-level singletons living for the whole app lifetime; no explicit unlisten needed.
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
  // Tool list changed (notifications/tools/list_changed): refresh status and tool cache
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
      // Refresh failure is non-fatal: the settings page re-fetches on next open
    }
  });
};

/**
 * Load and initialize the MCP server list, sync backend connection status, and auto-start servers marked autoStart.
 * Project-repo servers that are not trusted yet never auto-start; they enter the pending-confirmation queue instead.
 */
export const initMcpServers = async (projectId?: string | null) => {
  try {
    const list = await listMcpServers(projectId);
    const map: Record<string, McpServerConfig> = {};
    for (const cfg of list) map[cfg.id] = cfg;
    setMcpServers(map);

    const statusMap = await listMcpServerStatuses();
    setMcpServerStatus((prev) => ({ ...prev, ...statusMap }));

    // Install the process-level background listeners first (await them before autoStart to avoid lost events)
    await installMcpListeners();

    // P0-1: project-level untrusted servers go to the confirmation queue instead of auto-starting
    let untrustedIds = new Set<string>();
    if (projectId) {
      try {
        const untrusted = await listUntrustedProjectMcpServers(projectId);
        if (untrusted.length > 0) {
          untrustedIds = new Set(untrusted.map((cfg) => cfg.id));
          setPendingProjectMcpProjectId(projectId);
          setPendingProjectMcpServers(untrusted);
        } else {
          setPendingProjectMcpProjectId(null);
          setPendingProjectMcpServers([]);
        }
      } catch {
        // Untrusted listing is best-effort; the backend start gate still refuses untrusted spawns
      }
    }

    // Auto-start servers marked autoStart (skip connected + untrusted project servers)
    // (Whether an assistant uses a server is filtered by Assistant.mcpServerIds in list_mcp_tools_for_assistant)
    const autoStartIds = selectAutoStartIds(map, statusMap, untrustedIds);
    if (autoStartIds.length > 0) {
      const pid = projectId ?? null;
      await Promise.allSettled(autoStartIds.map((id) => startMcpServerAndRefresh(id, pid)));
    }
  } catch (e) {
    console.warn('Failed to load MCP server list:', e);
  }
};

/**
 * Confirm trust for one project-level MCP server: persist the trust marker
 * (config fingerprint, repo changes revoke it), then start the server.
 */
export const confirmProjectMcpServer = async (cfg: McpServerConfig): Promise<void> => {
  const pid = pendingProjectMcpProjectId() ?? currentProjectId();
  if (!pid) return;
  try {
    await trustProjectMcpServer(pid, cfg.id);
    setPendingProjectMcpServers((prev) => prev.filter((c) => c.id !== cfg.id));
    if (pendingProjectMcpServers().length === 0) {
      setPendingProjectMcpProjectId(null);
    }
    await startMcpServerAndRefresh(cfg.id, pid).catch(() => {
      // Start failure is surfaced via mcp-server-status; trust itself already succeeded
    });
  } catch (e) {
    console.warn('Failed to trust project MCP server:', e);
  }
};

/** Dismiss one pending project MCP server (this session only; re-prompted on next app start). */
export const dismissProjectMcpServer = (id: string) => {
  setPendingProjectMcpServers((prev) => prev.filter((c) => c.id !== id));
  if (pendingProjectMcpServers().length === 0) {
    setPendingProjectMcpProjectId(null);
  }
};

/**
 * Start an MCP server and refresh the tool cache
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

/** Re-fetch tools from all connected servers and update mcpToolsCache (for global status display) */
export const refreshMcpToolsCache = async () => {
  try {
    const tools = await listMcpTools(currentProjectId());
    setMcpToolsCache(tools);
  } catch (e) {
    console.warn('Failed to refresh MCP tool cache:', e);
  }
};

// Config initialization and management
// H5 hardening: API keys are stored in the Rust-side keyring; frontend localStorage keeps only non-sensitive fields

/**
 * Default skeleton of the frontend config (aligned with backend AppConfig::default()).
 * Only a pre-startup local snapshot; the authoritative value is what load_app_config returns.
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
  memoryRerank: { enabled: false, provider: 'local', model: '' },
  memoryDedupThreshold: 0.92,
  memoryQueryRewrite: false,
  memoryConsolidateBudget: 20,
  autoStartEnabled: false,
  maxConcurrentSubagents: undefined,
  maxToolRounds: undefined,
  subagentIsolationMode: 'auto',
});

const initialConfig: AppConfig = (() => {
  try {
    const raw = localStorage.getItem('app_config');
    if (!raw) return defaultAppConfig();
    const parsed = JSON.parse(raw);
    // Old-data compat: keep non-sensitive fields, clear apiKey (reloaded by Rust)
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

/** App config signal holding global settings such as API URL and key */
export const [config, setConfig] = createSignal<AppConfig>(initialConfig);

// Backend persistence
/**
 * Persist the given assistant's data to the backend
 * @param id - unique identifier of the assistant to save
 */
export const saveSingleAssistantToBackend = async (id: string) => {
  const asst = datas.assistants.find((a) => a.id === id);
  if (!asst) return;

  try {
    // Deep-clone to strip the SolidJS reactive proxy so the value is serializable
    await saveAssistantApi(JSON.parse(JSON.stringify(asst)));
  } catch (err) {
    console.error('Failed to save assistant:', err);
  }
};

// ====== Topic title auto-rename ======

/**
 * Signal for a pending rename request.
 * ChatPage watches it via createEffect and performs the actual LLM call.
 * Cleared right after consumption to avoid duplicate triggers.
 */
export const [pendingRenameRequest, setPendingRenameRequest] = createSignal<{
  asstId: string;
  topicId: string;
} | null>(null);

/**
 * Trigger regeneration of a topic title (manual entry point).
 * Used by the "Regenerate title" context-menu button.
 * Behavior:
 *   1. Validate the topic exists and is not the default topic (default topics are never renamed)
 *   2. Reset `renamed = false` so a later checkAndRename can proceed
 *   3. Persist to the backend
 *   4. Notify ChatPage via the pendingRenameRequest signal to perform the actual LLM call
 * @returns Whether the request was issued (false for the default or a missing topic)
 */
export const requestRenameTopic = (asstId: string, topicId: string): boolean => {
  const asst = datas.assistants.find((a: any) => a.id === asstId);
  const topic = asst?.topics.find((t: Topic) => t.id === topicId);
  if (!asst || !topic) return false;
  // The default topic (first entry in each assistant's topic list) never takes part in auto-rename
  if (asst.topics[0]?.id === topic.id) return false;

  // Reset the flag so checkAndRename can proceed
  setDatas(
    'assistants',
    (a: any) => a.id === asstId,
    'topics',
    (t: Topic) => t.id === topicId,
    'renamed',
    false,
  );
  // Persist asynchronously (does not block the UI)
  void saveSingleAssistantToBackend(asstId);

  // Notify ChatPage via the signal
  setPendingRenameRequest({ asstId, topicId });
  return true;
};

/**
 * Physically delete an assistant config file from the backend
 * @param id - unique identifier of the assistant to delete
 */
export const deleteAssistantFile = async (id: string) => {
  try {
    await deleteAssistantApi(id);
  } catch (err) {
    console.error('Failed to hard-delete:', err);
  }
};

// Watch theme color changes and sync them to CSS variables and localStorage
createEffect(() => {
  const color = themeColor();
  // Set a flag to globally disable transitions so the theme color switch is instant, without animation
  document.documentElement.setAttribute('data-theme-changing', '');
  document.documentElement.style.setProperty('--primary-color', color);
  // Sync RGB components for use by rgba(var(--primary-rgb), 0.xx)
  const r = parseInt(color.slice(1, 3), 16);
  const g = parseInt(color.slice(3, 5), 16);
  const b = parseInt(color.slice(5, 7), 16);
  if (!isNaN(r) && !isNaN(g) && !isNaN(b)) {
    document.documentElement.style.setProperty('--primary-rgb', `${r}, ${g}, ${b}`);
  }
  localStorage.setItem('theme-color', color);
  // Remove the flag on the next frame to restore normal transition animations
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      document.documentElement.removeAttribute('data-theme-changing');
    });
  });
});

// Sync dark mode to the HTML data-theme attribute and inject the border brightness fix
createEffect(() => {
  const dark = isDarkMode();
  document.documentElement.setAttribute('data-theme-changing', '');
  document.documentElement.setAttribute('data-theme', dark ? 'dark' : 'light');
  localStorage.setItem('dark-mode', String(dark));

  // Dynamically inject/remove the dark-mode border brightness styles (injected at runtime for highest priority)
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

/** Load skill configs from the backend (supports project-level merge). */
export const initSkills = async (projectId?: string | null) => {
  try {
    const list = await listSkills(projectId);
    setSkills(Object.fromEntries(list.map((skill) => [skill.id, skill])));
  } catch (e) {
    console.warn('Failed to load skill list:', e);
  }
};

/** Return an assistant's enabled skills that still exist, in the assistant's configured order. */
export const resolveAssistantSkills = (assistant: Assistant | undefined | null): SkillConfig[] => {
  if (!assistant) return [];
  const skillMap = skills();
  return (assistant.skillIds ?? [])
    .map((id) => skillMap[id])
    .filter((skill): skill is SkillConfig => Boolean(skill));
};

/**
 * All currently available models (cloud + local merged).
 * Cloud models come from providerConfigs; local engine models are injected via localModelPaths on providerConfigs.
 * Used by the model selector in the assistant settings dialog and by resolveAssistantModel.
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
  // Legacy local models: keep only those not overlapping providerConfigs (backward compat)
  const providerModelIds = new Set(cloud.map((m) => m.model_id + '@' + m.api_url));
  const local = datas.activatedModels.filter(
    (m) => isLocalModel(m) && !providerModelIds.has(m.model_id + '@' + (m.api_url ?? '')),
  );
  return [...cloud, ...local];
};

/**
 * Resolve which model an assistant should actually use.
 * First match the assistant's bound modelId by the composite modelKey; if it is a legacy plain
 * model_id (no @), fall back to the first match by model_id for backward compatibility.
 * If unbound or unmatched, fall back to the current global selectedModel, then to the first available model.
 * Returns null when no model is currently available.
 */
export const resolveAssistantModel = (
  asst: Assistant | undefined | null,
): ActivatedModel | null => {
  if (!asst) return null;
  const all = allAvailableModels();
  if (asst.modelId) {
    // 1. Prefer an exact match on the composite key
    const exact = all.find((m) => modelKey(m) === asst.modelId);
    if (exact) return exact;
    // 2. Old-data compat: when the bound modelId has no @, fall back to matching by model_id
    if (!asst.modelId.includes('@')) {
      const legacy = all.find((m) => m.model_id === asst.modelId);
      if (legacy) return legacy;
    }
  }
  const cur = selectedModel();
  if (cur) return cur;
  return all[0] ?? null;
};

/** Whether the current mode is chat mode (plain conversation, no tool calls) */
export const isChatMode = (): boolean => {
  const asst = datas.assistants.find((a) => a.id === currentAssistantId());
  return asst?.agentMode === 'off' && !asst?.projectId;
};

/** Get or create the assistant ID for a project */
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
