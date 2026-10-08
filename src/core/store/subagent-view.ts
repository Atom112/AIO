// 子智能体独立对话视图状态 —— 自包含域（自 subagent 工作流 UI 重构引入）。
//
// 主代理时间线不再内嵌子智能体的工作细节；改为：
// - 状态条右侧上拉菜单列出当前话题的全部子代理（数据随消息 agentSteps 持久化）；
// - activeSubagentView 指示当前正在查看的子代理（null = 主代理对话视图）。

import { createSignal } from 'solid-js';
import type { AgentStep, Topic } from './types';

/** 当前查看的子智能体 ID（对应 AgentStep.subagentId），null 表示主对话视图 */
export const [activeSubagentView, setActiveSubagentView] = createSignal<string | null>(null);

/**
 * 收集当前话题中出现的全部子智能体步骤（含已完成的历史轮次）。
 * 遍历 assistant 消息的 agentSteps，按时间顺序返回 type='subagent' 的步骤。
 */
export function collectSubagents(topic: Topic | null | undefined): AgentStep[] {
  if (!topic || !Array.isArray(topic.history)) return [];
  const out: AgentStep[] = [];
  for (const msg of topic.history) {
    if (!msg || msg.role !== 'assistant') continue;
    const steps = (msg as { agentSteps?: AgentStep[] }).agentSteps;
    if (!Array.isArray(steps)) continue;
    for (const s of steps) {
      if (s && s.type === 'subagent' && s.subagentId) out.push(s);
    }
  }
  return out;
}

/** 按 subagentId 查找当前话题中的子智能体步骤（找不到返回 undefined） */
export function findSubagentStep(
  topic: Topic | null | undefined,
  subagentId: string | null | undefined,
): AgentStep | undefined {
  if (!subagentId) return undefined;
  return collectSubagents(topic).find((s) => s.subagentId === subagentId);
}
