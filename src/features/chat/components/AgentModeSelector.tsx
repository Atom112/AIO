/**
 * Agent mode quick selector
 *
 * Sits between the model selector and the reasoning button in the chat input; opens a pull-up menu to choose the working mode.
 * Modes: 只读 (read-only), 工作区内修改 (workspace edits), 完全权限 (full autonomy inherent), 计划 (plan first).
 * Switching to an Agent mode auto-restores the last working directory; if none exists, a "choose directory" button lets the user pick or create one.
 */
import { Component, For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import {
  datas,
  setDatas,
  currentAssistantId,
  saveSingleAssistantToBackend,
  currentProjectId,
  setCurrentProjectId,
  projects,
  initProjects,
  initSkills,
  initMcpServers,
  getLastAgentProjectId,
  saveLastAgentProjectId,
  type AgentMode,
} from '../../../core/store/store';
import Icon, { type IconName } from '../../../shared/components/Icon';
import { t, type TranslationKey } from '../../../core/i18n';
import { btnSecondary } from '../../../shared/components/buttonStyles';

interface ModeOption {
  value: AgentMode;
  labelKey: TranslationKey;
  descKey: TranslationKey;
  icon: IconName;
}

const MODES: ModeOption[] = [
  {
    value: 'off',
    labelKey: 'agent.mode.chat',
    descKey: 'agent.mode.chatDescription',
    icon: 'eye',
  },
  {
    value: 'normal',
    labelKey: 'agent.mode.normal',
    descKey: 'agent.mode.normalDescription',
    icon: 'wrench',
  },
  {
    value: 'auto',
    labelKey: 'agent.mode.auto',
    descKey: 'agent.mode.autoDescription',
    icon: 'zap',
  },
  {
    value: 'plan',
    labelKey: 'agent.mode.plan',
    descKey: 'agent.mode.planDescription',
    icon: 'document',
  },
];

const MODE_COLORS: Record<string, string> = {
  off: 'rgba(var(--primary-rgb),0.4)',
  normal: 'rgba(124,217,160,0.7)',
  auto: 'rgba(224,192,96,0.7)',
  plan: 'rgba(160,124,217,0.7)',
};

const AgentModeSelector: Component = () => {
  const [open, setOpen] = createSignal(false);
  const [showAutoWarning, setShowAutoWarning] = createSignal(false);
  let containerRef: HTMLDivElement | undefined;
  let autoModeConfirmed = false; // remembered for the session; after one confirmation the dialog no longer shows
  let pendingMode: AgentMode = 'auto'; // mode currently awaiting confirmation

  const asst = () => datas.assistants.find((a) => a.id === currentAssistantId()) as any;
  const currentMode = (): AgentMode => asst()?.agentMode || 'off';

  const activeOption = () => MODES.find((m) => m.value === currentMode()) || MODES[0];

  /** Close on outside click */
  const onDocClick = (e: MouseEvent) => {
    if (!containerRef) return;
    if (!containerRef.contains(e.target as Node)) {
      setOpen(false);
    }
  };

  onMount(() => document.addEventListener('mousedown', onDocClick));
  onCleanup(() => document.removeEventListener('mousedown', onDocClick));
  const choose = async (mode: AgentMode) => {
    // Switching to 完全权限 (auto = full autonomy inherent) shows the risk dialog first
    if (mode === 'auto' && !autoModeConfirmed) {
      pendingMode = mode;
      setOpen(false);
      setShowAutoWarning(true);
      return;
    }

    const id = currentAssistantId();
    if (!id) return;
    setDatas('assistants', (a) => a.id === id, 'agentMode', mode);
    await saveSingleAssistantToBackend(id);
    setOpen(false);

    // Auto-restore the last working directory when switching to an Agent mode
    if (mode !== 'off' && !currentProjectId()) {
      await initProjects();
      const lastId = getLastAgentProjectId();
      if (lastId && projects().find((p) => p.id === lastId)) {
        // A last working directory exists and is still valid -> restore it directly
        setCurrentProjectId(lastId);
        saveLastAgentProjectId(lastId);
        await Promise.all([initSkills(lastId), initMcpServers(lastId)]);
      }
      // No last working directory -> do nothing (the project panel lives in ProjectSidebar on the left for the user to pick or create one)
    } else if (mode !== 'off' && currentProjectId()) {
      // A working directory already exists; refresh and save it as lastAgentProjectId
      saveLastAgentProjectId(currentProjectId());
    }

    // Clear lastAgentProjectId when switching to chat mode (so switching back to Agent does not auto-restore)
    if (mode === 'off') {
      saveLastAgentProjectId(null);
    }
  };

  const isDisabled = () => !currentAssistantId();

  return (
    <div ref={containerRef} class="relative inline-block">
      <button
        type="button"
        class="flex items-center gap-1.5 px-2.5 h-8 rounded-md border-none cursor-pointer transition-all duration-200 select-none bg-transparent text-white/40 text-xs font-medium hover:bg-white/[0.06] hover:text-[var(--color-pri)]/60"
        classList={{ 'is-active': currentMode() !== 'off' }}
        style={{
          color:
            currentMode() !== 'off' ? MODE_COLORS[currentMode()] || MODE_COLORS.off : undefined,
          opacity: isDisabled() ? 0.4 : 1,
        }}
        title={isDisabled() ? t('agent.mode.chooseAssistant') : t(activeOption().descKey)}
        onClick={(e) => {
          if (isDisabled()) return;
          e.stopPropagation();
          setOpen(!open());
        }}
      >
        <Icon
          name={activeOption().icon}
          size={15}
          class="flex items-center justify-center shrink-0"
        />
        <span class="leading-none">{t(activeOption().labelKey)}</span>
      </button>

      {/* Dropdown panel */}
      <div
        class="absolute bottom-full left-0 mb-2 z-[41] w-[190px] rounded-xl overflow-hidden transition-all duration-150 ease-out origin-bottom"
        style={{
          background: 'var(--popup-bg)',
          border: '1px solid var(--border-dim)',
          'backdrop-filter': 'var(--popup-blur)',
          'box-shadow': '0 -8px 30px rgba(0,0,0,0.4)',
        }}
        classList={{
          'invisible opacity-0 scale-95 -translate-y-1 pointer-events-none': !open(),
          'visible opacity-100 scale-100 translate-y-0 pointer-events-auto': open(),
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest"
          style={{
            color: 'rgba(var(--text-base-rgb),0.35)',
            background: 'rgba(var(--text-base-rgb),0.04)',
            'border-bottom': '1px solid var(--border-dim)',
          }}
        >
          {t('agent.mode.title')}
        </div>
        <div class="py-1">
          <For each={MODES}>
            {(opt) => (
              <button
                type="button"
                class="w-full flex items-center gap-2.5 px-3 py-2 text-left transition-colors cursor-pointer border-none"
                style={{ color: 'rgba(var(--text-base-rgb),0.75)' }}
                classList={{ '!bg-[rgba(var(--primary-rgb),0.12)]': currentMode() === opt.value }}
                onClick={() => choose(opt.value)}
                onMouseEnter={(e) => {
                  if (currentMode() !== opt.value)
                    e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.05)';
                }}
                onMouseLeave={(e) => {
                  if (currentMode() !== opt.value) e.currentTarget.style.background = 'transparent';
                }}
              >
                <Icon
                  name={opt.icon}
                  size={14}
                  class="shrink-0"
                  style={{ color: MODE_COLORS[opt.value] }}
                />
                <div class="flex-1 min-w-0 text-sm font-medium">{t(opt.labelKey)}</div>
                <Show when={currentMode() === opt.value}>
                  <Icon
                    name="check"
                    size={13}
                    class="shrink-0"
                    style={{ color: 'rgba(var(--primary-rgb),0.8)' }}
                  />
                </Show>
              </button>
            )}
          </For>
        </div>
      </div>

      {/* Auto mode risk warning dialog */}
      <Show when={showAutoWarning()}>
        <div
          class="fixed inset-0 z-[2000] flex items-center justify-center bg-black/50 backdrop-blur-[12px]"
          style={{ 'z-index': '2100', animation: 'modalOverlayIn 0.2s ease-out both' }}
          onClick={() => setShowAutoWarning(false)}
        >
          <div
            class="p-6 rounded-lg max-w-[420px] w-full"
            style={{
              background: 'var(--dialog-bg)',
              border: '1px solid var(--border-dim)',
              'backdrop-filter': 'var(--popup-blur)',
              'box-shadow': '0 8px 32px rgba(0,0,0,0.5)',
              animation: 'modalPanelIn 0.25s cubic-bezier(0.16, 1, 0.3, 1) both',
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <div class="flex items-center gap-3 mb-4">
              <Icon name="alert-triangle" size={24} />
              <h2
                style={{
                  color: 'rgba(224,192,96,0.9)',
                  'font-size': '1.1rem',
                  'font-weight': '600',
                  margin: '0',
                }}
              >
                {t('agent.mode.warningTitle')}
              </h2>
            </div>

            <div
              style={{
                color: 'rgba(var(--text-base-rgb),0.7)',
                'font-size': '0.875rem',
                'line-height': '1.7',
                'margin-bottom': '1.5rem',
              }}
            >
              <p style={{ margin: '0 0 0.75rem 0' }}>{t('agent.mode.warningPrimary')}</p>
              <p style={{ margin: '0 0 0.75rem 0' }}>{t('agent.mode.warningDetails')}</p>
              <p style={{ margin: '0' }}>{t('agent.mode.warningTrust')}</p>
            </div>

            <div class="flex justify-end gap-3">
              <button type="button" class={btnSecondary} onClick={() => setShowAutoWarning(false)}>
                {t('common.cancel')}
              </button>
              <button
                type="button"
                class="px-4 py-2 rounded-lg text-sm font-medium transition-all duration-200 cursor-pointer border-solid"
                style={{
                  background: 'rgba(224,192,96,0.1)',
                  color: 'rgba(224,192,96,0.95)',
                  border: '1.5px solid rgba(224,192,96,0.5)',
                }}
                onClick={async () => {
                  autoModeConfirmed = true;
                  setShowAutoWarning(false);
                  // Perform the actual mode switch
                  await choose(pendingMode);
                }}
              >
                {t('agent.mode.warningConfirm', { mode: t('agent.mode.auto') })}
              </button>
            </div>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default AgentModeSelector;
