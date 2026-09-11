// 工作流可视化状态 —— 从 store.ts 拆出（自包含域）。

import { createSignal } from 'solid-js';

// ====== 工作流状态（Workflow Visualization） ======

/** 工作流步骤状态 */
export interface WorkflowStepState {
  stepId: string;
  profileId: string;
  name: string;
  status: 'pending' | 'running' | 'completed' | 'failed';
  startedAt?: number;
  duration?: number;
}

/** 工作流整体状态 */
export interface WorkflowState {
  workflowId: string;
  title: string;
  steps: WorkflowStepState[];
  active: boolean;
}

/** 当前活跃的工作流状态，null 表示无活跃工作流 */
export const [workflowState, setWorkflowState] = createSignal<WorkflowState | null>(null);
