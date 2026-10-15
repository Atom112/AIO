// Agent task-board state (main-agent task list; self-contained domain).
//
// The board is mirrored from the backend's create_tasks / update_task tools via the
// agent-tasks-changed event (see chatEventListeners); the TaskBoardPanel renders it
// inside the agent status bar.

import { createSignal } from 'solid-js';

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
