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
    <div
      class="relative flex items-center [app-region:no-drag]"
      onMouseEnter={() => setIsVisible(true)}
      onMouseLeave={() => setIsVisible(false)}
    >
      <div class="nav-icon-link" title="选择模型">
        <Icon src="/icons/app-logo/model-selector.svg" class="w-6 h-6" />
      </div>

      <div 
        class="absolute top-full left-1/2 -translate-x-1/2 mt-2 bg-dark min-w-[480px] border border-dark-300 rounded-xl shadow-[0_10px_25px_rgba(0,0,0,0.5)] z-[1000] transition-all duration-200 overflow-hidden"
        classList={{ 
          'invisible opacity-0 translate-y-2': !isVisible(), 
          'visible opacity-100 translate-y-0 border-pri': isVisible() 
        }}
      >
        <div class="flex flex-row h-[400px]">
          {/* 线上模型列 */}
          <div class="flex-1 flex flex-col min-w-[240px]">
            <div class="px-4 py-3 text-[12px] font-bold text-[#888] uppercase tracking-widest bg-dark-600 border-b border-dark-300">线上模型</div>
            <div class="flex-1 overflow-y-auto p-2 scrollbar-thin scrollbar-thumb-[#444]">
              <For each={props.onlineModels}>
                {(model) => (
                  <div
                    class="flex flex-row items-center gap-2.5 p-2 text-[#a0a0a0] text-sm rounded-lg cursor-pointer select-none transition-all hover:bg-dark-200 hover:text-white"
                    classList={{ 'bg-pri-20 border-l-3 border-pri': props.selectedModel?.model_id === model.model_id }}
                    onClick={() => {
                        props.onSelect(model);
                        setIsVisible(false);
                    }}
                  >
                    <div class="w-7 h-7 bg-white rounded-full flex items-center justify-center shrink-0 shadow-sm"
                         classList={{ 'border border-pri': props.selectedModel?.model_id === model.model_id }}>
                      <img src={props.getModelLogo(model.model_id)} alt="logo" class="w-[18px] h-[18px] object-contain" />
                    </div>
                    <div class="flex-1 flex flex-col items-start justify-center overflow-hidden text-left">
                      <div class="max-w-[160px] text-[13px] text-white font-medium truncate">{model.model_id}</div>
                      <div class="text-[10px] text-pri opacity-70">{model.owned_by}</div>
                    </div>
                  </div>
                )}
              </For>
              {props.onlineModels.length === 0 && <div class="p-5 text-center text-[#555] text-[13px]">无线上模型</div>}
            </div>
          </div>

          <div class="w-[1px] bg-dark-300 self-stretch"></div>

          {/* 本地模型列 */}
          <div class="flex-1 flex flex-col min-w-[240px]">
            <div class="px-4 py-3 text-[12px] font-bold text-[#888] uppercase tracking-widest bg-dark-600 border-b border-dark-300">本地模型</div>
            <div class="flex-1 overflow-y-auto p-2 scrollbar-thin scrollbar-thumb-[#444]">
              <For each={props.localModels}>
                {(model) => (
                  <div
                    class="flex flex-row items-center gap-2.5 p-2 text-[#a0a0a0] text-sm rounded-lg cursor-pointer select-none transition-all hover:bg-dark-200 hover:text-white"
                    classList={{ 'bg-pri-20 border-l-3 border-pri': props.selectedModel?.model_id === model.model_id }}
                    onClick={() => {
                        props.onSelect(model);
                        setIsVisible(false);
                    }}
                  >
                    <div class="w-7 h-7 bg-white rounded-full flex items-center justify-center shrink-0 shadow-sm"
                         classList={{ 'border border-pri': props.selectedModel?.model_id === model.model_id }}>
                      <img src={props.getModelLogo(model.model_id)} alt="logo" class="w-[18px] h-[18px] object-contain" />
                    </div>
                    <div class="flex-1 flex flex-col items-start justify-center overflow-hidden text-left">
                      <div class="max-w-[160px] text-[13px] text-white font-medium truncate">{model.model_id}</div>
                      <div class="text-[10px] text-pri opacity-70">Local</div>
                    </div>
                  </div>
                )}
              </For>
              {props.localModels.length === 0 && <div class="p-5 text-center text-[#555] text-[13px]">无本地模型</div>}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default ModelDropdown;