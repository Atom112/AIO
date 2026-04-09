import { Component, createSignal, For, createMemo, onCleanup, onMount } from 'solid-js';
import { config, saveConfig, setDatas, selectedModel } from '../store/store';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

/** 从 API 获取的原始模型信息 */
interface ModelItem { 
    id: string; 
    owned_by: string; 
}

/** 已激活的模型完整配置（含 API 连接信息） */
interface ActivatedModel { 
    owned_by: string; 
    api_url: string; 
    api_key: string; 
    model_id: string; 
    local_path?: string; 
}

/**
 * 提供商设置页面组件
 * @component
 * @description 管理 AI 模型 API 配置和模型激活状态，支持云端和本地模型。
 * @returns {JSX.Element} 设置页面 JSX 元素
 */
const ProviderSettings: Component = () => {

    const [apiUrl, setApiUrl] = createSignal(config().apiUrl);                  //API 供应商网址输入值
    const [apiKey, setApiKey] = createSignal(config().apiKey);                  //API Key 输入值
    const [saveStatus, setSaveStatus] = createSignal("");                       //保存状态提示文本（如"配置已保存"）
    const [localModelPath, setLocalModelPath] = createSignal("");               //选中的本地 .gguf 模型文件路径
    const [isLocalRunning, setIsLocalRunning] = createSignal(false);            //本地 Llama 服务是否正在运行
    const [models, setModels] = createSignal<ModelItem[]>([]);                  //从 API 获取的可用模型列表
    const [isLoading, setIsLoading] = createSignal(false);                      //是否正在查询模型列表
    const [activatedModels, setActivatedModels] = createSignal<ActivatedModel[]>([]);    //已激活的模型配置列表（持久化到本地）
    const [searchQuery, setSearchQuery] = createSignal("");                     //可用模型搜索关键词
    const [selectedProvider, setSelectedProvider] = createSignal("All");        //可用模型厂商筛选条件
    const [isDropdownOpen, setIsDropdownOpen] = createSignal(false);            //可用模型厂商下拉菜单展开状态
    const [searchQueryAct, setSearchQueryAct] = createSignal("");               // 已激活模型搜索关键词
    const [selectedProviderAct, setSelectedProviderAct] = createSignal("All");  //已激活模型厂商筛选条件
    const [isDropdownOpenAct, setIsDropdownOpenAct] = createSignal(false);      //已激活模型厂商下拉菜单展开状态

    let dropdownRef: HTMLDivElement | undefined;        //可用模型厂商下拉容器引用（用于点击外部关闭）
    let dropdownRefAct: HTMLDivElement | undefined;    //已激活模型厂商下拉容器引用（用于点击外部关闭）

    /**
     * 根据模型名称获取对应的品牌 Logo 路径
     * @param {string} modelName - 模型名称或 ID
     * @returns {string} Logo 图片的 URL 路径
     */
    const getModelLogo = (modelName: string) => {
        const name = modelName.toLowerCase();
        if (name.includes('gpt')) return '/icons/model-logo/openai.svg';
        if (name.includes('claude')) return '/icons/model-logo/claude-color.svg';
        if (name.includes('grok')) return '/icons/model-logo/grok.svg';
        if (name.includes('gemini')) return '/icons/model-logo/gemini-color.svg';
        if (name.includes('deepseek')) return '/icons/model-logo/deepseek-color.svg';
        if (name.includes('qwen')) return '/icons/model-logo/qwen-color.svg';
        if (name.includes('kimi') || name.includes('moonshot')) return '/icons/model-logo/moonshot.svg';
        if (name.includes('doubao')) return '/icons/model-logo/doubao-color.svg';
        if (name.includes('glm')) return '/icons/model-logo/zhipu-color.svg';
        return '/icons/model-logo/ollama.svg';
    };

    /**
     * 打开文件对话框选择本地 .gguf 模型文件
     */
    const selectModelFile = async () => {
        const file = await open({
            multiple: false,
            filters: [{ name: 'GGUF Models', extensions: ['gguf'] }]
        });

        if (file) {
            setLocalModelPath(file);

            // 从路径提取文件名并去除扩展名
            const fileNameWithExt = file.split(/[\\/]/).pop() || "local-model";
            const modelName = fileNameWithExt.replace(/\.[^/.]+$/, "");

            // 构建本地模型配置对象
            const localModelInfo: ActivatedModel = {
                model_id: modelName,
                owned_by: "Local-Llama.cpp",
                api_url: "http://127.0.0.1:8080/v1", // 默认本地端口
                api_key: "local-no-key",
                local_path: file
            };

            // 防止重复添加相同路径的模型
            const exists = activatedModels().find(m => m.local_path === file);
            if (!exists) {
                const newList = [...activatedModels(), localModelInfo];
                setActivatedModels(newList);
                setDatas('activatedModels', newList); // 同步到全局 Store
                await invoke('save_activated_models', { models: newList });
                setSaveStatus(`已添加本地模型: ${modelName}`);
            }
        }
    };

    /**
     * 切换本地 Llama 服务器的运行状态（启动/停止）
     */
    const toggleLocalEngine = async () => {
        if (isLocalRunning()) {
            // 停止分支
            await invoke('stop_local_server');
            setIsLocalRunning(false);
            setSaveStatus("本地引擎已停止");
        } else {
            // 启动分支
            if (!localModelPath()) return alert("请先选择模型文件");
            
            try {
                // 保存路径到配置
                const currentCfg = await invoke<any>('load_app_config');
                await invoke('save_app_config', {
                    config: { ...currentCfg, localModelPath: localModelPath() }
                });
                
                setSaveStatus("正在启动本地引擎...");

                // 启动本地服务
                const serverUrl: string = await invoke('start_local_server', {
                    modelPath: localModelPath(),
                    port: 8080,
                    gpuLayers: 99 // 默认尽可能使用 GPU 加速
                });

                setIsLocalRunning(true);
                setSaveStatus("本地引擎已就绪");

                // 提取模型名称
                const fullPath = localModelPath();
                const fileNameWithExt = fullPath.split(/[\\/]/).pop() || "local-model";
                const modelName = fileNameWithExt.replace(/\.[^/.]+$/, "");

                // 构建服务启动后的模型配置
                const localModelInfo: ActivatedModel = {
                    model_id: modelName,
                    owned_by: "Local-Llama.cpp",
                    api_url: serverUrl,
                    api_key: "local-no-key"
                };

                // 防止重复添加
                if (!activatedModels().some(m => m.model_id === modelName && m.api_url === serverUrl)) {
                    const newList = [...activatedModels(), localModelInfo];
                    setActivatedModels(newList);
                    setDatas('activatedModels', newList);
                    await invoke('save_activated_models', { models: newList });
                }

                setSaveStatus(`本地模型 ${modelName} 已启动`);
            } catch (err) {
                alert("启动失败: " + err);
                setIsLocalRunning(false);
            }
        }
    };

    /**
     * 从激活列表中移除指定模型
     * @param {ActivatedModel} target - 待移除的模型配置对象
     */
    const removeActivatedModel = async (target: ActivatedModel) => {
        // 根据 model_id 和 api_url 组合唯一标识过滤
        const newList = activatedModels().filter(m =>
            !(m.model_id === target.model_id && m.api_url === target.api_url)
        );

        setActivatedModels(newList);
        setDatas('activatedModels', newList);
        await invoke('save_activated_models', { models: newList });
    };

    /**
     * 组件挂载时初始化数据
     */
    onMount(async () => {
        // 加载持久化数据
        const listAct = await invoke<ActivatedModel[]>('load_activated_models');
        setActivatedModels(listAct);
        
        const cachedModels = await invoke<ModelItem[]>('load_fetched_models');
        if (cachedModels) setModels(cachedModels);
        
        const running = await invoke<boolean>('is_local_server_running');
        setIsLocalRunning(running);
        
        const configData = await invoke<any>('load_app_config');
        if (configData.localModelPath) setLocalModelPath(configData.localModelPath);

        // 点击外部关闭下拉菜单
        const handleClickOutside = (e: MouseEvent) => {
            if (dropdownRef && !dropdownRef.contains(e.target as Node)) setIsDropdownOpen(false);
            if (dropdownRefAct && !dropdownRefAct.contains(e.target as Node)) setIsDropdownOpenAct(false);
        };
        window.addEventListener('click', handleClickOutside);
        onCleanup(() => window.removeEventListener('click', handleClickOutside));
    });

    /** 可用模型的厂商列表（去重，用于筛选下拉） */
    const providers = createMemo(() => ["All", ...Array.from(new Set(models().map(m => m.owned_by || "unknown")))]);
    
    /** 筛选后的可用模型列表（搜索 + 厂商筛选） */
    const filteredModels = createMemo(() => 
        models().filter(m => 
            m.id.toLowerCase().includes(searchQuery().toLowerCase()) && 
            (selectedProvider() === "All" || m.owned_by === selectedProvider())
        )
    );

    /** 已激活模型的厂商列表（去重） */
    const providersAct = createMemo(() => ["All", ...Array.from(new Set(activatedModels().map(m => m.owned_by || "unknown")))]);
    
    /** 筛选后的已激活模型列表（搜索 + 厂商筛选） */
    const filteredActivatedModels = createMemo(() => 
        activatedModels().filter(m => 
            m.model_id.toLowerCase().includes(searchQueryAct().toLowerCase()) && 
            (selectedProviderAct() === "All" || m.owned_by === selectedProviderAct())
        )
    );

    /**
     * 切换模型的激活状态（勾选/取消勾选）
     * @param {ModelItem} item - 从 API 获取的可用模型项
     */
    const toggleActivation = async (item: ModelItem) => {
        // 检查是否已使用当前 API 配置激活
        const isCurrentlyActive = activatedModels().some(m => 
            m.model_id === item.id && m.api_url === apiUrl()
        );
        
        let newList;
        if (isCurrentlyActive) {
            // 取消激活：移除匹配项
            newList = activatedModels().filter(m => 
                !(m.model_id === item.id && m.api_url === apiUrl())
            );
        } else {
            // 激活：构建完整配置对象
            const newActive: ActivatedModel = {
                model_id: item.id,
                owned_by: item.owned_by || "unknown",
                api_url: apiUrl(),
                api_key: apiKey(),
            };
            newList = [...activatedModels(), newActive];
        }
        
        setActivatedModels(newList);
        setDatas('activatedModels', newList); // 同步到全局 Store 供 NavBar 等组件使用
        await invoke('save_activated_models', { models: newList });
    };

    /**
     * 保存当前 API 配置
     */
    const handleSave = async () => {
        const newConfig = { 
            apiUrl: apiUrl(), 
            apiKey: apiKey(), 
            defaultModel: selectedModel()?.model_id || "" 
        };
        await invoke('save_app_config', { config: newConfig });
        saveConfig(newConfig as any);
        setSaveStatus("配置已成功保存！");
        setTimeout(() => setSaveStatus(""), 3000);
    };

    /**
     * 从配置的 API 端点获取可用模型列表
     */
    const handleQueryModels = async () => {
        setIsLoading(true);
        try {
            const list: ModelItem[] = await invoke('fetch_models', {
                apiUrl: apiUrl(),
                apiKey: apiKey()
            });
            setModels(list);
            await invoke('save_fetched_models', { models: list });
        } catch (err) {
            alert(`查询失败: ${err}`);
        } finally {
            setIsLoading(false);
        }
    };

return (
        <div class="flex gap-4 h-full">
            {/* 左侧配置面板 */}
            <div class="w-[22%] p-5 border border-pri rounded-xl bg-pri-5 overflow-y-auto">
                <div class="border-b border-pri-20 pb-2.5 mb-8">
                    <h3 class="text-lg font-bold">API 服务配置</h3>
                </div>
                
                <div class="flex flex-col gap-5">
                    <div class="flex flex-col gap-2">
                        <label class="text-sm font-bold">API 供应商网址</label>
                        <input 
                            type="text" 
                            class="bg-dark-850 border border-dark-300 text-white p-3 rounded-md outline-none transition-colors duration-300 focus:border-pri" 
                            value={apiUrl()} 
                            onInput={(e) => setApiUrl(e.currentTarget.value)} 
                        />
                    </div>
                    
                    <div class="flex flex-col gap-2">
                        <label class="text-sm font-bold">API Key</label>
                        <input 
                            type="password" 
                            class="bg-dark-850 border border-dark-300 text-white p-3 rounded-md outline-none transition-colors duration-300 focus:border-pri" 
                            value={apiKey()} 
                            onInput={(e) => setApiKey(e.currentTarget.value)} 
                        />
                    </div>
                    
                    <button 
                        class="mt-2.5 primary-btn p-3"
                        onClick={handleSave}
                    >
                        保存当前配置
                    </button>
                    
                    <button 
                        class="primary-btn p-3 disabled:opacity-50"
                        onClick={handleQueryModels} 
                        disabled={isLoading()}
                    >
                        {isLoading() ? "查询中..." : "获取可用模型列表"}
                    </button>

                    <div class="flex flex-col gap-2">
                        <label class="text-sm font-bold border-t border-pri-20 pt-4 mt-2">本地模型管理 (.gguf)</label>
                        <div class="flex flex-col gap-2.5">
                            <button 
                                class="primary-btn p-3 w-full"
                                onClick={selectModelFile} 
                            >
                                选择并添加本地模型
                            </button>

                            <button
                                class="p-3 font-bold rounded-md cursor-pointer transition-all duration-200 hover:opacity-80 text-black border-none"
                                style={{
                                    "background-color": isLocalRunning() ? "#E08090" : "var(--primary-color)"
                                }}
                                onClick={toggleLocalEngine}
                            >
                                {isLocalRunning() ? "停止本地 Llama 服务器" : "启动本地 Llama 服务器"}
                            </button>
                        </div>
                    </div>
                    
                    {saveStatus() && (
                        <div class="text-sm text-pri animate-pulse text-center">
                            {saveStatus()}
                        </div>
                    )}
                </div>
            </div>

            {/* 中间已激活面板 */}
            <div class="flex-1 border border-pri rounded-xl bg-pri-5 flex flex-col min-w-0 overflow-hidden shadow-[0_4px_20px_rgba(0,0,0,0.3)]">
                <div class="p-5 border-b border-pri-20 flex flex-col gap-4 bg-pri-5">
                    <h4 class="font-bold flex items-center gap-2">
                        <span class="w-2 h-2 rounded-full bg-green-400 animate-pulse"></span>
                        已激活模型 ({filteredActivatedModels().length})
                    </h4>
                    <div class="flex gap-3">
                        <input 
                            type="text" 
                            placeholder="搜索已激活..." 
                            class="flex-[2] bg-dark-850 border border-dark-300 text-white px-3 py-2 rounded-md outline-none text-xs transition-colors focus:border-pri" 
                            onInput={(e) => setSearchQueryAct(e.currentTarget.value)} 
                        />
                        
                        <div class="relative flex-1 min-w-[140px]" ref={dropdownRefAct}>
                            <div 
                                class={`bg-dark-850 border border-dark-300 px-3 py-2 rounded-md cursor-pointer text-xs flex justify-between items-center hover:border-pri transition-all ${isDropdownOpenAct() ? 'border-pri' : ''}`} 
                                onClick={() => setIsDropdownOpenAct(!isDropdownOpenAct())}
                            >
                                {selectedProviderAct() === "All" ? "所有厂商" : selectedProviderAct()}
                                <span class={`transition-transform duration-200 ${isDropdownOpenAct() ? 'rotate-180' : ''}`}>▼</span>
                            </div>
                            {isDropdownOpenAct() && (
                                <div class="absolute top-[calc(100%+5px)] left-0 right-0 bg-dark-850 border border-pri rounded-lg z-[100] max-h-[250px] overflow-y-auto shadow-2xl">
                                    <For each={providersAct()}>
                                        {(p) => (
                                            <div 
                                                class={`p-2.5 text-[#eee] cursor-pointer text-xs hover:bg-pri-20 hover:text-pri transition-colors ${selectedProviderAct() === p ? 'bg-pri-10 text-pri' : ''}`}
                                                onClick={() => { setSelectedProviderAct(p); setIsDropdownOpenAct(false); }}
                                            >
                                                {p === "All" ? "所有厂商" : p}
                                            </div>
                                        )}
                                    </For>
                                </div>
                            )}
                        </div>
                    </div>
                </div>
                
                <div class="flex-1 overflow-y-auto p-4 space-y-2.5 scrollbar-thin scrollbar-thumb-[#333]">
                    <For each={filteredActivatedModels()}>
                        {(m) => (
                            <div class="border-l-[3px] border-green-500 bg-white/5 rounded-lg p-3 flex items-center gap-4 transition-all duration-200 hover:bg-pri-20">
                                <div class="w-8 h-8 bg-white rounded-full flex items-center justify-center shrink-0 border border-pri-20">
                                    <img src={getModelLogo(m.model_id)} alt="logo" class="w-5 h-5 object-contain" />
                                </div>

                                <div class="grow flex flex-col font-mono">
                                    <span class="text-[#eee] font-medium text-sm truncate">{m.model_id}</span>
                                    <span class="text-[#666] text-[10px]">
                                        {m.api_url.includes('127.0.0.1') ? '● 本地服务' : m.api_url.replace('https://', '').split('/')[0]}
                                    </span>
                                </div>
                                <button 
                                    class="border border-danger bg-transparent text-danger px-2.5 py-1 rounded cursor-pointer whitespace-nowrap shrink-0 transition-all duration-200 hover:text-dark-850 hover:bg-danger text-xs" 
                                    onClick={() => removeActivatedModel(m)}
                                >
                                    移除
                                </button>
                            </div>
                        )}
                    </For>
                </div>
            </div>

            {/* 右侧可用列表面板 */}
            <div class="flex-1 border border-pri rounded-xl bg-pri-5 flex flex-col min-w-0 overflow-hidden shadow-[0_4px_20px_rgba(0,0,0,0.3)]">
                <div class="p-5 border-b border-pri-20 flex flex-col gap-4 bg-pri-5">
                    <h4 class="font-bold">可用模型列表 ({filteredModels().length})</h4>
                    <div class="flex gap-3">
                        <input 
                            type="text" 
                            placeholder="搜索模型名称..." 
                            class="flex-[2] bg-dark-850 border border-dark-300 text-white px-3 py-2 rounded-md outline-none text-xs transition-colors focus:border-pri" 
                            onInput={(e) => setSearchQuery(e.currentTarget.value)} 
                        />
                        <div class="relative flex-1 min-w-[140px]" ref={dropdownRef}>
                            <div 
                                class={`bg-dark-850 border border-dark-300 px-3 py-2 rounded-md cursor-pointer text-xs flex justify-between items-center hover:border-pri transition-all ${isDropdownOpen() ? 'border-pri' : ''}`} 
                                onClick={() => setIsDropdownOpen(!isDropdownOpen())}
                            >
                                {selectedProvider() === "All" ? "所有厂商" : selectedProvider()}
                                <span class={`transition-transform duration-200 ${isDropdownOpen() ? 'rotate-180' : ''}`}>▼</span>
                            </div>
                            {isDropdownOpen() && (
                                <div class="absolute top-[calc(100%+5px)] left-0 right-0 bg-dark-850 border border-pri rounded-lg z-[100] max-h-[250px] overflow-y-auto shadow-2xl text-xs">
                                    <For each={providers()}>
                                        {(p) => (
                                            <div 
                                                class={`p-2.5 text-[#eee] cursor-pointer hover:bg-pri-20 hover:text-pri transition-colors ${selectedProvider() === p ? 'bg-pri-10 text-pri' : ''}`}
                                                onClick={() => { setSelectedProvider(p); setIsDropdownOpen(false); }}
                                            >
                                                {p === "All" ? "所有厂商" : p}
                                            </div>
                                        )}
                                    </For>
                                </div>
                            )}
                        </div>
                    </div>
                </div>
                
                <div class="flex-1 overflow-y-auto p-4 space-y-2.5 scrollbar-thin scrollbar-thumb-[#333]">
                    {models().length === 0 && (
                        <div class="text-[#444] text-center mt-12 italic text-sm">
                            点击左侧“查询模型”按钮获取列表
                        </div>
                    )}
                    
                    <For each={filteredModels()}>
                        {(m) => (
                            <div class="border-l-[3px] border-pri bg-white/5 rounded-lg p-3 flex items-center gap-4 transition-all duration-200 hover:bg-pri-20">
                                <div class="w-8 h-8 bg-white rounded-full flex items-center justify-center shrink-0 border border-pri-20">
                                    <img src={getModelLogo(m.id)} alt="logo" class="w-5 h-5 object-contain" />
                                </div>

                                <div class="grow flex flex-col font-mono text-sm">
                                    <span class="text-[#eee] font-medium truncate">{m.id}</span>
                                    <span class="text-[#666] text-[10px]">Provider: {m.owned_by}</span>
                                </div>
                                
                                <div class="shrink-0 flex items-center">
                                    <label class="relative inline-block w-9 h-5 cursor-pointer">
                                        <input 
                                            class="opacity-0 w-0 h-0 peer"
                                            type="checkbox"
                                            checked={activatedModels().some(am => 
                                                am.model_id === m.id && am.api_url === apiUrl()
                                            )}
                                            onChange={() => toggleActivation(m)} 
                                        />
                                        <span class="absolute inset-0 bg-dark-300 border border-dark-100 rounded-full transition-all duration-300 peer-checked:bg-pri peer-checked:border-pri after:content-[''] after:absolute after:top-0.5 after:left-0.5 after:bg-white after:w-3.5 after:h-3.5 after:rounded-full after:transition-all peer-checked:after:translate-x-4"></span>
                                    </label>
                                </div>
                            </div>
                        )}
                    </For>
                </div>
            </div>
        </div>
    );
};

export default ProviderSettings;