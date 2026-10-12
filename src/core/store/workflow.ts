// Workflow visualization state (self-contained domain).

import { createSignal } from 'solid-js';

// ====== Workflow state (Workflow Visualization) ======

/** Workflow step state. */
export interface WorkflowStepState {
  stepId: string;
  profileId: string;
  name: string;
  status: 'pending' | 'running' | 'completed' | 'failed' | 'skipped';
  startedAt?: number;
  duration?: number;
}

/** Overall workflow state. */
export interface WorkflowState {
  workflowId: string;
  title: string;
  steps: WorkflowStepState[];
  active: boolean;
}

/** Currently active workflow state; null means no active workflow. */
export const [workflowState, setWorkflowState] = createSignal<WorkflowState | null>(null);

// ====== Agent task board (main-agent task list) ======

/** One tracked task on the agent task board (snake_case payload from the backend). */
export interface AgentTaskState {
  id: string;
  title: string;
  status: 'pending' | 'in_progress' | 'completed';
}

/** Task board snapshot for one (assistant, topic); mirrored via agent-tasks-changed. */
export interface AgentTaskBoardState {
  assistantId: string;
  topicId: string;
  tasks: AgentTaskState[];
}

/** Latest task board snapshot; null means no board yet. */
export const [agentTaskBoard, setAgentTaskBoard] = createSignal<AgentTaskBoardState | null>(null);
