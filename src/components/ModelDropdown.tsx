import { Component, For, createSignal } from 'solid-js';
import Icon from './Icon';
import { ActivatedModel } from '../store/store';

interface ModelDropdownProps {
  selectedModel: ActivatedModel | null;
  onlineModels: ActivatedModel[];
  localModels: ActivatedModel[];
  onSelect: (model: ActivatedModel) => void;
  getModelLogo: (name: string) => string;
}

const ModelDropdown: Component<ModelDropdownProps> = (props) => {
  const [isVisible, setIsVisible] = createSignal(false);

  return (
    <div class="relative flex items-center [app-region:no-drag]"
      onMouseEnter={() => setIsVisible(true)}
      onMouseLeave={() => setIsVisible(false)}>
      <div class="nav-icon-link" title="选择模型">
        <Icon src="/icons/app-logo/model-selector.svg" class="w-6 h-6" />
      </div>
      <div
        class="absolute top-full left-1/2 -translate-x-1/2 mt-2 min-w-[480px] rounded-xl shadow-[0_10px_25px_rgba(0,0,0,0.5)] z-[1000] transition-all duration-200 overflow-hidden"
        classList={{ 'invisible opacity-0 translate-y-2': !isVisible(), 'visible opacity-100 translate-y-0': isVisible() }}
        style="background: rgba(18, 22, 35, 0.8); backdrop-filter: blur(30px); -webkit-backdrop-filter: blur(30px); border: 1px solid rgba(255, 255, 255, 0.06);">
        <div class="flex flex-row h-[400px]">
          <div class="flex-1 flex flex-col min-w-[240px]">
            <div class="px-4 py-3 text-[12px] font-bold uppercase tracking-widest" style="color: rgba(255,255,255,0.35); background: rgba(255,255,255,0.04); border-bottom: 1px solid rgba(255,255,255,0.04);">线上模型</div>
            <div class="flex-1 overflow-y-auto p-2 scrollbar-thin">
              <For each={props.onlineModels}>
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
              {props.onlineModels.length === 0 && <div class="p-5 text-center text-[13px]" style="color: rgba(255,255,255,0.2);">无线上模型</div>}
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
