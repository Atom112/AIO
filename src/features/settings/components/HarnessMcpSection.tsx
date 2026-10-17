/**
 * Harness tool MCP server management (settings detail page): the tool's own
 * MCP config — claude ~/.claude.json mcpServers, gemini/qwen settings.json
 * mcpServers, opencode opencode.json mcp, codex config.toml
 * [mcp_servers.<name>], goose config.yaml extensions — listed with
 * transport/command, added/edited in a modal (stdio command/args/env or HTTP
 * URL) and deleted, all through the tool-native write shapes.
 */
import { Component, createSignal, For, onMount, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import { btnPrimaryOutlineSm, btnSecondarySm } from '../../../shared/components/buttonStyles';
import {
  harnessToolMcpDelete,
  harnessToolMcpList,
  harnessToolMcpSet,
  type HarnessMcpEntry,
} from '../../../core/api/harness';
import { t } from '../../../core/i18n';

/** Tools whose MCP management is supported (backend matrix). */
const MCP_TOOLS = new Set(['claude', 'gemini', 'qwen', 'opencode', 'codex', 'goose']);

interface EditState {
  name: string;
  isNew: boolean;
  transport: string;
  command: string;
  url: string;
  args: string;
  env: string;
}

const HarnessMcpSection: Component<{ toolId: string }> = (props) => {
  const [entries, setEntries] = createSignal<HarnessMcpEntry[]>([]);
  const [edit, setEdit] = createSignal<EditState | null>(null);
  const [newName, setNewName] = createSignal('');
  const [error, setError] = createSignal('');
  const [savedTick, setSavedTick] = createSignal(false);

  const reload = async () => {
    try {
      setEntries(await harnessToolMcpList(props.toolId));
    } catch (e) {
      setError(String(e));
    }
  };
  onMount(() => void reload());

  const openEdit = (e: HarnessMcpEntry) => {
    setError('');
    setEdit({
      name: e.name,
      isNew: false,
      transport: e.type,
      command: e.command,
      url: e.url ?? '',
      args: (e.args ?? []).join('\n'),
      env: (e.env ?? []).map(([k, v]) => `${k}=${v}`).join('\n'),
    });
  };

  const createNew = () => {
    const name = newName().trim();
    if (!name) return;
    setError('');
    setEdit({
      name,
      isNew: true,
      transport: 'stdio',
      command: '',
      url: '',
      args: '',
      env: '',
    });
    setNewName('');
  };

  const save = async () => {
    const st = edit();
    if (!st) return;
    setError('');
    const env: [string, string][] = st.env
      .split('\n')
      .map((l) => l.trim())
      .filter(Boolean)
      .map((l) => {
        const idx = l.indexOf('=');
        return idx > 0 ? [l.slice(0, idx), l.slice(idx + 1)] : [l, ''];
      });
    try {
      await harnessToolMcpSet(props.toolId, st.name, {
        name: st.name,
        type: st.transport,
        command: st.command.trim(),
        url: st.transport === 'http' ? st.url.trim() : undefined,
        args: st.args
          .split('\n')
          .map((l) => l.trim())
          .filter(Boolean),
        env,
      });
      setSavedTick(true);
      setTimeout(() => setSavedTick(false), 1500);
      setEdit(null);
      await reload();
    } catch (e) {
      setError(String(e));
    }
  };

  const remove = async (name: string) => {
    if (!confirm(t('harness.mcp.deleteConfirm', { name }))) return;
    setError('');
    try {
      await harnessToolMcpDelete(props.toolId, name);
      await reload();
    } catch (e) {
      setError(String(e));
    }
  };

  const inputClass =
    'w-full rounded-lg bg-black/20 border border-[var(--border-dim)] px-3 py-1.5 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors font-mono';

  return (
    <div
      class="rounded-xl p-4"
      style={{
        background: 'rgba(var(--text-base-rgb),0.03)',
        border: '1px solid var(--border-dim)',
      }}
    >
      {/* Entries */}
      <For each={entries()}>
        {(entry) => (
          <div class="flex items-center gap-2 py-1.5 group">
            <Icon
              name={entry.type === 'http' ? 'globe' : 'terminal'}
              size={13}
              class="text-white/30 shrink-0"
            />
            <button
              class="flex-1 min-w-0 text-left cursor-pointer border-none bg-transparent p-0"
              onClick={() => openEdit(entry)}
            >
              <div class="text-xs text-white/75 truncate">{entry.name}</div>
              <div class="text-[11px] text-white/35 truncate font-mono">
                {entry.type === 'http'
                  ? entry.url
                  : [entry.command, ...(entry.args ?? [])].join(' ')}
              </div>
            </button>
            <Show when={entry.enabled === false}>
              <span class="text-[10px] text-white/25 shrink-0">{t('harness.mcp.disabled')}</span>
            </Show>
            <button
              class="opacity-0 group-hover:opacity-100 text-white/30 hover:text-danger transition-all duration-200 cursor-pointer border-none bg-transparent p-1 shrink-0"
              title={t('common.delete')}
              onClick={() => void remove(entry.name)}
            >
              <Icon name="trash" size={12} />
            </button>
          </div>
        )}
      </For>
      <Show when={entries().length === 0}>
        <div class="text-[11px] text-white/30 py-1.5">{t('harness.mcp.empty')}</div>
      </Show>

      {/* Create */}
      <div class="flex items-center gap-2 mt-2">
        <input
          class="flex-1 min-w-[120px] max-w-[240px] rounded-lg bg-black/20 border border-[var(--border-dim)] px-3 py-1.5 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors"
          placeholder={t('harness.mcp.namePlaceholder')}
          value={newName()}
          onInput={(e) => setNewName(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') createNew();
          }}
        />
        <button
          class={btnPrimaryOutlineSm}
          disabled={!newName().trim()}
          onClick={() => createNew()}
        >
          <Icon name="plus" size={12} /> {t('harness.mcp.add')}
        </button>
      </div>

      <Show when={error()}>
        <div class="text-[11px] text-danger mt-2 break-all">{error()}</div>
      </Show>

      {/* Editor modal */}
      <Show when={edit()}>
        <div
          class="fixed inset-0 z-[1000] bg-black/25 flex items-center justify-center"
          onClick={() => setEdit(null)}
        >
          <div
            class="rounded-xl px-6 py-5 max-w-[560px] w-[92%] relative"
            style={{
              background: 'var(--dialog-bg)',
              'backdrop-filter': 'var(--popup-blur)',
              border: '1px solid var(--border-dim)',
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <div class="text-sm font-medium text-white/85 mb-3">
              {t('harness.mcp.editorTitle', { name: edit()!.name })}
            </div>

            {/* Transport */}
            <div class="flex items-center gap-2 mb-3">
              <button
                class={`px-3 py-1.5 text-xs rounded-md border cursor-pointer transition-all duration-200 ${
                  edit()!.transport === 'stdio'
                    ? 'border-pri-50 text-pri bg-pri-10'
                    : 'border-white/10 text-white/50 hover:text-white/80'
                }`}
                onClick={() => setEdit({ ...edit()!, transport: 'stdio' })}
              >
                <Icon name="terminal" size={12} /> stdio
              </button>
              <button
                class={`px-3 py-1.5 text-xs rounded-md border cursor-pointer transition-all duration-200 ${
                  edit()!.transport === 'http'
                    ? 'border-pri-50 text-pri bg-pri-10'
                    : 'border-white/10 text-white/50 hover:text-white/80'
                }`}
                onClick={() => setEdit({ ...edit()!, transport: 'http' })}
              >
                <Icon name="globe" size={12} /> HTTP
              </button>
            </div>

            <div class="flex flex-col gap-2 mb-2">
              <Show
                when={edit()!.transport === 'stdio'}
                fallback={
                  <input
                    class={inputClass}
                    placeholder="https://example.com/mcp"
                    value={edit()!.url}
                    onInput={(e) => setEdit({ ...edit()!, url: e.currentTarget.value })}
                  />
                }
              >
                <input
                  class={inputClass}
                  placeholder="npx"
                  value={edit()!.command}
                  onInput={(e) => setEdit({ ...edit()!, command: e.currentTarget.value })}
                />
                <textarea
                  class={`${inputClass} resize-none`}
                  rows={2}
                  placeholder={t('harness.mcp.argsPlaceholder')}
                  value={edit()!.args}
                  onInput={(e) => setEdit({ ...edit()!, args: e.currentTarget.value })}
                />
                <textarea
                  class={`${inputClass} resize-none`}
                  rows={2}
                  placeholder={t('harness.mcp.envPlaceholder')}
                  value={edit()!.env}
                  onInput={(e) => setEdit({ ...edit()!, env: e.currentTarget.value })}
                />
              </Show>
            </div>

            <div class="flex items-center justify-end gap-2 mt-4">
              <button class={btnSecondarySm} onClick={() => setEdit(null)}>
                {t('common.cancel')}
              </button>
              <button class={btnPrimaryOutlineSm} onClick={() => void save()}>
                {savedTick() ? t('common.saved') : t('common.save')}
              </button>
            </div>
            <Show when={error()}>
              <div class="text-[11px] text-danger mt-2 break-all">{error()}</div>
            </Show>
          </div>
        </div>
      </Show>
    </div>
  );
};

export { MCP_TOOLS };
export default HarnessMcpSection;
