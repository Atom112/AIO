// 聊天域 Tauri 命令封装（与 commands/messages.rs、commands/llm/mod.rs 对应）。
// 约定：组件不得直接 invoke 这些命令名，一律走本模块。
import { invoke } from '@tauri-apps/api/core';
import type { AgentMode, Assistant, Message, Topic } from '../store/types';
import type { ProfileModelOverrideBackend, CustomSubagentProfile } from '../store/profiles';
import type { SkillConfig } from '../types/skill';

/** LLM 连接凭证（apiUrl/apiKey/model）。 */
export interface LlmCredentials {
  apiUrl: string;
  apiKey: string;
  model: string;
}

/** 从 ActivatedModel 构造 LLM 凭证（组件层惯例转换，避免各处重复拼装）。 */
export const llmCreds = (m: {
  api_url: string;
  api_key?: string | null;
  model_id: string;
}): LlmCredentials => ({
  apiUrl: m.api_url,
  apiKey: m.api_key ?? '',
  model: m.model_id,
});

/** 追加一条消息到话题（持久化）。 */
export const appendMessage = (topicId: string, message: Message) =>
  invoke<void>('append_message', { topicId, message });

/** 删除话题中的一条消息。 */
export const deleteTopicMessage = (topicId: string, messageId: string) =>
  invoke<void>('delete_topic_message', { topicId, messageId });

/** 从一条消息分叉出新话题。 */
export const branchTopic = (sourceTopicId: string, sourceMessageId: string) =>
  invoke<Topic>('branch_topic', { sourceTopicId, sourceMessageId });

/** 加载全部助手（含话题与消息树）。 */
export const loadAssistants = () => invoke<Assistant[]>('load_assistants');

/** 保存（新建或更新）一个助手。 */
export const saveAssistant = (assistant: Assistant) =>
  invoke<void>('save_assistant', { assistant: assistant as unknown as Record<string, unknown> });

/** 删除一个助手。 */
export const deleteAssistant = (id: string) => invoke<void>('delete_assistant', { id });

/** 压缩/总结历史（覆盖式摘要），返回新摘要文本。 */
export const summarizeHistory = (creds: LlmCredentials, messagesJson: string) =>
  invoke<string>('summarize_history', { ...creds, messagesJson });

/** 为话题生成标题，返回标题文本。 */
export const generateTopicTitle = (creds: LlmCredentials, messagesJson: string) =>
  invoke<string>('generate_topic_title', { ...creds, messagesJson });

/** BTW 旁路问答（流式，经 btw-chunk 事件回报到指定 overlay）。 */
export const askBtwQuestion = (creds: LlmCredentials, overlayId: string, messagesJson: string) =>
  invoke<void>('ask_btw_question', { ...creds, overlayId, messagesJson });

/** 生成图像（经 llm-chunk/事件回报进度）。 */
export const generateImage = (
  creds: LlmCredentials,
  assistantId: string,
  topicId: string,
  prompt: string,
) => invoke<void>('generate_image', { ...creds, assistantId, topicId, prompt });

/** 对话模式流式调用（聊天，无工具）。 */
export const callLlmStream = (
  creds: LlmCredentials,
  assistantId: string,
  topicId: string,
  messages: Message[],
) => invoke<void>('call_llm_stream', { ...creds, assistantId, topicId, messages });

/** run_agent_turn 参数（Agent 一轮自驱循环）。 */
export interface RunAgentTurnArgs extends LlmCredentials {
  assistantId: string;
  topicId: string;
  messages: Message[];
  mcpServerIds: string[];
  agentMode: AgentMode;
  projectId: string | null;
  webSearchEnabled: boolean;
  profileModelOverrides: ProfileModelOverrideBackend[];
  customSubagentProfiles: CustomSubagentProfile[];
  locale: string;
  contextWindow: number;
  skills?: SkillConfig[];
  /** P4.4：恢复指定回合的检查点（崩溃后续跑）；缺省 = 正常新回合 */
  resumeTurnId?: string;
}

/** 启动 Agent 一轮自驱循环（进度经 llm-* 系列事件回报）。 */
export const runAgentTurn = (args: RunAgentTurnArgs) =>
  invoke<void>('run_agent_turn', args as unknown as Record<string, unknown>);

/** 停止某话题的 LLM 流式任务。 */
export const stopLlmStream = (assistantId: string, topicId: string) =>
  invoke<void>('stop_llm_stream', { assistantId, topicId });

/** 停止后台子智能体（fire-and-forget）。返回是否找到并发出取消信号（false = 已完成/不存在，幂等）。 */
export const stopSubagent = (subagentId: string) =>
  invoke<boolean>('stop_subagent', { subagentId });

/** 活跃检查点信息（崩溃残留的回合，可恢复续跑）。 */
export interface ActiveCheckpoint {
  turnId: string;
  topicId: string;
  round: number;
  createdAt: number;
}

/** 查询话题的活跃检查点（P4.4：崩溃后"继续未完成的回合"入口）。 */
export const getActiveCheckpoint = (topicId: string) =>
  invoke<ActiveCheckpoint | null>('get_active_checkpoint', { topicId });
