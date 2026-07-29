/**
 * Agent 模式快速选择器
 *
 * 位于聊天输入框模型选择器与推理按钮之间，点击弹出上拉菜单选择工作模式。
 * 四种模式：对话（关闭工具调用）、普通（写入确认）、自动（自主执行）、计划（先列计划）。
 * 切换到 Agent 模式时自动恢复上次的工作目录，若无则显示"选择目录"按钮供用户自行选择或新建。
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
    icon: 'chat',
  },
  {
    value: 'normal',
    labelKey: 'agent.mode.normal',
    descKey: 'agent.mode.normalDescription',
    icon: 'check-circle',
  },
  {
    value: 'auto',
    labelKey: 'agent.mode.auto',
    descKey: 'agent.mode.autoDescription',
    icon: 'refresh',
  },
  {
    value: 'plan',
    labelKey: 'agent.mode.plan',
    descKey: 'agent.mode.planDescription',
    icon: 'document',
  },
  {
    value: 'workflow',
    labelKey: 'agent.mode.workflow',
    descKey: 'agent.mode.workflowDescription',
    icon: 'layers',
  },
];

const MODE_COLORS: Record<string, string> = {
  off: 'rgba(var(--primary-rgb),0.4)',
  normal: 'rgba(124,217,160,0.7)',
  auto: 'rgba(224,192,96,0.7)',
  plan: 'rgba(160,124,217,0.7)',
  workflow: 'rgba(255,183,77,0.7)',
};

const AgentModeSelector: Component = () => {
  const [open, setOpen] = createSignal(false);
  const [showAutoWarning, setShowAutoWarning] = createSignal(false);
  let containerRef: HTMLDivElement | undefined;
  let autoModeConfirmed = false; // 会话内记住，确认一次后不再弹窗
  let pendingMode: AgentMode = 'auto'; // 当前正在确认的模式

  const asst = () => datas.assistants.find((a) => a.id === currentAssistantId()) as any;
  const currentMode = (): AgentMode => asst()?.agentMode || 'off';

  const activeOption = () => MODES.find((m) => m.value === currentMode()) || MODES[0];

  /** 点击外部关闭 */
  const onDocClick = (e: MouseEvent) => {
    if (!containerRef) return;
    if (!containerRef.contains(e.target as Node)) {
      setOpen(false);
    }
  };

  onMount(() => document.addEventListener('mousedown', onDocClick));
  onCleanup(() => document.removeEventListener('mousedown', onDocClick));
  const choose = async (mode: AgentMode) => {
    // 切换到自动模式或工作流模式时，先展示风险提醒弹窗
    if ((mode === 'auto' || mode === 'workflow') && !autoModeConfirmed) {
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

    // 切换到 Agent 模式时自动恢复上次的工作目录
    if (mode !== 'off' && !currentProjectId()) {
      await initProjects();
      const lastId = getLastAgentProjectId();
      if (lastId && projects().find((p) => p.id === lastId)) {
        // 有上次的工作目录且仍有效 → 直接恢复
        setCurrentProjectId(lastId);
        saveLastAgentProjectId(lastId);
        await Promise.all([initSkills(lastId), initMcpServers(lastId)]);
      }
      // 没有上次的工作目录→ 什么也不做，ProjectSelector 会显示"选择目录..."供用户自行选择或新建
    } else if (mode !== 'off' && currentProjectId()) {
      // 已有工作目录，刷新保存为 lastAgentProjectId
      saveLastAgentProjectId(currentProjectId());
    }

    // 切换到对话模式时清除 lastAgentProjectId（下次切回 Agent 时不自动恢复）
    if (mode === 'off') {
      saveLastAgentProjectId(null);
    }
  };

  const isDisabled = () => !currentAssistantId();

  return (
    <div ref={containerRef} class="relative inline-block">
      <button
        type="button"
        class="flex items-center gap-1.5 px-2.5 h-8 rounded-md border-none cursor-pointer transition-all duration-200 select-none bg-transparent text-white/40 text-xs font-medium hover:bg-white/[0.06] hover:text-[#7c9abf]/60"
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

      {/* 下拉面板 */}
      <div
        class="absolute bottom-full left-0 mb-2 z-[41] w-[260px] rounded-xl overflow-hidden transition-all duration-150 ease-out origin-bottom"
        style={{
          background: 'rgba(var(--surface-bg),0.96)',
          border: '1px solid var(--border-dim)',
          'backdrop-filter': 'blur(10px)',
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
                class="w-full flex items-start gap-2.5 px-3 py-2.5 text-left transition-colors cursor-pointer border-none"
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
                  class="mt-0.5 shrink-0"
                  style={{ color: MODE_COLORS[opt.value] }}
                />
                <div class="flex-1 min-w-0">
                  <div class="text-sm font-medium">{t(opt.labelKey)}</div>
                  <div
                    class="text-[11px] mt-0.5"
                    style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}
                  >
                    {t(opt.descKey)}
                  </div>
                </div>
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

      {/* 自动模式风险提醒弹窗 */}
      <Show when={showAutoWarning()}>
        <div
          class="fixed inset-0 z-[2000] flex items-center justify-center bg-black/50 backdrop-blur-[12px]"
          style={{ 'z-index': '2100', animation: 'modalOverlayIn 0.2s ease-out both' }}
          onClick={() => setShowAutoWarning(false)}
        >
          <div
            class="bg-dark-500 p-6 rounded-lg max-w-[420px] w-full"
            style={{
              background: 'rgba(var(--surface-bg),0.96)',
              border: '1px solid var(--border-dim)',
              'backdrop-filter': 'blur(12px)',
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
              <button
                type="button"
                class="px-4 py-2 rounded-lg text-sm font-medium transition-all duration-200 cursor-pointer border-none"
                style={{
                  background: 'rgba(var(--text-base-rgb),0.06)',
                  color: 'rgba(var(--text-base-rgb),0.5)',
                }}
                onClick={() => setShowAutoWarning(false)}
              >
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
                  // 执行实际的模式切换
                  await choose(pendingMode);
                }}
              >
                {t('agent.mode.warningConfirm', {
                  mode: t(
                    (pendingMode as AgentMode) === 'workflow'
                      ? 'agent.mode.workflow'
                      : 'agent.mode.auto',
                  ),
                })}
              </button>
            </div>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default AgentModeSelector;
