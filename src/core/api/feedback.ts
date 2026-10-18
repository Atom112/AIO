// Feedback-domain Tauri command wrappers (counterpart of commands/feedback.rs).
// Convention: components must not invoke these command names directly; always go through this module.
import { invoke } from '@tauri-apps/api/core';

/** One harness tool's detection status in the snapshot. */
export interface FeedbackToolStatus {
  toolId: string;
  status: string;
}

/** One local engine's installation status. */
export interface FeedbackEngineInfo {
  engineType: string;
  version?: string;
}

/** Privacy-safe environment snapshot (no keys, no user data). */
export interface FeedbackSnapshot {
  appVersion: string;
  os: string;
  osVersion: string;
  arch: string;
  locale: string;
  harnessTools: FeedbackToolStatus[];
  localEngines: FeedbackEngineInfo[];
  ramGb: number;
  /** Single-line text the user pastes into the GitHub Issue Form. */
  summary: string;
}

/** Build the environment snapshot (privacy-safe: no API keys, no session content). */
export const getFeedbackSnapshot = (locale: string): Promise<FeedbackSnapshot> =>
  invoke<FeedbackSnapshot>('get_feedback_snapshot', { locale });

/**
 * Export a diagnostic zip (snapshot + config slot states) to the OS Downloads
 * directory and return the absolute path. The user attaches it to a GitHub
 * issue themselves — nothing is uploaded automatically.
 */
export const exportDiagnosticPackage = (locale: string): Promise<string> =>
  invoke<string>('export_diagnostic_package', { locale });
