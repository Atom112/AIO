// 领域类型定义 —— 从 store.ts 拆出的纯接口层（无运行期依赖）。

import type { MemoryEmbeddingConfig, MemoryRerankConfig } from '../types/memory';

/** Agent 工作过程时间线步骤 */
export interface AgentStep {
  id: string;
  type: 'thinking' | 'tool_call' | 'content' | 'subagent';
  timestamp: number;
  status: 'running' | 'complete' | 'error';
  /** 步骤耗时（ms），步骤关闭时计算 */
  duration?: number;
  /** 思考步骤文本（仅 type=thinking） */
  thinkingText?: string;
  /** 工具调用详情（仅 type=tool_call） */
  toolCall?: ToolCallDisplay;
  /** 阶段性总结 / 回复文本（仅 type=content） */
  contentText?: string;
  // ===== 子智能体字段（仅 type=subagent） =====
  /** 子智能体唯一 ID */
  subagentId?: string;
  /** 子智能体类型标识（explorer | coder | general） */
  subagentProfile?: string;
  /** 子智能体显示名称 */
  subagentName?: string;
  /** 子智能体的任务描述摘要 */
  subagentTask?: string;
  /** 子智能体内部的步骤时间线 */
  subagentSteps?: SubagentStep[];
  /** 子智能体完成的最终结果 */
  subagentResult?: string;
  /** 完整工具结果（不截断），仅 type=tool_call 时有效 */
  fullResult?: string;
}

/** 子智能体步骤（内部时间线，结构与 AgentStep 相同但不含嵌套子智能体） */
export interface SubagentStep {
  id: string;
  type: 'thinking' | 'tool_call' | 'content' | 'streaming';
  timestamp: number;
  status: 'running' | 'complete' | 'error';
  duration?: number;
  summary?: string;
  toolName?: string;
  /** 所属 LLM 轮次（后端 subagent-step 载荷 round；旧持久化数据缺省） */
  round?: number;
}

/** 消息内容分块（多模态：文本 / 图片 URL；与 OpenAI content parts 对齐） */
export interface ContentPart {
  type: 'text' | 'image_url';
  text?: string;
  image_url?: { url: string };
}

/* 消息项接口，定义聊天消息的数据结构 */
export interface Message {
  id?: string; // 消息唯一 ID（工具消息/assistant 消息持久化需要）
  role: 'user' | 'assistant' | 'tool' | 'system'; // 消息发送者角色：'tool' 为工具执行结果，'system' 为系统指令
  /** 消息内容：纯文本字符串，或多模态分块数组（文本 + 图片） */
  content: string | ContentPart[];
  modelId?: string; // 生成回复的模型标识符，仅在 AI 助手回复时存在
  displayFiles?: AttachmentMeta[]; // 消息关联的附件元数据
  displayText?: string; // 用于界面显示的纯文本内容（已脱敏或解析处理）
  reasoning?: string; // 模型原生思维链（reasoning_content），仅 assistant 消息可能携带
  toolCallId?: string; // role="tool" 时对应触发的 tool_call id（OpenAI API 要求配对）
  name?: string; // role="tool" 时为被调用的函数名；role="assistant" 携带 tool_calls 时为 "assistant"
  toolCalls?: ToolCallDisplay[]; // role="assistant" 时携带模型发起的工具调用请求（含前端 UI 状态 state/result/error）
  agentStartTime?: number; // Agent 轮次开始时间戳（ms），用于计算工作耗时
  interimContent?: string; // 多轮 Agent 工作中，中间轮次的阶段性总结文本累积
  agentSteps?: AgentStep[]; // Agent 工作过程时间线（新），按时间顺序记录每个步骤
  inputTokens?: number; // 本轮/本消息输入 tokens 用量（跨轮累计，用于成本统计）
  outputTokens?: number; // 本轮/本消息输出 tokens 用量（跨轮累计，用于成本统计）
  contextTokens?: number; // 上下文峰值 tokens（最后一轮 API 调用的 input_tokens），用于上下文窗口进度条
  parentMessageId?: string; // 父消息 ID（用于会话分支树）
  branchIndex?: number; // 分支索引（缺省视为 0；旧数据可能无此字段）
  cachedInputTokens?: number; // 本轮缓存命中 tokens（OpenAI cached_tokens / DeepSeek prompt_cache_hit_tokens）
  images?: GeneratedImageInfo[]; // 模型生成的图片（assistant 消息），用于下载与历史回放
}

