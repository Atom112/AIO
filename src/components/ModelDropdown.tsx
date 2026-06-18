import { Component, For, createSignal, Show } from 'solid-js';
import Icon from './Icon';
import { ActivatedModel } from '../store/store';
import { modelsCatalog, providerConfigs } from '../store/store';
import { findModel, formatContextWindow } from '../utils/models';

interface ModelDropdownProps {
  selectedModel: ActivatedModel | null;
  onlineModels: ActivatedModel[];
  localModels: ActivatedModel[];
  onSelect: (model: ActivatedModel) => void;
  getModelLogo: (name: string) => string;
}

const ModelDropdown: Component<ModelDropdownProps> = (props) => {
  const [isVisible, setIsVisible] = createSignal(false);

  /** 推导 model 的 provider id (用于查 catalog 元数据) */
  const getProviderIdFor = (model: ActivatedModel): string => {
    if ((model as any).provider_id) return (model as any).provider_id;
    const url = (model.api_url || '').toLowerCase();
    if (url.includes('api.openai.com')) return 'openai';
    if (url.includes('api.anthropic.com')) return 'anthropic';
    if (url.includes('generativelanguage')) return 'google';
    if (url.includes('api.deepseek')) return 'deepseek';
    if (url.includes('api.groq')) return 'groq';
    if (url.includes('api.mistral')) return 'mistral';
    if (url.includes('api.x.ai')) return 'xai';
    if (url.includes('api.cohere')) return 'cohere';
    if (url.includes('openrouter')) return 'openrouter';
    return 'openai';
  };

  const getMeta = (model: ActivatedModel) => {
    const cat = modelsCatalog();
    if (!cat) return null;
    const pid = getProviderIdFor(model);
    return findModel(cat, pid, model.model_id);
  };

  return (
    <div class="relative flex items-center [app-region:no-drag]"
      onMouseEnter={() => setIsVisible(true)}
      onMouseLeave={() => setIsVisible(false)}>
      <div class="nav-icon-link" title="选择模型">
        <Icon src="/icons/app-logo/model-selector.svg" class="w-6 h-6" />
      </div>
      <div
        class="absolute top-full left-1/2 -translate-x-1/2 mt-2 min-w-[520px] rounded-xl shadow-[0_10px_25px_rgba(0,0,0,0.5)] z-[1000] transition-all duration-200 overflow-hidden"
        classList={{ 'invisible opacity-0 translate-y-2': !isVisible(), 'visible opacity-100 translate-y-0': isVisible() }}
        style="background: rgba(18, 22, 35, 0.8); backdrop-filter: blur(30px); -webkit-backdrop-filter: blur(30px); border: 1px solid rgba(255, 255, 255, 0.06);">
        <div class="flex flex-row h-[420px]">
          <div class="flex-1 flex flex-col min-w-[260px]">
            <div class="px-4 py-3 text-[12px] font-bold uppercase tracking-widest" style="color: rgba(255,255,255,0.35); background: rgba(255,255,255,0.04); border-bottom: 1px solid rgba(255,255,255,0.04);">线上模型</div>
            <div class="flex-1 overflow-y-auto p-2 scrollbar-thin">
              <For each={props.onlineModels}>
                {(model) => {
                  const meta = () => getMeta(model);
                  const noKey = () => !model.api_key;
                  return (
                    <div
                      class="flex flex-row items-center gap-2.5 p-2 text-sm rounded-lg cursor-pointer select-none transition-all"
                      style="color: rgba(255,255,255,0.5);"
                      classList={{ '!bg-[rgba(124,154,191,0.12)] !border-l-[3px] !border-[rgba(124,154,191,0.2)]': props.selectedModel?.model_id === model.model_id && props.selectedModel?.api_url === model.api_url }}
                      onClick={() => { props.onSelect(model); setIsVisible(false); }}
                      onMouseEnter={(e) => { if (!(props.selectedModel?.model_id === model.model_id && props.selectedModel?.api_url === model.api_url)) e.currentTarget.style.background = 'rgba(255,255,255,0.06)'; e.currentTarget.style.color = 'white'; }}
                      onMouseLeave={(e) => { if (!(props.selectedModel?.model_id === model.model_id && props.selectedModel?.api_url === model.api_url)) { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = 'rgba(255,255,255,0.5)'; } }}>
                      <div class="w-7 h-7 bg-white rounded-full flex items-center justify-center shrink-0 shadow-sm">
                        <img src={props.getModelLogo(model.model_id)} alt="logo" class="w-[18px] h-[18px] object-contain" />
                      </div>
                      <div class="flex-1 flex flex-col items-start justify-center overflow-hidden text-left min-w-0">
                        <div class="max-w-[200px] text-[13px] text-white font-medium truncate">{model.model_id}</div>
                        <div style="color: rgba(124,154,191,0.5); font-size: 10px;">{model.owned_by}</div>
                        <div class="flex gap-1 mt-0.5 flex-wrap">
                          <Show when={meta()}>
                            <span class="text-[9px] px-1 py-0.5 rounded bg-pri-20 text-pri">{formatContextWindow(meta()!.contextWindow)}</span>
                            <Show when={meta()!.capabilities.tools}>
                              <span class="text-[9px] px-1 py-0.5 rounded bg-green-500/20 text-green-300">工具</span>
                            </Show>
                            <Show when={meta()!.capabilities.vision}>
                              <span class="text-[9px] px-1 py-0.5 rounded bg-blue-500/20 text-blue-300">视觉</span>
                            </Show>
                            <Show when={meta()!.capabilities.reasoning}>
                              <span class="text-[9px] px-1 py-0.5 rounded bg-purple-500/20 text-purple-300">推理</span>
                            </Show>
                            <Show when={meta()!.status === 'deprecated'}>
                              <span class="text-[9px] px-1 py-0.5 rounded bg-red-500/20 text-red-300">已弃用</span>
                            </Show>
                          </Show>
                          <Show when={noKey()}>
                            <span class="text-[9px] px-1 py-0.5 rounded bg-yellow-500/20 text-yellow-300" title="该 provider 未配置 API Key">⚠ 未配置 Key</span>
                          </Show>
                        </div>
                      </div>
                    </div>
                  );
                }}
              </For>
              {props.onlineModels.length === 0 && (
                <div class="p-5 text-center text-[13px]" style="color: rgba(255,255,255,0.2);">
                  <div>无线上模型</div>
                  <div class="text-[10px] mt-1.5 leading-relaxed" style="color: rgba(255,255,255,0.25);">
                    去 <span style="color: rgba(124,154,191,0.5); font-medium;">设置中心 → 供应商设置</span><br/>
                    启用 provider 并填写 API Key
                  </div>
                </div>
              )}
            </div>
          </div>
          <div style="width: 1px; background: rgba(255,255,255,0.04); align-self: stretch;"></div>
          <div class="flex-1 flex flex-col min-w-[240px]">
            <div class="px-4 py-3 text-[12px] font-bold uppercase tracking-widest" style="color: rgba(255,255,255,0.35); background: rgba(255,255,255,0.04); border-bottom: 1px solid rgba(255,255,255,0.04);">本地模型</div>
            <div class="flex-1 overflow-y-auto p-2 scrollbar-thin">
              <For each={props.localModels}>
                {(model) => (
                  <div
                    class="flex flex-row items-center gap-2.5 p-2 text-sm rounded-lg cursor-pointer select-none transition-all"
                    style="color: rgba(255,255,255,0.5);"
                    classList={{ '!bg-[rgba(124,154,191,0.12)] !border-l-[3px] !border-[rgba(124,154,191,0.2)]': props.selectedModel?.model_id === model.model_id }}
                    onClick={() => { props.onSelect(model); setIsVisible(false); }}
                    onMouseEnter={(e) => { if (props.selectedModel?.model_id !== model.model_id) e.currentTarget.style.background = 'rgba(255,255,255,0.06)'; e.currentTarget.style.color = 'white'; }}
                    onMouseLeave={(e) => { if (props.selectedModel?.model_id !== model.model_id) { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = 'rgba(255,255,255,0.5)'; } }}>
                    <div class="w-7 h-7 bg-white rounded-full flex items-center justify-center shrink-0 shadow-sm">
                      <img src={props.getModelLogo(model.model_id)} alt="logo" class="w-[18px] h-[18px] object-contain" />
                    </div>
                    <div class="flex-1 flex flex-col items-start justify-center overflow-hidden text-left">
                      <div class="max-w-[160px] text-[13px] text-white font-medium truncate">{model.model_id}</div>
                      <div style="color: rgba(124,154,191,0.5); font-size: 10px;">{model.owned_by}</div>
                    </div>
                  </div>
                )}
              </For>
              {props.localModels.length === 0 && <div class="p-5 text-center text-[13px]" style="color: rgba(255,255,255,0.2);">无本地模型</div>}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default ModelDropdown;
