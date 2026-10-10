/**
 * LSP diagnostics state management
 *
 * Manages diagnostics from language servers (errors, warnings, hints),
 * grouped by file URI, with severity-level filtering and stats.
 */

import { createStore, produce } from 'solid-js/store';
import { createSignal } from 'solid-js';

// ====== Type definitions ======

export type DiagnosticSeverity = 'error' | 'warning' | 'info' | 'hint';

export interface DiagnosticRange {
  start: { line: number; character: number };
  end: { line: number; character: number };
}

export interface DiagnosticItem {
  range: DiagnosticRange;
  severity: DiagnosticSeverity;
  message: string;
  source?: string;
  code?: string;
}

export interface FileDiagnostics {
  /** Full file URI (file:///...) */
  uri: string;
  /** Path relative to the project root */
  filePath: string;
  /** Diagnostic items for this file */
  diagnostics: DiagnosticItem[];
}

export interface LspDiagnosticsUpdatePayload {
  languageId: string;
  params: {
    uri: string;
    diagnostics: DiagnosticItem[];
  };
}

// ====== State store ======

interface DiagnosticsState {
  /** key: file:// URI */
  diagnostics: Record<string, FileDiagnostics>;
  /** Language server that produced the last update */
  languageId: string;
}

const [diagnosticsState, setDiagnosticsState] = createStore<DiagnosticsState>({
  diagnostics: {},
  languageId: '',
});

// ====== Derived signals ======

/** Total error count */
export const [totalErrors, setTotalErrors] = createSignal(0);

/** Total warning count */
export const [totalWarnings, setTotalWarnings] = createSignal(0);

/** Whether the diagnostics panel is visible */
export const [problemsPanelVisible, setProblemsPanelVisible] = createSignal(false);

/** Current severity-level filter */
export const [severityFilter, setSeverityFilter] = createSignal<DiagnosticSeverity | 'all'>('all');

// ====== Operations ======

/**
 * Update diagnostics data from a language server
 */
export function updateDiagnostics(payload: LspDiagnosticsUpdatePayload) {
  const { uri, diagnostics } = payload.params;

  // An empty diagnostics array → clear that file's diagnostics
  if (!diagnostics || diagnostics.length === 0) {
    setDiagnosticsState(
      produce((s) => {
        delete s.diagnostics[uri];
      }),
    );
  } else {
    // Convert the URI to a relative path (extract the path part)
    const filePath = uriToFilePath(uri);

    setDiagnosticsState(
      produce((s) => {
        s.diagnostics[uri] = {
          uri,
          filePath,
          diagnostics,
        };
        s.languageId = payload.languageId;
      }),
    );
  }

  // Update stats
  recalculateStats();
}

/**
 * Clear all diagnostics
 */
export function clearAllDiagnostics() {
  setDiagnosticsState({ diagnostics: {}, languageId: '' });
  setTotalErrors(0);
  setTotalWarnings(0);
}

/**
 * Get all file diagnostics (with the filter applied)
 */
export function getFilteredDiagnostics(): FileDiagnostics[] {
  const filter = severityFilter();
  const files = Object.values(diagnosticsState.diagnostics);

  if (filter === 'all') {
    return files;
  }

  return files
    .map((f) => ({
      ...f,
      diagnostics: f.diagnostics.filter((d) => d.severity === filter),
    }))
    .filter((f) => f.diagnostics.length > 0);
}

/**
 * Whether any diagnostics exist
 */
export function hasDiagnostics(): boolean {
  return Object.keys(diagnosticsState.diagnostics).length > 0;
}

// ====== Internal helpers ======

function recalculateStats() {
  let errors = 0;
  let warnings = 0;

  for (const file of Object.values(diagnosticsState.diagnostics)) {
    for (const d of file.diagnostics) {
      if (d.severity === 'error') errors++;
      else if (d.severity === 'warning') warnings++;
    }
  }

  setTotalErrors(errors);
  setTotalWarnings(warnings);
}

/**
 * Convert a file:// URI to a readable relative path
 */
function uriToFilePath(uri: string): string {
  try {
    const url = new URL(uri);
    if (url.protocol === 'file:') {
      // Windows path: /C:/... → C:/...
      let path = decodeURIComponent(url.pathname);
      if (/^\/[a-zA-Z]:/.test(path)) {
        path = path.slice(1);
      }
      return path;
    }
  } catch {
    // Keep the original URI when it is not a valid URL.
  }
  return uri;
}