/** 前端展示用的工具调用（在 OpenAI tool_calls 基础上增加 UI 状态字段） */
export interface ToolCallDisplay {
  id: string;
  type: 'function';
  function: { name: string; arguments: string };
  /** UI 状态：calling / success / error */
  state?: 'calling' | 'success' | 'error';
  /** 工具返回的结构化结果 */
  result?: any;
  /** 错误信息（state=error 时） */
  error?: string;
  /** 文件变更详情（写/替换/删文件时由后端提供） */
  fileChanges?: FileChangeInfo[];
  /** 完整工具结果（未截断版）。content 是截断版（≤10000 字符）。 */
  fullContent?: string;
}

/** 单次文件修改记录 */
export interface FileChangeInfo {
  /** 文件路径（相对于项目根目录） */
  filePath: string;
  /** 操作类型: create | modify | delete */
  action: 'create' | 'modify' | 'delete';
  /** unified diff 文本 */
  diff: string;
  /** 增删行数概览 "+N -M" */
  summary: string;
}

export interface AttachmentMeta {
  id?: string;
  name: string;
  mimeType?: string;
  size?: number;
  /** 附件在磁盘上的绝对路径，用于渲染图片缩略图（convertFileSrc 资产 URL） */
  storagePath?: string;
}

/** 模型生成的图像元数据（assistant 消息），用于下载按钮与历史回放 */
export interface GeneratedImageInfo {
  name: string;
  mimeType: string;
  size: number;
  storagePath: string;
}

export interface StoredAttachment extends AttachmentMeta {
  id: string;
  mimeType: string;
  size: number;
  storagePath: string;
}

export interface PendingAttachment extends StoredAttachment {
  type: 'text' | 'image';
  previewUrl?: string;
}

/* 话题接口，定义对话主题的数据结构 */
export interface Topic {
  id: string; // 话题唯一标识符
  name: string; // 话题显示名称
  history: Message[]; // 该话题下的历史消息记录数组
  summary: string; // 话题的长期记忆摘要，对应数据库存储的 summary 字段
  /**
   * 是否已自动重命名。
   * - false：新建话题或迁移前的旧话题；首次对话结束后会被自动改名为 AI 生成的标题，并置为 true
   * - true：已自动重命名过，或用户手动改过名；后续不再触发自动重命名
   * - 缺省：兼容旧数据，等价于 false
   */
  renamed?: boolean;
  /** 分支来源消息 ID（此话题从哪条消息分支而来） */
  branchedFromMessageId?: string;
  /** 父话题 ID（用于话题树结构） */
  parentTopicId?: string;
}

/** Agent 执行模式 */
export type AgentMode = 'off' | 'normal' | 'auto' | 'plan' | 'workflow';

/* 助手接口，定义 AI 助手的数据结构 */
export interface Assistant {
  id: string; // 助手唯一标识符
  name: string; // 助手显示名称
  prompt: string; // 助手的系统提示词（Prompt）
  modelId?: string; // 助手绑定的首选模型 ID；未设置时回退到全局默认模型
  mcpServerIds?: string[]; // 助手启用的 MCP server id 列表；空/未设置 = 该助手不使用任何 MCP 工具（opt-in）
  skillIds?: string[]; // 助手启用的 Skill id 列表；空/未设置 = 不注入 Skill 指令
  projectId?: string; // 助手所属的项目 ID；未设置 = 全局助手
  agentMode?: AgentMode; // Agent 执行模式；'off' = 对话模式
  assistantType?: 'chat' | 'project'; // 助理类型：chat = 聊天模式，project = 项目助理
  topics: Topic[]; // 助手关联的话题列表
}

