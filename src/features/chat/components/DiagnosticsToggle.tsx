/**
 * LSP problems-panel toggle (阶段4 第2项拆分切片：ChatInterface <900 目标).
 *
 * Verbatim extraction of the diagnostics toolbar button from ChatInterface:
 * visible only when the LSP has diagnostics for the current file; shows the
 * error count badge and toggles the problems panel. All state (diagnostics,
 * panel visibility) lives in the module-level diagnostics store.
 */
import { Show, type Component } from 'solid-js';
import { formatNumber, t } from '../../../core/i18n';
import {
  hasDiagnostics,
  problemsPanelVisible,
  setProblemsPanelVisible,
  totalErrors,
  totalWarnings,
} from '../../../core/store/diagnostics';
import { btnIcon } from '../../../shared/components/buttonStyles';
import Icon from '../../../shared/components/Icon';

const DiagnosticsToggle: Component = () => {
  return (
    <Show when={hasDiagnostics()}>
      <button
        class={'relative p-1.5 ' + btnIcon}
        style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
        title={t('chat.diagnostics', {
          errors: formatNumber(totalErrors()),
          warnings: formatNumber(totalWarnings()),
        })}
        onClick={() => setProblemsPanelVisible(!problemsPanelVisible())}
        onMouseEnter={(e) => {
          e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.06)';
          e.currentTarget.style.color = 'rgba(var(--text-base-rgb),0.6)';
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.background = 'transparent';
          e.currentTarget.style.color = 'rgba(var(--text-base-rgb),0.4)';
        }}
      >
        <Icon name="clipboard" size={14} />
        <Show when={totalErrors() > 0}>
          <span class="absolute -top-1 -right-1 text-[9px] px-1 rounded-full bg-red-500 text-white font-bold leading-tight">
            {totalErrors()}
          </span>
        </Show>
      </button>
    </Show>
  );
};

export default DiagnosticsToggle;
