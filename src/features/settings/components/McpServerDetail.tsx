import { Component, createSignal, For, Show, onMount, untrack } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import type { McpServerConfig, McpTransport, ToolSpec } from '../../../core/types/mcp';
import { t } from '../../../core/i18n';

interface Props {
  config: McpServerConfig;
  isNew: boolean;
  onSave: (cfg: McpServerConfig) => void;
  onCancel: () => void;
  onTest: (cfg: McpServerConfig) => Promise<{ ok: boolean; tools?: ToolSpec[]; error?: string }>;
}

const McpServerDetail: Component<Props> = (props) => {
  const [config, setConfig] = createSignal<McpServerConfig>(
    untrack(() => JSON.parse(JSON.stringify(props.config))),
  );
  const [testResult, setTestResult] = createSignal<{
    ok: boolean;
    tools?: ToolSpec[];
    error?: string;
  } | null>(null);
  const [testing, setTesting] = createSignal(false);
  const [availableTools, setAvailableTools] = createSignal<ToolSpec[]>([]);

  // 加载已注册 transports
  const [transports, setTransports] = createSignal<string[]>([]);
  onMount(() => {
    void invoke<string[]>('list_mcp_transports')
      .then(setTransports)
      .catch(() => {
        // Keep the built-in transport choices when discovery is unavailable.
      });
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
    const next = cur.includes(name) ? cur.filter((n) => n !== name) : [...cur, name];
    updateField('enabledTools', next);
  };

  const selectAllTools = () => {
    updateField(
      'enabledTools',
      availableTools().map((t) => t.function.name),
    );
  };
  const deselectAllTools = () => {
    updateField('enabledTools', []);
  };

  return (
    <div
      class="fixed inset-0 z-50 flex items-center justify-center p-6"
      style={{ background: 'rgba(0,0,0,0.6)', 'backdrop-filter': 'blur(8px)' }}
      onClick={(e) => {
        if (e.target === e.currentTarget) props.onCancel();
      }}
    >
      <div
        class="w-[640px] max-w-full max-h-[90vh] overflow-y-auto rounded-xl p-6 flex flex-col gap-4"
        style={{
          background: 'rgba(var(--surface-bg),0.95)',
          border: '1px solid var(--border-dim)',
        }}
      >
        <h3 class="text-base font-semibold">
          {props.isNew ? t('mcp.detail.titleAdd') : t('mcp.detail.titleEdit')}
        </h3>

        {/* 名称 + 启用 */}
        <div class="flex flex-col gap-1">
          <label class="text-xs" style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}>
            {t('mcp.detail.name')}
          </label>
          <input
            class="px-3 py-1.5 rounded text-sm outline-none"
            style={{
              background: 'rgba(0,0,0,0.3)',
              border: '1px solid var(--border-dim)',
              color: 'white',
            }}
            value={config().displayName}
            onInput={(e) => updateField('displayName', e.currentTarget.value)}
            placeholder={t('mcp.detail.namePlaceholder')}
          />
        </div>
        <div class="flex items-center gap-3">
          <label class="flex items-center gap-2 text-sm cursor-pointer">
            <input
              type="checkbox"
              checked={config().autoStart}
              onChange={(e) => updateField('autoStart', e.currentTarget.checked)}
            />
            {t('mcp.detail.autoStart')}
          </label>
          <span class="text-xs" style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}>
            {t('mcp.detail.autoStartHint')}
          </span>
        </div>

        {/* 传输类型 */}
        <div class="flex flex-col gap-1">
          <label class="text-xs" style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}>
            {t('mcp.detail.transport')}
          </label>
          <select
            class="px-3 py-1.5 rounded text-sm outline-none"
            style={{
              background: 'rgba(0,0,0,0.3)',
              border: '1px solid var(--border-dim)',
              color: 'white',
            }}
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
              <option value="stdio">{t('mcp.detail.transportStdio')}</option>
            </Show>
            <Show when={transports().includes('http')}>
              <option value="http">{t('mcp.detail.transportHttp')}</option>
            </Show>
          </select>
        </div>

        {/* stdio 字段 */}
        <Show when={config().transport.transport === 'stdio'}>
          {(() => {
            const trans = () => config().transport as Extract<McpTransport, { transport: 'stdio' }>;
            return (
              <>
                <div class="flex flex-col gap-1">
                  <label class="text-xs" style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}>
                    {t('mcp.detail.command')}
                  </label>
                  <input
                    class="px-3 py-1.5 rounded text-sm outline-none"
                    style={{
                      background: 'rgba(0,0,0,0.3)',
                      border: '1px solid var(--border-dim)',
                      color: 'white',
                    }}
                    value={trans().command}
                    onInput={(e) => updateTransport({ ...trans(), command: e.currentTarget.value })}
                    placeholder={t('mcp.detail.commandPlaceholder')}
                  />
                </div>
                <div class="flex flex-col gap-1">
                  <label
                    class="text-xs flex items-center justify-between"
                    style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}
                  >
                    {t('mcp.detail.args')}
                    <button
                      class="text-xs px-2 py-0.5 rounded"
                      style={{ background: 'rgba(var(--primary-rgb),0.2)' }}
                      onClick={addStdioArg}
                    >
                      {t('mcp.detail.add')}
                    </button>
                  </label>
                  <For each={trans().args}>
                    {(arg, idx) => (
                      <div class="flex items-center gap-1">
                        <input
                          class="flex-1 px-3 py-1.5 rounded text-sm outline-none"
                          style={{
                            background: 'rgba(0,0,0,0.3)',
                            border: '1px solid var(--border-dim)',
                            color: 'white',
                          }}
                          value={arg}
                          onInput={(e) => updateStdioArg(idx(), e.currentTarget.value)}
                        />
                        <button
                          class="px-2 py-1 rounded text-xs"
                          style={{
                            background: 'rgba(255,77,77,0.1)',
                            color: 'rgba(255,107,107,0.9)',
                          }}
                          onClick={() => removeStdioArg(idx())}
                        >
                          ×
                        </button>
                      </div>
                    )}
                  </For>
                </div>
                <div class="flex flex-col gap-1">
                  <label
                    class="text-xs flex items-center justify-between"
                    style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}
                  >
                    {t('mcp.detail.env')}
                    <button
                      class="text-xs px-2 py-0.5 rounded"
                      style={{ background: 'rgba(var(--primary-rgb),0.2)' }}
                      onClick={addEnvEntry}
                    >
                      {t('mcp.detail.add')}
                    </button>
                  </label>
                  <For each={Object.entries(trans().env)}>
                    {([k, v]) => (
                      <div class="flex items-center gap-1">
                        <input
                          class="w-1/3 px-2 py-1.5 rounded text-sm outline-none"
                          style={{
                            background: 'rgba(0,0,0,0.3)',
                            border: '1px solid var(--border-dim)',
                            color: 'white',
                          }}
                          value={k}
                          readonly
                        />
                        <input
                          class="flex-1 px-2 py-1.5 rounded text-sm outline-none"
                          style={{
                            background: 'rgba(0,0,0,0.3)',
                            border: '1px solid var(--border-dim)',
                            color: 'white',
                          }}
                          value={v.includes('${KEYRING:') ? t('mcp.detail.savedToKeychain') : v}
                          onInput={(e) => updateEnvEntry(k, e.currentTarget.value)}
                          placeholder={t('mcp.detail.envValuePlaceholder')}
                        />
                        <button
                          class="px-2 py-1 rounded text-xs whitespace-nowrap"
                          style={{
                            background: 'rgba(255,180,77,0.15)',
                            color: 'rgba(255,200,120,0.95)',
                          }}
                          title={t('mcp.detail.saveToKeychainTitle')}
                          onClick={() => void storeEnvSecret(k, v)}
                        >
                          {t('mcp.detail.saveToKeychain')}
                        </button>
                        <button
                          class="px-2 py-1 rounded text-xs"
                          style={{
                            background: 'rgba(255,77,77,0.1)',
                            color: 'rgba(255,107,107,0.9)',
                          }}
                          onClick={() => removeEnvEntry(k)}
                        >
                          ×
                        </button>
                      </div>
                    )}
                  </For>
                </div>
              </>
            );
          })()}
        </Show>

        {/* http 字段 */}
        <Show
          when={
            config().transport.transport === 'http' ||
            config().transport.transport === 'streamable_http'
          }
        >
          {(() => {
            const trans = () =>
              config().transport as Extract<
                McpTransport,
                { transport: 'http' | 'streamable_http' }
              >;
            return (
              <div class="flex flex-col gap-1">
                <label class="text-xs" style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}>
                  URL
                </label>
                <input
                  class="px-3 py-1.5 rounded text-sm outline-none"
                  style={{
                    background: 'rgba(0,0,0,0.3)',
                    border: '1px solid var(--border-dim)',
                    color: 'white',
                  }}
                  value={trans().url}
                  onInput={(e) => updateTransport({ ...trans(), url: e.currentTarget.value })}
                  placeholder="https://mcp.example.com/sse"
                />
                <Show
                  when={
                    !trans().url.startsWith('http://') &&
                    !trans().url.startsWith('https://') &&
                    trans().url !== ''
                  }
                >
                  <span class="text-xs" style={{ color: '#ff8a8a' }}>
                    {t('mcp.detail.urlValidate')}
                  </span>
                </Show>
              </div>
            );
          })()}
        </Show>

        {/* 测试连接 */}
        <div class="flex items-center gap-2">
          <button
            class="px-3 py-1.5 rounded text-sm cursor-pointer transition-colors"
            style={{
              background: 'rgba(var(--primary-rgb),0.2)',
              border: '1px solid rgba(var(--primary-rgb),0.3)',
            }}
            disabled={testing()}
            onClick={handleTest}
          >
            {testing() ? t('mcp.detail.testing') : t('mcp.detail.testConnection')}
          </button>
          <Show when={testResult()}>
            <span
              class="text-xs px-2 py-1 rounded"
              style={
                testResult()!.ok
                  ? 'background: rgba(124,217,160,0.1); color: #7cd9a0;'
                  : 'background: rgba(255,77,77,0.1); color: #ff8a8a;'
              }
            >
              {testResult()!.ok
                ? `✓ ${t('mcp.detail.testSuccess', { count: testResult()!.tools?.length ?? 0 })}`
                : `✗ ${testResult()!.error}`}
            </span>
          </Show>
        </div>

        {/* 工具白名单 */}
        <Show when={availableTools().length > 0}>
          <div class="flex flex-col gap-2">
            <div class="flex items-center justify-between">
              <label class="text-xs" style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}>
                {t('mcp.detail.toolWhitelist')}（
                {config().enabledTools.length === 0
                  ? t('mcp.detail.enableAll')
                  : `${t('mcp.detail.selected')} ${config().enabledTools.length}/${availableTools().length}`}
                ）
              </label>
              <div class="flex gap-2">
                <button
                  class="text-xs px-2 py-0.5 rounded"
                  style={{ background: 'rgba(var(--primary-rgb),0.2)' }}
                  onClick={selectAllTools}
                >
                  {t('mcp.detail.selectAll')}
                </button>
                <button
                  class="text-xs px-2 py-0.5 rounded"
                  style={{ background: 'rgba(var(--text-base-rgb),0.05)' }}
                  onClick={deselectAllTools}
                >
                  {t('mcp.detail.clearAll')}
                </button>
              </div>
            </div>
            <div class="flex flex-col gap-1 max-h-40 overflow-y-auto">
              <For each={availableTools()}>
                {(tool) => (
                  <label
                    class="flex items-start gap-2 text-xs px-2 py-1 rounded cursor-pointer"
                    style={{ background: 'rgba(var(--text-base-rgb),0.03)' }}
                  >
                    <input
                      type="checkbox"
                      checked={
                        config().enabledTools.length === 0 ||
                        config().enabledTools.includes(tool.function.name)
                      }
                      onChange={() => toggleTool(tool.function.name)}
                      class="mt-0.5"
                    />
                    <div class="flex-1 min-w-0">
                      <div class="font-mono" style={{ color: 'rgba(var(--text-base-rgb),0.85)' }}>
                        {tool.function.name}
                      </div>
                      <div style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}>
                        {tool.function.description}
                      </div>
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
            style={{
              background: 'rgba(var(--text-base-rgb),0.05)',
              border: '1px solid var(--border-dim)',
            }}
            onClick={() => props.onCancel()}
          >
            {t('common.cancel')}
          </button>
          <button
            class="px-3 py-1.5 rounded text-sm cursor-pointer"
            style={{
              background: 'rgba(124,217,160,0.2)',
              border: '1px solid rgba(124,217,160,0.3)',
            }}
            onClick={() => props.onSave(config())}
          >
            {t('common.save')}
          </button>
        </div>
      </div>
    </div>
  );
};

export default McpServerDetail;