/* 应用基础配置接口，存储 API 连接等全局设置 */
/** 记忆嵌入模型的启用/停用设置条目。 */
export interface EmbeddingModelSetting {
  /** 模型 id（对应模型注册表条目） */
  id: string;
  /** 是否启用（缺省条目视为启用） */
  enabled: boolean;
}

/**
 * 应用程序全局配置（镜像后端 core/models/config.rs 的 AppConfig 序列化形状）。
 *
 * 注意：后端仅对常用字段做了 serde rename（camelCase），autoRetry* 等保留
 * snake_case（历史兼容）——此处按**实际序列化名**声明，切勿想当然改成 camelCase。
 */
export interface AppConfig {
  /** API 服务提供商的基础 URL 地址 */
  apiUrl: string;
  /** 用于身份验证的 API 密钥 */
  apiKey: string;
  /** 用户偏好的默认模型 ID */
  defaultModel: string;
  /** 本地模型目录路径 */
  localModelPath: string;
  /** 工具调用失败时是否自动重试（默认 true；序列化名保留 snake_case） */
  auto_retry_enabled: boolean;
  /** 自动重试次数上限（默认 2） */
  auto_retry_count: number;
  /** 每次重试间隔（毫秒，默认 500） */
  auto_retry_delay_ms: number;
  /** 项目级记忆（语义 RAG）总开关 */
  memoryEnabled: boolean;
  /** 是否自动注入检索到的记忆到系统提示词（默认 true） */
  memoryAutoInject: boolean;
  /** 是否后台自动提取事实（默认 true） */
  memoryAutoExtract: boolean;
  /** 每项目活跃事实上限（默认 2000） */
  memoryMaxFacts: number;
  /** 记忆注入系统提示词的 token 预算（默认 3000） */
  memoryInjectionBudgetTokens: number;
  /** 后台自动提取去抖秒数（默认 60） */
  memoryExtractDebounceSecs: number;
  /** 记忆嵌入配置（项目级嵌入的旧全局迁移源；仅 local） */
  memoryEmbedding: MemoryEmbeddingConfig;
  /** 全局嵌入模型安装目录（空 = 默认 app_data_dir/models/embed） */
  embeddingModelsDir: string;
  /** 全局嵌入模型启用/停用注册表 */
  embeddingModels: EmbeddingModelSetting[];
  /** 记忆重排配置（RRF 后二阶段精排；默认关） */
  memoryRerank: MemoryRerankConfig;
  /** 判重/仲裁触发的余弦相似度阈值（默认 0.92） */
  memoryDedupThreshold: number;
  /** 检索时是否用 LLM 改写查询（默认关） */
  memoryQueryRewrite: boolean;
  /** 每日后台合并整理的 LLM 仲裁调用预算上限（默认 20） */
  memoryConsolidateBudget: number;
  /** 系统自启（默认关） */
  autoStartEnabled: boolean;
  /** 最大并发子智能体数量（undefined = 默认 5） */
  maxConcurrentSubagents?: number;
  /** 单次 Agent 运行最大工具调用轮数（undefined = 默认） */
  maxToolRounds?: number;
}

/* 已激活模型配置接口，定义可用 AI 模型的连接信息 */
export interface ActivatedModel {
  api_url: string; // 模型 API 端点地址
  api_key: string; // 访问模型所需的 API 密钥
  model_id: string; // 模型唯一标识符，例如 'gpt-4o', 'llama3'
  owned_by: string; // 模型提供商或厂商名称
  local_path?: string; // 本地模型的文件系统绝对路径，仅本地模型有效
  engine_type?: string; // 本地推理引擎类型标识，如 "llama_cpp", "vllm"
}

/** 项目接口 */
export interface Project {
  id: string;
  name: string;
  path: string;
  createdAt: string;
  updatedAt: string;
  /** 项目关联的助理 id（后端 list_projects 一并返回） */
  assistantId: string;
}
