// Subagent standalone conversation view state (self-contained domain).
//
// The main agent timeline no longer embeds subagent work details; instead:
// - a pull-up menu at the right of the status bar lists all subagents of the current topic (data persisted with message agentSteps);
// - activeSubagentView indicates the subagent currently being viewed (null = main agent conversation view).

import { createSignal } from 'solid-js';
import type { AgentStep, Topic } from './types';

/** Subagent ID currently being viewed (matches AgentStep.subagentId); null means the main conversation view. */
export const [activeSubagentView, setActiveSubagentView] = createSignal<string | null>(null);

/**
 * Collect all subagent steps in the current topic (including finished past rounds).
 * Walks the agentSteps of assistant messages and returns type='subagent' steps in time order.
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

/** Find a subagent step in the current topic by subagentId (undefined if not found). */
export function findSubagentStep(
  topic: Topic | null | undefined,
  subagentId: string | null | undefined,
): AgentStep | undefined {
  if (!subagentId) return undefined;
  return collectSubagents(topic).find((s) => s.subagentId === subagentId);
}
