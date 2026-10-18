import { Component, createSignal, For, Show, onMount, untrack } from 'solid-js';
import { listMcpTransports, saveMcpServerSecret } from '../../../core/api/mcp';
import type { McpServerConfig, McpTransport, ToolSpec } from '../../../core/types/mcp';
import { t } from '../../../core/i18n';
import Dropdown from '../../../shared/components/Dropdown';
import Checkbox from '../../../shared/components/Checkbox';
import Icon from '../../../shared/components/Icon';
import {
  btnBadge,
  btnDangerOutline,
  btnPrimaryOutlineSm,
  btnSecondarySm,
  btnSuccessOutline,
  btnWarnOutline,
} from '../../../shared/components/buttonStyles';

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

  // Load registered transports
  const [transports, setTransports] = createSignal<string[]>([]);
  onMount(() => {
    void listMcpTransports()
      .then(setTransports)
      .catch(() => {
        // Keep the built-in transport choices when discovery is unavailable.
      });
  });

  // Transport dropdown options: registered transports (discovery may lag) plus the
  // config's current transport, so an existing streamable_http entry still renders.
  const TRANSPORT_LABELS: Record<string, () => string> = {
    stdio: () => t('mcp.detail.transportStdio'),
    http: () => t('mcp.detail.transportHttp'),
    streamable_http: () => t('mcp.detail.transportStreamable'),
  };
  const transportOptions = () => {
    const registered = transports();
    const current = config().transport.transport as string;
    return Object.keys(TRANSPORT_LABELS)
      .filter((id) => registered.includes(id) || id === current)
      .map((id) => ({ value: id as McpTransport['transport'], label: TRANSPORT_LABELS[id]() }));
  };

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

  // Key names carry meaning (BRAVE_API_KEY / Authorization), so keys are editable:
  // renaming rebuilds the map entry; collisions keep the old name (the controlled
  // input snaps back).
  const renameMapKey = (
    obj: Record<string, string>,
    oldKey: string,
    newKey: string,
  ): Record<string, string> => {
    if (!newKey || newKey === oldKey || Object.prototype.hasOwnProperty.call(obj, newKey))
      return obj;
    return Object.fromEntries(
      Object.entries(obj).map(([k, v]) => (k === oldKey ? [newKey, v] : [k, v])),
    );
  };

  const renameEnvEntry = (oldKey: string, newKey: string) => {
    const t = config().transport;
    if (t.transport !== 'stdio') return;
    updateTransport({ ...t, env: renameMapKey(t.env, oldKey, newKey) });
  };

  const storeEnvSecret = async (key: string, value: string) => {
    if (!value || value.includes('${KEYRING:')) return;
    const placeholder = await saveMcpServerSecret(config().id, 'env', key, value);
    const t = config().transport;
    if (t.transport !== 'stdio') return;
    updateTransport({ ...t, env: { ...t.env, [key]: placeholder } });
    updateField('hasStoredSecret', true);
  };

  type HttpTransport = Extract<McpTransport, { transport: 'http' | 'streamable_http' }>;
  const httpTransport = (): HttpTransport | null => {
    const t = config().transport;
    return t.transport === 'http' || t.transport === 'streamable_http' ? t : null;
  };

  const addHeaderEntry = () => {
    const t = httpTransport();
    if (!t) return;
    const key = `X-Header-${Object.keys(t.headers).length + 1}`;
    updateTransport({ ...t, headers: { ...t.headers, [key]: '' } });
  };

  const updateHeaderEntry = (key: string, value: string) => {
    const t = httpTransport();
    if (!t) return;
    updateTransport({ ...t, headers: { ...t.headers, [key]: value } });
  };

  const renameHeaderEntry = (oldKey: string, newKey: string) => {
    const t = httpTransport();
    if (!t) return;
    updateTransport({ ...t, headers: renameMapKey(t.headers, oldKey, newKey) });
  };

  const storeHeaderSecret = async (key: string, value: string) => {
    if (!value || value.includes('${KEYRING:')) return;
    const placeholder = await saveMcpServerSecret(config().id, 'header', key, value);
    const t = httpTransport();
    if (!t) return;
    updateTransport({ ...t, headers: { ...t.headers, [key]: placeholder } });
    updateField('hasStoredSecret', true);
  };

  const removeHeaderEntry = (key: string) => {
    const t = httpTransport();
    if (!t) return;
    const headers = { ...t.headers };
    delete headers[key];
    updateTransport({ ...t, headers });
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
      class="fixed inset-0 z-50 flex items-center justify-center p-6 bg-black/25"
      onClick={(e) => {
        if (e.target === e.currentTarget) props.onCancel();
      }}
    >
      <div
        class="w-[640px] max-w-full max-h-[90vh] overflow-y-auto rounded-xl p-6 flex flex-col gap-4"
        style={{
          background: 'var(--dialog-bg)',
          'backdrop-filter': 'var(--popup-blur)',
          border: '1px solid var(--border-dim)',
        }}
      >
        <h3 class="text-base font-semibold">
          {props.isNew ? t('mcp.detail.titleAdd') : t('mcp.detail.titleEdit')}
        </h3>

        {/* Name + enable */}
        <div class="flex flex-col gap-1">
          <label class="text-xs" style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}>
            {t('mcp.detail.name')}
          </label>
          <input
            class="px-3 py-1.5 rounded-lg text-sm outline-none"
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
          <Checkbox
            class="text-sm"
            checked={config().autoStart}
            onChange={(v) => updateField('autoStart', v)}
            label={t('mcp.detail.autoStart')}
          />
        </div>
        <div class="flex items-center gap-3">
          <Checkbox
            class="text-sm"
            checked={config().trusted ?? false}
            onChange={(v) => updateField('trusted', v)}
            label={t('mcp.detail.trusted')}
          />
        </div>

        {/* Transport type */}
        <div class="flex flex-col gap-1">
          <label class="text-xs" style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}>
            {t('mcp.detail.transport')}
          </label>
          <Dropdown
            variant="ghost"
            class="w-full"
            value={config().transport.transport}
            onChange={(v) => {
              if (v === 'stdio') {
                updateTransport({ transport: 'stdio', command: 'npx', args: [], env: {} });
              } else if (v === 'http') {
                updateTransport({ transport: 'http', url: '', headers: {} });
              } else {
                updateTransport({ transport: 'streamable_http', url: '', headers: {} });
              }
            }}
            options={transportOptions()}
          />
        </div>

        {/* stdio fields */}
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
                    class="px-3 py-1.5 rounded-lg text-sm outline-none"
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
                    <button class={btnBadge} onClick={addStdioArg}>
                      {t('mcp.detail.add')}
                    </button>
                  </label>
                  <For each={trans().args}>
                    {(arg, idx) => (
                      <div class="flex items-center gap-1">
                        <input
                          class="flex-1 px-3 py-1.5 rounded-lg text-sm outline-none"
                          style={{
                            background: 'rgba(0,0,0,0.3)',
                            border: '1px solid var(--border-dim)',
                            color: 'white',
                          }}
                          value={arg}
                          onInput={(e) => updateStdioArg(idx(), e.currentTarget.value)}
                        />
                        <button class={btnDangerOutline} onClick={() => removeStdioArg(idx())}>
                          <Icon name="x" size={12} />
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
                    <button class={btnBadge} onClick={addEnvEntry}>
                      {t('mcp.detail.add')}
                    </button>
                  </label>
                  <For each={Object.entries(trans().env)}>
                    {([k, v]) => (
                      <div class="flex items-center gap-1">
                        <input
                          class="w-1/3 px-2 py-1.5 rounded-lg text-sm outline-none"
                          style={{
                            background: 'rgba(0,0,0,0.3)',
                            border: '1px solid var(--border-dim)',
                            color: 'white',
                          }}
                          value={k}
                          onInput={(e) => renameEnvEntry(k, e.currentTarget.value)}
                        />
                        <Show
                          when={v.includes('${KEYRING:')}
                          fallback={
                            <input
                              class="flex-1 px-2 py-1.5 rounded-lg text-sm outline-none"
                              style={{
                                background: 'rgba(0,0,0,0.3)',
                                border: '1px solid var(--border-dim)',
                                color: 'white',
                              }}
                              value={v}
                              onInput={(e) => updateEnvEntry(k, e.currentTarget.value)}
                              placeholder={t('mcp.detail.envValuePlaceholder')}
                            />
                          }
                        >
                          {/* FE-01: read-only keyring placeholder (title explains how to replace) */}
                          <input
                            class="flex-1 px-2 py-1.5 rounded-lg text-sm outline-none"
                            style={{
                              background: 'rgba(0,0,0,0.3)',
                              border: '1px solid var(--border-dim)',
                              color: 'rgba(var(--text-base-rgb),0.55)',
                            }}
                            value={t('mcp.detail.savedToKeychain')}
                            readonly
                            title={t('mcp.detail.keyringReadOnlyTitle')}
                          />
                        </Show>
                        <button
                          class={btnWarnOutline}
                          title={t('mcp.detail.saveToKeychainTitle')}
                          onClick={() => void storeEnvSecret(k, v)}
                        >
                          {t('mcp.detail.saveToKeychain')}
                        </button>
                        <button class={btnDangerOutline} onClick={() => removeEnvEntry(k)}>
                          <Icon name="x" size={12} />
                        </button>
                      </div>
                    )}
                  </For>
                </div>
              </>
            );
          })()}
        </Show>

        {/* http fields */}
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
                  class="px-3 py-1.5 rounded-lg text-sm outline-none"
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
                  <span class="text-xs" style={{ color: 'var(--color-danger)' }}>
                    {t('mcp.detail.urlValidate')}
                  </span>
                </Show>
              </div>
            );
          })()}
        </Show>

        {/* http headers (credentials live here — secrets go to the keyring) */}
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
                <label
                  class="text-xs flex items-center justify-between"
                  style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}
                >
                  {t('mcp.detail.headers')}
                  <button class={btnBadge} onClick={addHeaderEntry}>
                    {t('mcp.detail.add')}
                  </button>
                </label>
                <For each={Object.entries(trans().headers)}>
                  {([k, v]) => (
                    <div class="flex items-center gap-1">
                      <input
                        class="w-1/3 px-2 py-1.5 rounded-lg text-sm outline-none"
                        style={{
                          background: 'rgba(0,0,0,0.3)',
                          border: '1px solid var(--border-dim)',
                          color: 'white',
                        }}
                        value={k}
                        placeholder={t('mcp.detail.headerNamePlaceholder')}
                        onInput={(e) => renameHeaderEntry(k, e.currentTarget.value)}
                      />
                      <Show
                        when={v.includes('${KEYRING:')}
                        fallback={
                          <input
                            class="flex-1 px-2 py-1.5 rounded-lg text-sm outline-none"
                            style={{
                              background: 'rgba(0,0,0,0.3)',
                              border: '1px solid var(--border-dim)',
                              color: 'white',
                            }}
                            value={v}
                            onInput={(e) => updateHeaderEntry(k, e.currentTarget.value)}
                            placeholder={t('mcp.detail.envValuePlaceholder')}
                          />
                        }
                      >
                        <input
                          class="flex-1 px-2 py-1.5 rounded-lg text-sm outline-none"
                          style={{
                            background: 'rgba(0,0,0,0.3)',
                            border: '1px solid var(--border-dim)',
                            color: 'rgba(var(--text-base-rgb),0.55)',
                          }}
                          value={t('mcp.detail.savedToKeychain')}
                          readonly
                          title={t('mcp.detail.keyringReadOnlyTitle')}
                        />
                      </Show>
                      <button
                        class={btnWarnOutline}
                        title={t('mcp.detail.saveToKeychainTitle')}
                        onClick={() => void storeHeaderSecret(k, v)}
                      >
                        {t('mcp.detail.saveToKeychain')}
                      </button>
                      <button class={btnDangerOutline} onClick={() => removeHeaderEntry(k)}>
                        <Icon name="x" size={12} />
                      </button>
                    </div>
                  )}
                </For>
              </div>
            );
          })()}
        </Show>

        {/* Test connection */}
        <div class="flex items-center gap-2">
          <button class={btnSuccessOutline} disabled={testing()} onClick={handleTest}>
            {testing() ? t('mcp.detail.testing') : t('mcp.detail.testConnection')}
          </button>
          <Show when={testResult()}>
            <span
              class="text-xs px-2 py-1 rounded"
              style={
                testResult()!.ok
                  ? 'background: rgba(124,217,160,0.1); color: var(--color-success);'
                  : 'background: rgba(255,77,77,0.1); color: var(--color-danger);'
              }
            >
              {testResult()!.ok
                ? `✓ ${t('mcp.detail.testSuccess', { count: testResult()!.tools?.length ?? 0 })}`
                : `✗ ${testResult()!.error}`}
            </span>
          </Show>
        </div>

        {/* Tool whitelist */}
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
                  <Checkbox
                    class="items-start gap-2 text-xs px-2 py-1 rounded"
                    style={{ background: 'rgba(var(--text-base-rgb),0.03)' }}
                    checked={
                      config().enabledTools.length === 0 ||
                      config().enabledTools.includes(tool.function.name)
                    }
                    onChange={() => toggleTool(tool.function.name)}
                  >
                    <div class="flex-1 min-w-0">
                      <div class="font-mono" style={{ color: 'rgba(var(--text-base-rgb),0.85)' }}>
                        {tool.function.name}
                      </div>
                      <div style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}>
                        {tool.function.description}
                      </div>
                    </div>
                  </Checkbox>
                )}
              </For>
            </div>
          </div>
        </Show>

        {/* Buttons */}
        <div class="flex justify-end gap-2 mt-2">
          <button class={btnSecondarySm} onClick={() => props.onCancel()}>
            {t('common.cancel')}
          </button>
          <button class={btnPrimaryOutlineSm} onClick={() => props.onSave(config())}>
            {t('common.save')}
          </button>
        </div>
      </div>
    </div>
  );
};

export default McpServerDetail;
