/**
 * ProblemsPanel — LSP 诊断面板组件
 *
 * 显示语言服务器报告的错误、警告、信息和提示。
 * 位于聊天页面的底部面板或侧边栏区域。
 */

import { Component, For, Show, createMemo } from 'solid-js';
import {
  getFilteredDiagnostics,
  totalErrors,
  totalWarnings,
  severityFilter,
  setSeverityFilter,
  problemsPanelVisible,
  setProblemsPanelVisible,
  hasDiagnostics,
} from '../../../core/store/diagnostics';
import { invoke } from '@tauri-apps/api/core';

import Icon, { type IconName } from '../../../shared/components/Icon';
import { formatNumber, t, type TranslationKey } from '../../../core/i18n';

/** 诊断严重级别对应的图标和颜色 */
const SEVERITY_CONFIG: Record<string, { icon: IconName; color: string; bgColor: string }> = {
  error: { icon: 'x-circle', color: 'text-red-500', bgColor: 'bg-red-500/10' },
  warning: { icon: 'alert-triangle', color: 'text-yellow-500', bgColor: 'bg-yellow-500/10' },
  info: { icon: 'info', color: 'text-blue-500', bgColor: 'bg-blue-500/10' },
  hint: { icon: 'lightbulb', color: 'text-gray-400', bgColor: 'bg-gray-500/10' },
};

/** 过滤器选项 */
const FILTER_OPTIONS: { value: string; labelKey: TranslationKey }[] = [
  { value: 'all', labelKey: 'chat.problemAll' },
  { value: 'error', labelKey: 'common.error' },
  { value: 'warning', labelKey: 'chat.problemWarning' },
  { value: 'info', labelKey: 'chat.problemInfo' },
  { value: 'hint', labelKey: 'chat.problemHint' },
];

/**
 * 尝试使用系统应用打开指定文件
 */
async function openFileInEditor(filePath: string) {
  try {
    await invoke('open_project_directory', { path: filePath });
  } catch {
    // 静默失败
  }
}

const ProblemsPanel: Component = () => {
  const filteredFiles = createMemo(() => getFilteredDiagnostics());
  const activeFilter = createMemo(() => severityFilter());

  return (
    <Show when={problemsPanelVisible()}>
      <div class="border-t border-gray-700/50 bg-gray-900/80 backdrop-blur-sm">
        {/* 标题栏 */}
        <div class="flex items-center justify-between px-4 py-2 border-b border-gray-700/30">
          <div class="flex items-center gap-3">
            <span class="text-sm font-medium text-gray-200 inline-flex items-center gap-1">
              <Icon name="clipboard" size={14} />
              {t('chat.problems')}
            </span>
            <Show when={totalErrors() > 0}>
              <span class="text-xs px-2 py-0.5 rounded-full bg-red-500/20 text-red-400 font-medium">
                {t('chat.errorsCount', { count: formatNumber(totalErrors()) })}
              </span>
            </Show>
            <Show when={totalWarnings() > 0}>
              <span class="text-xs px-2 py-0.5 rounded-full bg-yellow-500/20 text-yellow-400 font-medium">
                {t('chat.warningsCount', { count: formatNumber(totalWarnings()) })}
              </span>
            </Show>
          </div>

          <div class="flex items-center gap-2">
            {/* 过滤器 */}
            <div class="flex rounded-md overflow-hidden border border-gray-600/50">
              <For each={FILTER_OPTIONS}>
                {(opt) => (
                  <button
                    class={`text-xs px-2 py-1 transition-colors ${
                      activeFilter() === opt.value
                        ? 'bg-blue-600/40 text-blue-300'
                        : 'text-gray-400 hover:text-gray-200 hover:bg-gray-700/50'
                    }`}
                    onClick={() => setSeverityFilter(opt.value as any)}
                  >
                    {t(opt.labelKey)}
                  </button>
                )}
              </For>
            </div>

            {/* 关闭按钮 */}
            <button
              class="text-gray-400 hover:text-gray-200 p-1 rounded hover:bg-gray-700/50"
              onClick={() => setProblemsPanelVisible(false)}
              title={t('chat.closePanel')}
            >
              ✕
            </button>
          </div>
        </div>

        {/* 诊断列表 */}
        <div class="max-h-64 overflow-y-auto">
          <Show
            when={filteredFiles().length > 0}
            fallback={
              <div class="flex items-center justify-center py-8 text-gray-500 text-sm">
                <Show when={hasDiagnostics()} fallback={`✓ ${t('chat.noProblems')}`}>
                  {t('chat.noMatchingDiagnostics')}
                </Show>
              </div>
            }
          >
            <For each={filteredFiles()}>
              {(file) => (
                <div class="border-b border-gray-700/20 last:border-b-0">
                  {/* 文件名标题 */}
                  <button
                    class="w-full flex items-center gap-2 px-4 py-1.5 text-xs font-mono text-gray-300 hover:bg-gray-800/50 transition-colors cursor-pointer"
                    onClick={() => openFileInEditor(file.filePath)}
                    title={t('chat.openFile', { path: file.filePath })}
                  >
                    <Icon name="file" size={14} />
                    <span class="truncate">{file.filePath}</span>
                    <span class="text-gray-500 ml-auto">{file.diagnostics.length}</span>
                  </button>

                  {/* 诊断项列表 */}
                  <div>
                    <For each={file.diagnostics}>
                      {(diag) => {
                        const sev = SEVERITY_CONFIG[diag.severity] || SEVERITY_CONFIG.error;
                        const lineNum = diag.range.start.line + 1; // LSP 是 0-based，显示 1-based
                        const colNum = diag.range.start.character + 1;

                        return (
                          <div
                            class={`flex items-start gap-2 px-4 py-1.5 pl-8 ${sev.bgColor} border-l-2 ${
                              diag.severity === 'error'
                                ? 'border-l-red-500'
                                : diag.severity === 'warning'
                                  ? 'border-l-yellow-500'
                                  : diag.severity === 'info'
                                    ? 'border-l-blue-500'
                                    : 'border-l-gray-500'
                            }`}
                          >
                            <Icon name={sev.icon} size={14} class="text-xs mt-0.5 shrink-0" />
                            <div class="flex-1 min-w-0">
                              <span class="text-xs text-gray-300 break-all">{diag.message}</span>
                              <div class="flex items-center gap-2 mt-0.5">
                                <span class="text-[10px] text-gray-500 font-mono">
                                  Ln {lineNum}, Col {colNum}
                                </span>
                                <Show when={diag.source}>
                                  <span class="text-[10px] text-gray-600">{diag.source}</span>
                                </Show>
                                <Show when={diag.code}>
                                  <span class="text-[10px] text-gray-600 font-mono">
                                    {diag.code}
                                  </span>
                                </Show>
                              </div>
                            </div>
                          </div>
                        );
                      }}
                    </For>
                  </div>
                </div>
              )}
            </For>
          </Show>
        </div>
      </div>
    </Show>
  );
};

export default ProblemsPanel;
