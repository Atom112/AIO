/**
 * Quick model selector for the chat input (LobeHub-style)
 * @description Shows the current model on the left of the input toolbar; clicking opens an
 * upward-expanding dropdown listing all available models (cloud / local) for quick switching. Selection binds to the current assistant and persists.
 */
import { Component, createSignal, Show, For } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import { t } from '../../../core/i18n';
import {
  datas,
  currentAssistantId,
  allAvailableModels,
  isLocalModel,
  resolveAssistantModel,
  modelKey,
  setAssistantModel,
  ActivatedModel,
} from '../../../core/store/store';
import { getLogo as getLogoByIds } from '../../../core/utils/modelLogo';

const getModelLogo = (name: string) => getLogoByIds(null, name);

const ModelSelector: Component = () => {
  const [open, setOpen] = createSignal(false);

  /** Current assistant object (used to resolve its bound active model) */
  const currentAssistant = () => datas.assistants.find((a) => a.id === currentAssistantId()) as any;

  /** Active model (resolves via the assistant's bound modelId first, falls back to the global selectedModel when unbound) */
  const activeModel = () => resolveAssistantModel(currentAssistant());

  const cloudModels = () => allAvailableModels().filter((m) => !isLocalModel(m));
  const localModels = () => allAvailableModels().filter((m) => isLocalModel(m));

  /** Whether a model is the current selection (exact match on the composite key) */
  const isCurrent = (m: ActivatedModel): boolean => {
    const cur = activeModel();
    return cur ? modelKey(m) === modelKey(cur) : false;
  };

  /** Select a model: bind to the current assistant + sync global + persist (local models also start their engine) */
  const handlePick = (m: ActivatedModel) => {
    const id = currentAssistantId();
    if (id) void setAssistantModel(id, m);
    setOpen(false);
  };

  return (
    <div class="relative">
      {/* Trigger button: shows only the current model icon (hover title shows the full name) */}
      <button
        class="flex items-center justify-center bg-transparent border-none rounded-md cursor-pointer p-1 transition-all duration-200"
        title={activeModel() ? activeModel()!.model_id : t('chat.model.switch')}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((o) => !o);
        }}
        onMouseEnter={(e) => {
          e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.06)';
        }}
        onMouseLeave={(e) => {
          if (!open()) {
            e.currentTarget.style.background = 'transparent';
          }
        }}
      >
        <Show when={activeModel()} fallback={<Icon name="model" size={18} class="opacity-50" />}>
          <div class="w-[22px] h-[22px] bg-white rounded-full flex items-center justify-center shrink-0 shadow-sm">
            <Show
              when={getModelLogo(activeModel()!.model_id)}
              fallback={
                <span class="text-[11px] font-bold" style={{ color: 'var(--color-dark-700)' }}>
                  {activeModel()!.model_id.charAt(0).toUpperCase()}
                </span>
              }
            >
              <img
                src={getModelLogo(activeModel()!.model_id)!}
                alt="logo"
                class="w-[15px] h-[15px] object-contain"
              />
            </Show>
          </div>
        </Show>
      </button>

      {/* Transparent overlay: rendered only while open; catches clicks on empty space to collapse */}
      <Show when={open()}>
        <div class="fixed inset-0 z-[40]" onClick={() => setOpen(false)} />
      </Show>
      {/* Dropdown panel: always rendered; class toggling drives the expand/collapse transition (opens upward) */}
      <div
        class="absolute bottom-full left-0 mb-2 z-[41] w-[300px] rounded-xl overflow-hidden flex flex-col transition-all duration-200 ease-out origin-bottom"
        classList={{
          'opacity-0 scale-95 translate-y-1 pointer-events-none': !open(),
          'opacity-100 scale-100 translate-y-0 pointer-events-auto': open(),
        }}
        style={{
          background: 'var(--popup-bg)',
          border: '1px solid var(--border-dim)',
          'backdrop-filter': 'var(--popup-blur)',
          'box-shadow': '0 -8px 30px rgba(0,0,0,0.4)',
          'max-height': '340px',
        }}
      >
        <div class="flex flex-col overflow-y-auto scrollbar-thin">
          {/* Cloud models */}
          <div
            class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest sticky top-0"
            style={{
              color: 'rgba(var(--text-base-rgb),0.35)',
              background: 'rgba(var(--text-base-rgb),0.04)',
              'border-bottom': '1px solid var(--border-dim)',
            }}
          >
            {t('chat.model.cloud')}
          </div>
          <div class="p-1.5">
            <For each={cloudModels()}>
              {(model) => (
                <div
                  class="flex items-center gap-2 p-2 rounded-lg cursor-pointer select-none transition-all"
                  style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}
                  classList={{ '!bg-[rgba(var(--primary-rgb),0.12)]': isCurrent(model) }}
                  onClick={() => handlePick(model)}
                  onMouseEnter={(e) => {
                    if (!isCurrent(model)) {
                      e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.06)';
                      e.currentTarget.style.color = 'white';
                    }
                  }}
                  onMouseLeave={(e) => {
                    if (!isCurrent(model)) {
                      e.currentTarget.style.background = 'transparent';
                      e.currentTarget.style.color = 'rgba(var(--text-base-rgb),0.6)';
                    }
                  }}
                >
                  <div class="w-6 h-6 bg-white rounded-full flex items-center justify-center shrink-0">
                    <img
                      src={getModelLogo(model.model_id)}
                      alt="logo"
                      class="w-[14px] h-[14px] object-contain"
                    />
                  </div>
                  <div class="flex-1 min-w-0">
                    <div class="text-[13px] text-white font-medium truncate">{model.model_id}</div>
                    <div
                      class="text-[10px] truncate"
                      style={{ color: 'rgba(var(--primary-rgb),0.5)' }}
                    >
                      {model.owned_by}
                    </div>
                  </div>
                  <Show when={isCurrent(model)}>
                    <Icon
                      name="check"
                      size={14}
                      class="shrink-0"
                      style={{ color: 'rgba(var(--primary-rgb),0.9)' }}
                    />
                  </Show>
                </div>
              )}
            </For>
            <Show when={cloudModels().length === 0}>
              <div
                class="p-3 text-center text-[12px]"
                style={{ color: 'rgba(var(--text-base-rgb),0.25)' }}
              >
                {t('chat.model.noCloud')}
              </div>
            </Show>
          </div>

          {/* Local models */}
          <div
            class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest"
            style={{
              color: 'rgba(var(--text-base-rgb),0.35)',
              background: 'rgba(var(--text-base-rgb),0.04)',
              'border-top': '1px solid var(--border-dim)',
              'border-bottom': '1px solid var(--border-dim)',
            }}
          >
            {t('chat.model.local')}
          </div>
          <div class="p-1.5">
            <For each={localModels()}>
              {(model) => (
                <div
                  class="flex items-center gap-2 p-2 rounded-lg cursor-pointer select-none transition-all"
                  style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}
                  classList={{ '!bg-[rgba(var(--primary-rgb),0.12)]': isCurrent(model) }}
                  onClick={() => handlePick(model)}
                  onMouseEnter={(e) => {
                    if (!isCurrent(model)) {
                      e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.06)';
                      e.currentTarget.style.color = 'white';
                    }
                  }}
                  onMouseLeave={(e) => {
                    if (!isCurrent(model)) {
                      e.currentTarget.style.background = 'transparent';
                      e.currentTarget.style.color = 'rgba(var(--text-base-rgb),0.6)';
                    }
                  }}
                >
                  <div class="w-6 h-6 bg-white rounded-full flex items-center justify-center shrink-0">
                    <img
                      src={getModelLogo(model.model_id)}
                      alt="logo"
                      class="w-[14px] h-[14px] object-contain"
                    />
                  </div>
                  <div class="flex-1 min-w-0">
                    <div class="text-[13px] text-white font-medium truncate">{model.model_id}</div>
                    <div
                      class="text-[10px] truncate"
                      style={{ color: 'rgba(var(--primary-rgb),0.5)' }}
                    >
                      {model.owned_by}
                    </div>
                  </div>
                  <Show when={isCurrent(model)}>
                    <Icon
                      name="check"
                      size={14}
                      class="shrink-0"
                      style={{ color: 'rgba(var(--primary-rgb),0.9)' }}
                    />
                  </Show>
                </div>
              )}
            </For>
            <Show when={localModels().length === 0}>
              <div
                class="p-3 text-center text-[12px]"
                style={{ color: 'rgba(var(--text-base-rgb),0.25)' }}
              >
                {t('chat.model.noLocal')}
              </div>
            </Show>
          </div>
        </div>
      </div>
    </div>
  );
};

export default ModelSelector;
