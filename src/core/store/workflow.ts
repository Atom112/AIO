// Workflow visualization state (self-contained domain).

import { createSignal } from 'solid-js';

// ====== Workflow state (Workflow Visualization) ======

/** Workflow step state. */
export interface WorkflowStepState {
  stepId: string;
  profileId: string;
  name: string;
  status: 'pending' | 'running' | 'completed' | 'failed';
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
