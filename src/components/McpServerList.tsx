import { Component, For, Show, createSignal, createMemo, onMount } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import {
    mcpServers, setMcpServers, mcpServerStatus, setMcpServerStatus,
    startMcpServerAndRefresh, datas, setDatas, saveSingleAssistantToBackend,
} from '../store/store';
import type { McpServerConfig } from '../types/mcp';
import { transportLabel, statusLabel, statusColor, emptyMcpServerConfig } from '../utils/mcp';
import McpServerDetail from './McpServerDetail';

const McpServerList: Component = () => {
    const [editingConfig, setEditingConfig] = createSignal<McpServerConfig | null>(null);
    const [isCreating, setIsCreating] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    onMount(async () => {
        try {
            const list = await invoke<McpServerConfig[]>('list_mcp_servers');
            const map: Record<string, McpServerConfig> = {};
            for (const c of list) map[c.id] = c;
            setMcpServers(map);
        } catch (e: any) {
            setError(String(e));
        }
    });

    const handleStart = async (id: string) => {
        try {
            await startMcpServerAndRefresh(id);
        } catch (e: any) {
            setError(`启动失败: ${e}`);
        }
    };

    const handleStop = async (id: string) => {
        try {
            await invoke('stop_mcp_server', { id });
            setMcpServerStatus({ ...mcpServerStatus(), [id]: { id, status: 'disconnected', toolCount: 0 } });
        } catch (e: any) {
            setError(`停止失败: ${e}`);
        }
    };

    const handleRemove = async (id: string) => {
        if (!confirm('确定删除此 MCP server 配置？此操作不可恢复。')) return;
        try {
            await invoke('remove_mcp_server', { id });
            const m = { ...mcpServers() };
            delete m[id];
            setMcpServers(m);

            const affectedAssistantIds = datas.assistants
                .filter(assistant => assistant.mcpServerIds?.includes(id))
                .map(assistant => assistant.id);
            for (const assistantId of affectedAssistantIds) {
                setDatas(
                    'assistants',
                    assistant => assistant.id === assistantId,
                    'mcpServerIds',
                    (serverIds: string[] | undefined) => (serverIds ?? []).filter(serverId => serverId !== id),
                );
            }
            await Promise.all(affectedAssistantIds.map(saveSingleAssistantToBackend));
        } catch (e: any) {
            setError(`删除失败: ${e}`);
        }
    };

    const handleEdit = (cfg: McpServerConfig) => {
        setIsCreating(false);
        setEditingConfig({ ...cfg });
    };

    const handleSave = async (cfg: McpServerConfig) => {
        try {
            await invoke('add_mcp_server', { config: cfg });
            setMcpServers({ ...mcpServers(), [cfg.id]: cfg });
            setEditingConfig(null);
            setIsCreating(false);
        } catch (e: any) {
            setError(`保存失败: ${e}`);
        }
    };

    const handleTest = async (cfg: McpServerConfig): Promise<{ ok: boolean; tools?: any[]; error?: string }> => {
        try {
            const tools = await invoke<any[]>('test_mcp_server_connection', { config: cfg });
            return { ok: true, tools };
        } catch (e: any) {
            return { ok: false, error: String(e) };
        }
    };

    const sortedServers = createMemo(() => {
        return Object.values(mcpServers()).sort((a, b) => a.displayName.localeCompare(b.displayName));
    });

    return (
        <div class="flex flex-col gap-4 p-6 h-full overflow-y-auto" style="color: rgba(255,255,255,0.85);">
            <div class="flex items-center justify-between">
                <div>
                    <h2 class="text-lg font-semibold">MCP 服务器</h2>
                    <p class="text-xs" style="color: rgba(255,255,255,0.5); margin-top: 4px;">
                        配置 Model Context Protocol 服务器，让模型在聊天中自动调用工具
                    </p>
                </div>
                <button
                    class="px-3 py-1.5 rounded-md cursor-pointer transition-colors"
                    style="background: rgba(124,154,191,0.2); border: 1px solid rgba(124,154,191,0.3);"
                    onClick={() => { setIsCreating(true); setEditingConfig(emptyMcpServerConfig()); }}
                >
                    + 添加服务器
                </button>
            </div>

            <Show when={error()}>
                <div
                    class="px-3 py-2 rounded-md text-sm"
                    style="background: rgba(255,77,77,0.1); border: 1px solid rgba(255,77,77,0.3); color: #ff8a8a;"
                >
                    {error()}
                    <button class="ml-3 underline" onClick={() => setError(null)}>关闭</button>
                </div>
            </Show>

            <div class="flex flex-col gap-2">
                <For each={sortedServers()} fallback={
                    <div class="text-sm py-8 text-center" style="color: rgba(255,255,255,0.4);">
                        还没有 MCP 服务器。点击右上角"添加服务器"开始。
                    </div>
                }>
                    {(cfg) => {
                        const status = () => mcpServerStatus()[cfg.id]?.status ?? 'disconnected';
                        const isConnected = () => status() === 'connected';
                        const isConnecting = () => status() === 'connecting';
                        return (
                            <div
                                class="flex items-center justify-between px-4 py-3 rounded-lg transition-colors"
                                style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,255,255,0.08);"
                            >
                                <div class="flex flex-col gap-1 flex-1 min-w-0">
                                    <div class="flex items-center gap-2">
                                        <span class="font-medium truncate">{cfg.displayName || cfg.id}</span>
                                        <span
                                            class="px-1.5 py-0.5 rounded text-[10px] uppercase tracking-wide"
                                            style={`background: ${statusColor(status())}22; color: ${statusColor(status())};`}
                                        >
                                            {statusLabel(status())}
                                        </span>
                                        <Show when={isConnected()}>
                                            <span class="text-xs" style="color: rgba(255,255,255,0.5);">
                                                ({mcpServerStatus()[cfg.id]?.toolCount ?? 0} 工具)
                                            </span>
                                        </Show>
                                    </div>
                                    <div class="text-xs truncate" style="color: rgba(255,255,255,0.5);">
                                        {transportLabel(cfg.transport)}
                                    </div>
                                </div>
                                <div class="flex items-center gap-2 flex-shrink-0">
                                    <Show when={!isConnected() && !isConnecting()}>
                                        <button
                                            class="px-2 py-1 rounded text-xs cursor-pointer transition-colors"
                                            style="background: rgba(124,154,191,0.2); border: 1px solid rgba(124,154,191,0.3);"
                                            onClick={() => handleStart(cfg.id)}
                                        >
                                            启动
                                        </button>
                                    </Show>
                                    <Show when={isConnected()}>
                                        <button
                                            class="px-2 py-1 rounded text-xs cursor-pointer transition-colors"
                                            style="background: rgba(255,255,255,0.05); border: 1px solid rgba(255,255,255,0.1);"
                                            onClick={() => handleStop(cfg.id)}
                                        >
                                            停止
                                        </button>
                                    </Show>
                                    <button
                                        class="px-2 py-1 rounded text-xs cursor-pointer transition-colors"
                                        style="background: rgba(255,255,255,0.05); border: 1px solid rgba(255,255,255,0.1);"
                                        onClick={() => handleEdit(cfg)}
                                    >
                                        编辑
                                    </button>
                                    <button
                                        class="px-2 py-1 rounded text-xs cursor-pointer transition-colors"
                                        style="background: rgba(255,77,77,0.1); border: 1px solid rgba(255,77,77,0.3); color: rgba(255,107,107,0.9);"
                                        onClick={() => handleRemove(cfg.id)}
                                    >
                                        删除
                                    </button>
                                </div>
                            </div>
                        );
                    }}
                </For>
            </div>

            <Show when={editingConfig()}>
                <McpServerDetail
                    config={editingConfig()!}
                    isNew={isCreating()}
                    onSave={handleSave}
                    onCancel={() => { setEditingConfig(null); setIsCreating(false); }}
                    onTest={handleTest}
                />
            </Show>
        </div>
    );
};

export default McpServerList;
