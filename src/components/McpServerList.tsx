import { Component, For, Show, createMemo, createSignal, onMount } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import {
    datas, mcpServers, mcpServerStatus, saveSingleAssistantToBackend, setDatas,
    setMcpServers, setMcpServerStatus, startMcpServerAndRefresh,
} from '../store/store';
import type {
    McpCatalogDelivery, McpCatalogInstallRequest, McpCatalogPage,
    McpCatalogServer, McpServerConfig,
} from '../types/mcp';
import { emptyMcpServerConfig, statusColor, statusLabel, transportLabel } from '../utils/mcp';
import McpServerDetail from './McpServerDetail';

type ViewMode = 'market' | 'downloaded';

const McpServerList: Component = () => {
    const [view, setView] = createSignal<ViewMode>('market');
    const [catalog, setCatalog] = createSignal<McpCatalogServer[]>([]);
    const [nextCursor, setNextCursor] = createSignal<string | undefined>();
    const [query, setQuery] = createSignal('');
    const [loading, setLoading] = createSignal(false);
    const [loadingMore, setLoadingMore] = createSignal(false);
    const [refreshing, setRefreshing] = createSignal(false);
    const [editingConfig, setEditingConfig] = createSignal<McpServerConfig | null>(null);
    const [isCreating, setIsCreating] = createSignal(false);
    const [installing, setInstalling] = createSignal<McpCatalogServer | null>(null);
    const [selectedDeliveryId, setSelectedDeliveryId] = createSignal('');
    const [installValues, setInstallValues] = createSignal<Record<string, string>>({});
    const [runtimeAvailable, setRuntimeAvailable] = createSignal(true);
    const [installingId, setInstallingId] = createSignal<string | null>(null);
    const [error, setError] = createSignal<string | null>(null);

    const loadLocal = async () => {
        const list = await invoke<McpServerConfig[]>('list_mcp_servers');
        setMcpServers(Object.fromEntries(list.map(server => [server.id, server])));
    };

    const loadCatalog = async (forceRefresh = false, cursor?: string) => {
        cursor ? setLoadingMore(true) : setLoading(true);
        setError(null);
        try {
            const page = await invoke<McpCatalogPage>('list_mcp_catalog', {
                search: query().trim(),
                cursor: cursor ?? null,
                limit: 30,
                forceRefresh,
            });
            setCatalog(cursor ? [...catalog(), ...page.servers] : page.servers);
            setNextCursor(page.nextCursor);
        } catch (e) {
            setError(`加载 MCP Registry 失败: ${e}`);
        } finally {
            setLoading(false);
            setLoadingMore(false);
        }
    };

    onMount(async () => {
        try {
            await Promise.all([loadLocal(), loadCatalog()]);
        } catch (e) {
            setError(String(e));
        }
    });

    const sortedServers = createMemo(() =>
        Object.values(mcpServers()).sort((a, b) => a.displayName.localeCompare(b.displayName)),
    );
    const installedSourceIds = createMemo(() =>
        new Set(Object.values(mcpServers()).map(server => server.fromCatalog?.sourceId).filter(Boolean)),
    );
    const filteredLocal = createMemo(() => {
        const keyword = query().trim().toLowerCase();
        if (!keyword) return sortedServers();
        return sortedServers().filter(server =>
            `${server.displayName} ${server.fromCatalog?.sourceId ?? ''} ${transportLabel(server.transport)}`
                .toLowerCase().includes(keyword),
        );
    });
    const selectedDelivery = createMemo(() =>
        installing()?.deliveries.find(delivery => delivery.id === selectedDeliveryId()),
    );

    const search = () => {
        if (view() === 'market') void loadCatalog();
    };

    const refresh = async () => {
        setRefreshing(true);
        await loadCatalog(true);
        setRefreshing(false);
    };

    const handleStart = async (id: string) => {
        try {
            await startMcpServerAndRefresh(id);
        } catch (e) {
            setError(`启动失败: ${e}`);
        }
    };

    const handleStop = async (id: string) => {
        try {
            await invoke('stop_mcp_server', { id });
            setMcpServerStatus({ ...mcpServerStatus(), [id]: { id, status: 'disconnected', toolCount: 0 } });
        } catch (e) {
            setError(`停止失败: ${e}`);
        }
    };

    const handleRemove = async (id: string) => {
        if (!confirm('确定删除此 MCP 服务器配置？此操作不可恢复。')) return;
        try {
            await invoke('remove_mcp_server', { id });
            const next = { ...mcpServers() };
            delete next[id];
            setMcpServers(next);
            const affected = datas.assistants.filter(a => a.mcpServerIds?.includes(id)).map(a => a.id);
            for (const assistantId of affected) {
                setDatas(
                    'assistants',
                    assistant => assistant.id === assistantId,
                    'mcpServerIds',
                    (ids: string[] | undefined) => (ids ?? []).filter(serverId => serverId !== id),
                );
            }
            await Promise.all(affected.map(saveSingleAssistantToBackend));
        } catch (e) {
            setError(`删除失败: ${e}`);
        }
    };

    const handleSave = async (config: McpServerConfig) => {
        try {
            await invoke('add_mcp_server', { config });
            setMcpServers({ ...mcpServers(), [config.id]: config });
            setEditingConfig(null);
            setIsCreating(false);
        } catch (e) {
            setError(`保存失败: ${e}`);
        }
    };

    const handleTest = async (config: McpServerConfig) => {
        try {
            const tools = await invoke<any[]>('test_mcp_server_connection', { config });
            return { ok: true, tools };
        } catch (e) {
            return { ok: false, error: String(e) };
        }
    };

    const chooseDelivery = async (delivery: McpCatalogDelivery) => {
        setSelectedDeliveryId(delivery.id);
        setInstallValues(Object.fromEntries(
            delivery.inputs.map(input => [input.name, input.defaultValue ?? '']),
        ));
        const available = await invoke<boolean>('check_mcp_catalog_runtime', {
            packageType: delivery.kind,
        }).catch(() => false);
        setRuntimeAvailable(available);
    };

    const openInstall = (server: McpCatalogServer) => {
        setInstalling(server);
        const preferred = server.deliveries.find(d => d.kind === 'http')
            ?? server.deliveries.find(d => d.kind === 'npm')
            ?? server.deliveries[0];
        if (preferred) void chooseDelivery(preferred);
    };

    const install = async () => {
        const server = installing();
        const delivery = selectedDelivery();
        if (!server || !delivery) return;
        const missing = delivery.inputs.find(input => input.required && !installValues()[input.name]?.trim());
        if (missing) {
            setError(`请填写必填配置：${missing.name}`);
            return;
        }
        setInstallingId(server.id);
        setError(null);
        try {
            const values: Record<string, string> = {};
            const secrets: Record<string, string> = {};
            for (const input of delivery.inputs) {
                const value = installValues()[input.name] ?? '';
                (input.secret ? secrets : values)[input.name] = value;
            }
            const request: McpCatalogInstallRequest = {
                server,
                deliveryId: delivery.id,
                values,
                secrets,
            };
            const config = await invoke<McpServerConfig>('install_mcp_catalog_server', { request });
            setMcpServers({ ...mcpServers(), [config.id]: config });
            setInstalling(null);
            setView('downloaded');
            try {
                await startMcpServerAndRefresh(config.id);
            } catch (e) {
                setError(`已安装，但首次启动失败：${e}`);
            }
        } catch (e) {
            setError(`安装失败: ${e}`);
        } finally {
            setInstallingId(null);
        }
    };

    return (
        <div class="flex flex-col h-full overflow-hidden p-6 gap-4" style="color: rgba(255,255,255,0.88);">
            <div class="flex items-start justify-between gap-4">
                <div>
                    <h2 class="text-xl font-semibold">MCP 服务器</h2>
                    <p class="text-xs mt-1" style="color: rgba(255,255,255,0.5);">
                        浏览 Official MCP Registry，安装后可在各助手设置中分别启用。
                    </p>
                </div>
                <button
                    class="px-3 py-1.5 rounded-md text-sm"
                    style="background: rgba(124,154,191,0.2); border: 1px solid rgba(124,154,191,0.3);"
                    onClick={() => { setIsCreating(true); setEditingConfig(emptyMcpServerConfig()); }}
                >
                    + 手动添加
                </button>
            </div>

            <div class="flex items-center justify-between gap-3 flex-wrap">
                <div class="flex items-center gap-1 p-1 rounded-lg" style="background: rgba(255,255,255,0.04);">
                    <button class="px-3 py-1.5 rounded-md text-sm"
                        classList={{ 'bg-pri-20 text-pri': view() === 'market' }}
                        onClick={() => setView('market')}>商店</button>
                    <button class="px-3 py-1.5 rounded-md text-sm"
                        classList={{ 'bg-pri-20 text-pri': view() === 'downloaded' }}
                        onClick={() => setView('downloaded')}>已下载 ({sortedServers().length})</button>
                </div>
                <div class="flex gap-2">
                    <input
                        class="w-[280px] max-w-full px-3 py-2 rounded-lg text-sm outline-none"
                        style="background: rgba(0,0,0,0.25); border: 1px solid rgba(255,255,255,0.1);"
                        value={query()}
                        onInput={event => setQuery(event.currentTarget.value)}
                        onKeyDown={event => event.key === 'Enter' && search()}
                        placeholder="搜索 MCP 服务器"
                    />
                    <Show when={view() === 'market'}>
                        <button class="px-3 py-2 rounded-lg text-xs"
                            style="background: rgba(255,255,255,0.05); border: 1px solid rgba(255,255,255,0.1);"
                            onClick={search}>搜索</button>
                        <button class="px-3 py-2 rounded-lg text-xs"
                            disabled={refreshing()}
                            style="background: rgba(255,255,255,0.05); border: 1px solid rgba(255,255,255,0.1);"
                            onClick={() => void refresh()}>{refreshing() ? '更新中…' : '刷新'}</button>
                    </Show>
                </div>
            </div>

            <Show when={error()}>
                <div class="px-3 py-2 rounded-md text-sm" style="background: rgba(255,77,77,0.1); color: #ff8a8a;">
                    {error()} <button class="ml-3 underline" onClick={() => setError(null)}>关闭</button>
                </div>
            </Show>

            <div class="flex-1 overflow-y-auto min-h-0">
                <Show when={loading()}>
                    <div class="h-full flex items-center justify-center text-sm" style="color: rgba(255,255,255,0.45);">
                        正在加载 MCP Registry…
                    </div>
                </Show>

                <Show when={!loading() && view() === 'market'}>
                    <div class="grid grid-cols-1 xl:grid-cols-2 gap-3">
                        <For each={catalog()} fallback={
                            <div class="col-span-full py-12 text-center text-sm" style="color: rgba(255,255,255,0.4);">
                                没有匹配的 MCP 服务器。
                            </div>
                        }>
                            {(server) => {
                                const installed = () => installedSourceIds().has(server.name);
                                return (
                                    <div class="flex flex-col gap-3 rounded-xl p-4"
                                        style="background: rgba(255,255,255,0.035); border: 1px solid rgba(255,255,255,0.08);">
                                        <div class="flex items-start justify-between gap-3">
                                            <div class="min-w-0">
                                                <div class="font-semibold truncate">{server.displayName}</div>
                                                <div class="text-[11px] font-mono truncate mt-1" style="color: rgba(124,154,191,0.85);">
                                                    {server.name}
                                                </div>
                                            </div>
                                            <Show when={server.version}>
                                                <span class="text-[10px] px-2 py-1 rounded shrink-0" style="background: rgba(255,255,255,0.06);">
                                                    v{server.version}
                                                </span>
                                            </Show>
                                        </div>
                                        <p class="text-xs leading-relaxed line-clamp-3 min-h-[3rem]" style="color: rgba(255,255,255,0.55);">
                                            {server.description || '该服务器暂无简介。'}
                                        </p>
                                        <div class="flex items-center gap-1 flex-wrap">
                                            <For each={server.deliveries}>
                                                {delivery => <span class="text-[10px] px-1.5 py-0.5 rounded bg-pri-10 text-pri">{delivery.label}</span>}
                                            </For>
                                        </div>
                                        <div class="flex items-center justify-between gap-3">
                                            <button class="text-[11px] hover:underline" style="color: rgba(255,255,255,0.45);"
                                                disabled={!server.repositoryUrl && !server.websiteUrl}
                                                onClick={() => void openUrl(server.repositoryUrl || server.websiteUrl)}>
                                                Official MCP Registry
                                            </button>
                                            <button
                                                class="px-3 py-1.5 rounded-md text-xs"
                                                disabled={installed() || installingId() === server.id}
                                                style={installed()
                                                    ? 'background: rgba(124,217,160,0.12); color: #7cd9a0;'
                                                    : 'background: rgba(124,154,191,0.2); border: 1px solid rgba(124,154,191,0.3);'}
                                                onClick={() => openInstall(server)}
                                            >
                                                {installed() ? '已安装' : installingId() === server.id ? '安装中…' : '安装'}
                                            </button>
                                        </div>
                                    </div>
                                );
                            }}
                        </For>
                    </div>
                    <Show when={nextCursor()}>
                        <div class="flex justify-center py-5">
                            <button class="px-4 py-2 rounded text-xs"
                                disabled={loadingMore()}
                                style="background: rgba(255,255,255,0.05); border: 1px solid rgba(255,255,255,0.1);"
                                onClick={() => void loadCatalog(false, nextCursor())}>
                                {loadingMore() ? '加载中…' : '加载更多'}
                            </button>
                        </div>
                    </Show>
                </Show>

                <Show when={view() === 'downloaded'}>
                    <div class="flex flex-col gap-2">
                        <For each={filteredLocal()} fallback={
                            <div class="py-12 text-center text-sm" style="color: rgba(255,255,255,0.4);">尚未安装 MCP 服务器。</div>
                        }>
                            {(config) => {
                                const status = () => mcpServerStatus()[config.id]?.status ?? 'disconnected';
                                return (
                                    <div class="flex items-center justify-between px-4 py-3 rounded-lg"
                                        style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,255,255,0.08);">
                                        <div class="flex-1 min-w-0">
                                            <div class="flex items-center gap-2">
                                                <span class="font-medium truncate">{config.displayName || config.id}</span>
                                                <span class="px-1.5 py-0.5 rounded text-[10px]"
                                                    style={`background: ${statusColor(status())}22; color: ${statusColor(status())};`}>
                                                    {statusLabel(status())}
                                                </span>
                                                <Show when={config.fromCatalog}>
                                                    <span class="text-[10px] px-1.5 py-0.5 rounded bg-pri-10 text-pri">Registry</span>
                                                </Show>
                                            </div>
                                            <div class="text-xs truncate mt-1" style="color: rgba(255,255,255,0.5);">
                                                {transportLabel(config.transport)}
                                                {status() === 'connected' ? ` · ${mcpServerStatus()[config.id]?.toolCount ?? 0} 工具` : ''}
                                            </div>
                                        </div>
                                        <div class="flex items-center gap-2 ml-3">
                                            <Show when={status() !== 'connected' && status() !== 'connecting'}>
                                                <button class="px-2 py-1 rounded text-xs" style="background: rgba(124,154,191,0.2);"
                                                    onClick={() => void handleStart(config.id)}>启动</button>
                                            </Show>
                                            <Show when={status() === 'connected'}>
                                                <button class="px-2 py-1 rounded text-xs" style="background: rgba(255,255,255,0.05);"
                                                    onClick={() => void handleStop(config.id)}>停止</button>
                                            </Show>
                                            <button class="px-2 py-1 rounded text-xs" style="background: rgba(255,255,255,0.05);"
                                                onClick={() => { setIsCreating(false); setEditingConfig({ ...config }); }}>编辑</button>
                                            <button class="px-2 py-1 rounded text-xs"
                                                style="background: rgba(255,77,77,0.1); color: rgba(255,107,107,0.9);"
                                                onClick={() => void handleRemove(config.id)}>删除</button>
                                        </div>
                                    </div>
                                );
                            }}
                        </For>
                    </div>
                </Show>
            </div>

            <Show when={installing()}>
                <div class="fixed inset-0 z-50 flex items-center justify-center p-6"
                    style="background: rgba(0,0,0,0.6); backdrop-filter: blur(8px);"
                    onClick={event => event.target === event.currentTarget && setInstalling(null)}>
                    <div class="w-[620px] max-w-full max-h-[90vh] overflow-y-auto rounded-xl p-6 flex flex-col gap-4"
                        style="background: rgba(18,22,35,0.98); border: 1px solid rgba(255,255,255,0.1);">
                        <div>
                            <h3 class="font-semibold">安装 {installing()!.displayName}</h3>
                            <p class="text-xs mt-1" style="color: rgba(255,255,255,0.45);">
                                本地 MCP 包会以当前用户权限运行，请确认其仓库和配置可信。
                            </p>
                        </div>
                        <label class="flex flex-col gap-1 text-xs">
                            安装方式
                            <select class="px-3 py-2 rounded text-sm"
                                style="background: rgba(22,26,40,0.95); border: 1px solid rgba(255,255,255,0.1);"
                                value={selectedDeliveryId()}
                                onChange={event => {
                                    const delivery = installing()!.deliveries.find(item => item.id === event.currentTarget.value);
                                    if (delivery) void chooseDelivery(delivery);
                                }}>
                                <For each={installing()!.deliveries}>
                                    {delivery => <option value={delivery.id}>{delivery.label}</option>}
                                </For>
                            </select>
                        </label>
                        <Show when={selectedDelivery()?.kind !== 'http' && !runtimeAvailable()}>
                            <div class="px-3 py-2 rounded text-xs" style="background: rgba(255,180,77,0.12); color: #ffd080;">
                                未检测到 {selectedDelivery()?.kind === 'npm' ? 'npx（请安装 Node.js）' : 'uvx（请安装 uv）'}，安装后将无法启动。
                            </div>
                        </Show>
                        <Show when={selectedDelivery()}>
                            <div class="px-3 py-2 rounded font-mono text-xs break-all"
                                style="background: rgba(0,0,0,0.25); color: rgba(255,255,255,0.6);">
                                {selectedDelivery()!.kind === 'http'
                                    ? selectedDelivery()!.url
                                    : `${selectedDelivery()!.command} ${selectedDelivery()!.args.join(' ')}`}
                            </div>
                            <For each={selectedDelivery()!.inputs}>
                                {input => (
                                    <label class="flex flex-col gap-1 text-xs">
                                        <span>{input.name}{input.required ? ' *' : ''}</span>
                                        <input
                                            type={input.secret ? 'password' : 'text'}
                                            class="px-3 py-2 rounded text-sm outline-none"
                                            style="background: rgba(0,0,0,0.3); border: 1px solid rgba(255,255,255,0.1);"
                                            value={installValues()[input.name] ?? ''}
                                            placeholder={input.description || input.defaultValue}
                                            onInput={event => setInstallValues({
                                                ...installValues(),
                                                [input.name]: event.currentTarget.value,
                                            })}
                                        />
                                    </label>
                                )}
                            </For>
                        </Show>
                        <div class="flex justify-end gap-2">
                            <button class="px-3 py-1.5 rounded text-sm" style="background: rgba(255,255,255,0.05);"
                                onClick={() => setInstalling(null)}>取消</button>
                            <button class="px-3 py-1.5 rounded text-sm bg-pri text-black"
                                disabled={installingId() !== null || !runtimeAvailable()}
                                onClick={() => void install()}>
                                {installingId() ? '安装中…' : '安装并启动'}
                            </button>
                        </div>
                    </div>
                </div>
            </Show>

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
