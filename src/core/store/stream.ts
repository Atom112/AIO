/**
 * Global stream-state signals (CHAT-12).
 *
 * These signals previously lived in the ChatPage component scope together with the
 * event listeners that drive them. Leaving the chat page (e.g. switching to Settings
 * mid-generation) destroyed the listeners: the backend kept streaming but nothing
 * reached the store — the assistant message silently stopped updating and the final
 * `done` listener (which persists the assistant) never ran, losing the whole turn on
 * the next app start. Stream state must live for the whole app lifetime, so it is a
 * module-level singleton here and the stream listeners register from the root Layout.
 */

import { createSignal } from 'solid-js';

/** One pending tool-call approval pushed by the backend (tool-approval-requested). */
export interface PendingApproval {
  approvalId: string;
  serverId: string;
  toolName: string;
  // Tool arguments arrive as a raw JSON value from the backend payload; the any
  // budget stays capped by the repo-wide any-ratchet (scripts/check-gates.mjs).
  arguments: any;
  reason: string;
  /** File-change preview (unified diff) */
  previewDiff?: string;
  /** Path of the affected file */
  filePath?: string;
}

/** One option of an ask_user question. */
export interface QuestionOptionPayload {
  label: string;
  description?: string;
  recommended?: boolean;
}

/** One question of an ask_user request. */
export interface QuestionPayload {
  questionId: string;
  question: string;
  options: QuestionOptionPayload[];
  allowCustom: boolean;
}

/** One pending ask_user request. */
export interface PendingQuestion {
  requestId: string;
  questions: QuestionPayload[];
}

/** Whether the AI is thinking/generating (drives the loading animation and stop button). */
export const [isThinking, setIsThinking] = createSignal(false);

/** Index of the message currently shown with the typewriter effect; null = none. */
export const [typingIndex, setTypingIndex] = createSignal<number | null>(null);

/** Pending tool approvals awaiting the user's decision. */
export const [pendingApprovals, setPendingApprovals] = createSignal<PendingApproval[]>([]);

/** Pending ask_user requests awaiting the user's answers. */
export const [pendingQuestions, setPendingQuestions] = createSignal<PendingQuestion[]>([]);
