import { Component, createSignal, For, Show, createMemo, createEffect } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import type { McpServerConfig, McpTransport, ToolSpec } from '../types/mcp';

interface Props {
    config: McpServerConfig;
    isNew: boolean;
    onSave: (cfg: McpServerConfig) => void;
    onCancel: () => void;
    onTest: (cfg: McpServerConfig) => Promise<{ ok: boolean; tools?: ToolSpec[]; error?: string }>;
}

const McpServerDetail: Component<Props> = (props) => {
    const [config, setConfig] = createSignal<McpServerConfig>(JSON.parse(JSON.stringify(props.config)));
    const [testResult, setTestResult] = createSignal<{ ok: boolean; tools?: ToolSpec[]; error?: string } | null>(null);
    const [testing, setTesting] = createSignal(false);
    const [availableTools, setAvailableTools] = createSignal<ToolSpec[]>([]);

    // 加载已注册 transports
    const [transports, setTransports] = createSignal<string[]>([]);
    createEffect(async () => {
        try {
            const t = await invoke<string[]>('list_mcp_transports');
            setTransports(t);
        } catch { /* ignore */ }
    });

    const updateField = <K extends keyof McpServerConfig>(key: K, value: McpServerConfig[K]) => {
        setConfig({ ...config(), [key]: value });
    };

    const updateTransport = (t: McpTransport) => {
        setConfig({ ...config(), transport: t });
    };

    const updateStdioArg = (idx: number, value: string) => {
        const t = config().transport;
        if (t.transport !== 'stdio') return;
        const args = [...t.args];
        args[idx] = value;
        updateTransport({ ...t, args });
    };

    const addStdioArg = () => {
        const t = config().transport;
        if (t.transport !== 'stdio') return;
        updateTransport({ ...t, args: [...t.args, ''] });
    };

    const removeStdioArg = (idx: number) => {
        const t = config().transport;
        if (t.transport !== 'stdio') return;
        updateTransport({ ...t, args: t.args.filter((_, i) => i !== idx) });
    };

    const addEnvEntry = () => {
        const t = config().transport;
        if (t.transport !== 'stdio') return;
        const key = `VAR_${Object.keys(t.env).length + 1}`;
        updateTransport({ ...t, env: { ...t.env, [key]: '' } });
    };

    const updateEnvEntry = (key: string, value: string) => {
        const t = config().transport;
        if (t.transport !== 'stdio') return;
        const newEnv = { ...t.env, [key]: value };
        updateTransport({ ...t, env: newEnv });
    };

    const storeEnvSecret = async (key: string, value: string) => {
        if (!value || value.includes('${KEYRING:')) return;
        const placeholder = await invoke<string>('save_mcp_server_secret', {
            serverId: config().id,
            target: 'env',
            key,
            value,
        });
        const t = config().transport;
        if (t.transport !== 'stdio') return;
        updateTransport({ ...t, env: { ...t.env, [key]: placeholder } });
        updateField('hasStoredSecret', true);
    };

    const removeEnvEntry = (key: string) => {
        const t = config().transport;
        if (t.transport !== 'stdio') return;
        const newEnv = { ...t.env };
        delete newEnv[key];
        updateTransport({ ...t, env: newEnv });
    };

    const handleTest = async () => {
        setTesting(true);
        setTestResult(null);
        const r = await props.onTest(config());
        setTestResult(r);
        if (r.ok && r.tools) {
            setAvailableTools(r.tools);
        }
        setTesting(false);
    };

    const toggleTool = (name: string) => {
        const cur = config().enabledTools;
        const next = cur.includes(name) ? cur.filter(n => n !== name) : [...cur, name];
        updateField('enabledTools', next);
    };

    const selectAllTools = () => {
        updateField('enabledTools', availableTools().map(t => t.function.name));
    };
    const deselectAllTools = () => {
        updateField('enabledTools', []);
    };

    return (
        <div
            class="fixed inset-0 z-50 flex items-center justify-center p-6"
            style="background: rgba(0,0,0,0.6); backdrop-filter: blur(8px);"
            onClick={(e) => { if (e.target === e.currentTarget) props.onCancel(); }}
        >
            <div
                class="w-[640px] max-w-full max-h-[90vh] overflow-y-auto rounded-xl p-6 flex flex-col gap-4"
                style="background: rgba(18,22,35,0.95); border: 1px solid rgba(255,255,255,0.1);"
            >
                <h3 class="text-base font-semibold">{props.isNew ? '添加 MCP 服务器' : '编辑 MCP 服务器'}</h3>

                {/* 名称 + 启用 */}
                <div class="flex flex-col gap-1">
                    <label class="text-xs" style="color: rgba(255,255,255,0.6);">名称</label>
                    <input
                        class="px-3 py-1.5 rounded text-sm outline-none"
                        style="background: rgba(0,0,0,0.3); border: 1px solid rgba(255,255,255,0.1); color: white;"
                        value={config().displayName}
                        onInput={(e) => updateField('displayName', e.currentTarget.value)}
                        placeholder="本地文件系统"
                    />
                </div>
                <div class="flex items-center gap-3">
                    <label class="flex items-center gap-2 text-sm cursor-pointer">
                        <input
                            type="checkbox"
                            checked={config().autoStart}
                            onChange={(e) => updateField('autoStart', e.currentTarget.checked)}
                        />
                        应用启动时自动连接
                    </label>
                    <span class="text-xs" style="color: rgba(255,255,255,0.4);">
                        （是否被助手使用由各助手设置中的「MCP 服务器」勾选项决定）
                    </span>
                </div>

                {/* 传输类型 */}
                <div class="flex flex-col gap-1">
                    <label class="text-xs" style="color: rgba(255,255,255,0.6);">传输类型</label>
                    <select
                        class="px-3 py-1.5 rounded text-sm outline-none"
                        style="background: rgba(0,0,0,0.3); border: 1px solid rgba(255,255,255,0.1); color: white;"
                        value={config().transport.transport}
                        onChange={(e) => {
                            const t = e.currentTarget.value as 'stdio' | 'http' | 'streamable_http';
                            if (t === 'stdio') {
                                updateTransport({ transport: 'stdio', command: 'npx', args: [], env: {} });
                            } else if (t === 'http') {
                                updateTransport({ transport: 'http', url: '', headers: {} });
                            } else {
                                updateTransport({ transport: 'streamable_http', url: '', headers: {} });
                            }
                        }}
                    >
                        <Show when={transports().includes('stdio')}>
                            <option value="stdio">stdio（本地子进程）</option>
                        </Show>
                        <Show when={transports().includes('http')}>
                            <option value="http">HTTP+SSE（远程）</option>
                        </Show>
                    </select>
                </div>

                {/* stdio 字段 */}
                <Show when={config().transport.transport === 'stdio'}>
                    {(() => {
                        const t = () => config().transport as Extract<McpTransport, { transport: 'stdio' }>;
                        return (
                            <>
                                <div class="flex flex-col gap-1">
                                    <label class="text-xs" style="color: rgba(255,255,255,0.6);">命令 (command)</label>
                                    <input
                                        class="px-3 py-1.5 rounded text-sm outline-none"
                                        style="background: rgba(0,0,0,0.3); border: 1px solid rgba(255,255,255,0.1); color: white;"
                                        value={t().command}
                                        onInput={(e) => updateTransport({ ...t(), command: e.currentTarget.value })}
                                        placeholder="npx 或 /usr/local/bin/python"
                                    />
                                </div>
                                <div class="flex flex-col gap-1">
                                    <label class="text-xs flex items-center justify-between" style="color: rgba(255,255,255,0.6);">
                                        参数 (args)
                                        <button class="text-xs px-2 py-0.5 rounded" style="background: rgba(124,154,191,0.2);" onClick={addStdioArg}>+ 添加</button>
                                    </label>
                                    <For each={t().args}>
                                        {(arg, idx) => (
                                            <div class="flex items-center gap-1">
                                                <input
                                                    class="flex-1 px-3 py-1.5 rounded text-sm outline-none"
                                                    style="background: rgba(0,0,0,0.3); border: 1px solid rgba(255,255,255,0.1); color: white;"
                                                    value={arg}
                                                    onInput={(e) => updateStdioArg(idx(), e.currentTarget.value)}
                                                />
                                                <button
                                                    class="px-2 py-1 rounded text-xs"
                                                    style="background: rgba(255,77,77,0.1); color: rgba(255,107,107,0.9);"
                                                    onClick={() => removeStdioArg(idx())}
                                                >×</button>
                                            </div>
                                        )}
                                    </For>
                                </div>
                                <div class="flex flex-col gap-1">
                                    <label class="text-xs flex items-center justify-between" style="color: rgba(255,255,255,0.6);">
                                        环境变量 (env)
                                        <button class="text-xs px-2 py-0.5 rounded" style="background: rgba(124,154,191,0.2);" onClick={addEnvEntry}>+ 添加</button>
                                    </label>
                                    <For each={Object.entries(t().env)}>
                                        {([k, v]) => (
                                            <div class="flex items-center gap-1">
                                                <input
                                                    class="w-1/3 px-2 py-1.5 rounded text-sm outline-none"
                                                    style="background: rgba(0,0,0,0.3); border: 1px solid rgba(255,255,255,0.1); color: white;"
                                                    value={k}
                                                    readonly
                                                />
                                                <input
                                                    class="flex-1 px-2 py-1.5 rounded text-sm outline-none"
                                                    style="background: rgba(0,0,0,0.3); border: 1px solid rgba(255,255,255,0.1); color: white;"
                                                    value={v.includes('${KEYRING:') ? '[密钥·已存keyring]' : v}
                                                    onInput={(e) => updateEnvEntry(k, e.currentTarget.value)}
                                                    placeholder="值"
                                                />
                                                <button
                                                    class="px-2 py-1 rounded text-xs whitespace-nowrap"
                                                    style="background: rgba(255,180,77,0.15); color: rgba(255,200,120,0.95);"
                                                    title="存到系统钥匙串（不再显示在 UI 中）"
                                                    onClick={() => void storeEnvSecret(k, v)}
                                                >存密钥</button>
                                                <button
                                                    class="px-2 py-1 rounded text-xs"
                                                    style="background: rgba(255,77,77,0.1); color: rgba(255,107,107,0.9);"
                                                    onClick={() => removeEnvEntry(k)}
                                                >×</button>
                                            </div>
                                        )}
                                    </For>
                                </div>
                            </>
                        );
                    })()}
                </Show>

                {/* http 字段 */}
                <Show when={config().transport.transport === 'http' || config().transport.transport === 'streamable_http'}>
                    {(() => {
                        const t = () => config().transport as Extract<McpTransport, { transport: 'http' | 'streamable_http' }>;
                        return (
                            <div class="flex flex-col gap-1">
                                <label class="text-xs" style="color: rgba(255,255,255,0.6);">URL</label>
                                <input
                                    class="px-3 py-1.5 rounded text-sm outline-none"
                                    style="background: rgba(0,0,0,0.3); border: 1px solid rgba(255,255,255,0.1); color: white;"
                                    value={t().url}
                                    onInput={(e) => updateTransport({ ...t(), url: e.currentTarget.value })}
                                    placeholder="https://mcp.example.com/sse"
                                />
                                <Show when={!t().url.startsWith('http://') && !t().url.startsWith('https://') && t().url !== ''}>
                                    <span class="text-xs" style="color: #ff8a8a;">URL 必须以 http:// 或 https:// 开头</span>
                                </Show>
                            </div>
                        );
                    })()}
                </Show>

                {/* 测试连接 */}
                <div class="flex items-center gap-2">
                    <button
                        class="px-3 py-1.5 rounded text-sm cursor-pointer transition-colors"
                        style="background: rgba(124,154,191,0.2); border: 1px solid rgba(124,154,191,0.3);"
                        disabled={testing()}
                        onClick={handleTest}
                    >
                        {testing() ? '测试中…' : '测试连接'}
                    </button>
                    <Show when={testResult()}>
                        <span
                            class="text-xs px-2 py-1 rounded"
                            style={testResult()!.ok
                                ? "background: rgba(124,217,160,0.1); color: #7cd9a0;"
                                : "background: rgba(255,77,77,0.1); color: #ff8a8a;"}
                        >
                            {testResult()!.ok
                                ? `✓ 成功，${testResult()!.tools?.length ?? 0} 个工具`
                                : `✗ ${testResult()!.error}`}
                        </span>
                    </Show>
                </div>

                {/* 工具白名单 */}
                <Show when={availableTools().length > 0}>
                    <div class="flex flex-col gap-2">
                        <div class="flex items-center justify-between">
                            <label class="text-xs" style="color: rgba(255,255,255,0.6);">
                                工具白名单（{config().enabledTools.length === 0 ? '全部启用' : `已选 ${config().enabledTools.length}/${availableTools().length}`}）
                            </label>
                            <div class="flex gap-2">
                                <button class="text-xs px-2 py-0.5 rounded" style="background: rgba(124,154,191,0.2);" onClick={selectAllTools}>全选</button>
                                <button class="text-xs px-2 py-0.5 rounded" style="background: rgba(255,255,255,0.05);" onClick={deselectAllTools}>清空</button>
                            </div>
                        </div>
                        <div class="flex flex-col gap-1 max-h-40 overflow-y-auto">
                            <For each={availableTools()}>
                                {(tool) => (
                                    <label class="flex items-start gap-2 text-xs px-2 py-1 rounded cursor-pointer" style="background: rgba(255,255,255,0.03);">
                                        <input
                                            type="checkbox"
                                            checked={config().enabledTools.length === 0 || config().enabledTools.includes(tool.function.name)}
                                            onChange={() => toggleTool(tool.function.name)}
                                            class="mt-0.5"
                                        />
                                        <div class="flex-1 min-w-0">
                                            <div class="font-mono" style="color: rgba(255,255,255,0.85);">{tool.function.name}</div>
                                            <div style="color: rgba(255,255,255,0.5);">{tool.function.description}</div>
                                        </div>
                                    </label>
                                )}
                            </For>
                        </div>
                    </div>
                </Show>

                {/* 按钮 */}
                <div class="flex justify-end gap-2 mt-2">
                    <button
                        class="px-3 py-1.5 rounded text-sm cursor-pointer"
                        style="background: rgba(255,255,255,0.05); border: 1px solid rgba(255,255,255,0.1);"
                        onClick={props.onCancel}
                    >
                        取消
                    </button>
                    <button
                        class="px-3 py-1.5 rounded text-sm cursor-pointer"
                        style="background: rgba(124,217,160,0.2); border: 1px solid rgba(124,217,160,0.3);"
                        onClick={() => props.onSave(config())}
                    >
                        保存
                    </button>
                </div>
            </div>
        </div>
    );
};

export default McpServerDetail;
