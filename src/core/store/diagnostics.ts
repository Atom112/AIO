/**
 * LSP 诊断状态管理
 *
 * 管理来自语言服务器的诊断信息（错误、警告、提示），
 * 按文件 URI 分组存储，支持按严重级别过滤和统计。
 */

import { createStore, produce } from 'solid-js/store';
import { createSignal } from 'solid-js';

// ====== 类型定义 ======

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
  /** 文件完整 URI (file:///...) */
  uri: string;
  /** 相对于项目根目录的路径 */
  filePath: string;
  /** 该文件的诊断项列表 */
  diagnostics: DiagnosticItem[];
}

export interface LspDiagnosticsUpdatePayload {
  languageId: string;
  params: {
    uri: string;
    diagnostics: DiagnosticItem[];
  };
}

// ====== 状态 Store ======

interface DiagnosticsState {
  /** key: file:// URI */
  diagnostics: Record<string, FileDiagnostics>;
  /** 最后更新时对应的语言服务器 */
  languageId: string;
}

const [diagnosticsState, setDiagnosticsState] = createStore<DiagnosticsState>({
  diagnostics: {},
  languageId: '',
});

// ====== 派生信号 ======

/** 错误总数 */
export const [totalErrors, setTotalErrors] = createSignal(0);

/** 警告总数 */
export const [totalWarnings, setTotalWarnings] = createSignal(0);

/** 诊断面板是否可见 */
export const [problemsPanelVisible, setProblemsPanelVisible] = createSignal(false);

/** 当前严重级别过滤器 */
export const [severityFilter, setSeverityFilter] = createSignal<DiagnosticSeverity | 'all'>('all');

// ====== 操作函数 ======

/**
 * 更新来自语言服务器的诊断数据
 */
export function updateDiagnostics(payload: LspDiagnosticsUpdatePayload) {
  const { uri, diagnostics } = payload.params;

  // 如果诊断为空的数组 → 清除该文件的诊断
  if (!diagnostics || diagnostics.length === 0) {
    setDiagnosticsState(
      produce((s) => {
        delete s.diagnostics[uri];
      }),
    );
  } else {
    // 将 URI 转为相对路径（提取 path 部分）
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

  // 更新统计
  recalculateStats();
}

/**
 * 清除所有诊断
 */
export function clearAllDiagnostics() {
  setDiagnosticsState({ diagnostics: {}, languageId: '' });
  setTotalErrors(0);
  setTotalWarnings(0);
}

/**
 * 清除指定文件的诊断
 */
export function clearFileDiagnostics(uri: string) {
  setDiagnosticsState(
    produce((s) => {
      delete s.diagnostics[uri];
    }),
  );
  recalculateStats();
}

/**
 * 获取所有文件诊断（应用过滤器）
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
 * 获取指定文件的最严重级别（用于文件树标记）
 */
export function getFileMaxSeverity(uri: string): DiagnosticSeverity | null {
  const file = diagnosticsState.diagnostics[uri];
  if (!file || file.diagnostics.length === 0) return null;

  const severities = file.diagnostics.map((d) => d.severity);
  if (severities.includes('error')) return 'error';
  if (severities.includes('warning')) return 'warning';
  if (severities.includes('info')) return 'info';
  return 'hint';
}

/**
 * 是否有任何诊断
 */
export function hasDiagnostics(): boolean {
  return Object.keys(diagnosticsState.diagnostics).length > 0;
}

// ====== 内部辅助 ======

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
 * 将 file:// URI 转为可读的相对路径
 */
function uriToFilePath(uri: string): string {
  try {
    const url = new URL(uri);
    if (url.protocol === 'file:') {
      // Windows 路径: /C:/... → C:/...
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
