/**
 * Harness tool markdown collections (settings detail page): the tool's own
 * discovery directories — claude skills (`~/.claude/skills/<dir>/SKILL.md`),
 * goose prompt templates (`prompts/*.md`), opencode custom commands
 * (`commands/*.md`) — listed with one-line previews, edited in a modal
 * (create / overwrite / delete) through the atomic-write + backup pipeline.
 */
import { Component, createSignal, For, onMount, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import { btnDangerOutline, btnPrimaryOutlineSm } from '../../../shared/components/buttonStyles';
import {
  harnessToolMdDelete,
  harnessToolMdEntries,
  harnessToolMdRead,
  harnessToolMdWrite,
  type HarnessMarkdownEntry,
} from '../../../core/api/harness';
import { t } from '../../../core/i18n';

interface HarnessMarkdownCollectionProps {
  toolId: string;
  kind: string;
  label: string;
}

interface EditTarget {
  name: string;
  content: string;
  isNew: boolean;
}

const HarnessMarkdownCollection: Component<HarnessMarkdownCollectionProps> = (props) => {
  const [entries, setEntries] = createSignal<HarnessMarkdownEntry[]>([]);
  const [edit, setEdit] = createSignal<EditTarget | null>(null);
  const [draft, setDraft] = createSignal('');
  const [newName, setNewName] = createSignal('');
  const [error, setError] = createSignal('');
  const [savedTick, setSavedTick] = createSignal(false);

  const reload = async () => {
    try {
      setEntries(await harnessToolMdEntries(props.toolId, props.kind));
    } catch (e) {
      setError(String(e));
    }
  };
  onMount(() => void reload());

  const openEdit = async (name: string) => {
    setError('');
    try {
      const content = await harnessToolMdRead(props.toolId, props.kind, name);
      setEdit({ name, content, isNew: false });
      setDraft(content);
    } catch (e) {
      setError(String(e));
    }
  };

  const createNew = () => {
    const name = newName().trim();
    if (!name) return;
    setError('');
    setEdit({ name, content: '', isNew: true });
    setDraft('');
    setNewName('');
  };

  const save = async () => {
    const target = edit();
    if (!target) return;
    setError('');
    try {
      await harnessToolMdWrite(props.toolId, props.kind, target.name, draft());
      setSavedTick(true);
      setTimeout(() => setSavedTick(false), 1500);
      setEdit(null);
      await reload();
    } catch (e) {
      setError(String(e));
    }
  };

  const remove = async (name: string) => {
    if (!confirm(t('harness.md.deleteConfirm', { name }))) return;
    setError('');
    try {
      await harnessToolMdDelete(props.toolId, props.kind, name);
      await reload();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div
      class="rounded-xl p-4"
      style={{
        background: 'rgba(var(--text-base-rgb),0.03)',
        border: '1px solid var(--border-dim)',
      }}
    >
      <div class="text-xs font-semibold text-white/70 mb-2">{props.label}</div>

      {/* Entries */}
      <For each={entries()}>
        {(entry) => (
          <div class="flex items-center gap-2 py-1.5 group">
            <Icon name="document" size={13} class="text-white/30 shrink-0" />
            <button
              class="flex-1 min-w-0 text-left cursor-pointer border-none bg-transparent p-0"
              onClick={() => void openEdit(entry.name)}
              title={entry.path}
            >
              <div class="text-xs text-white/75 truncate">{entry.name}</div>
              <Show when={entry.summary}>
                <div class="text-[11px] text-white/30 truncate">{entry.summary}</div>
              </Show>
            </button>
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
        <div class="text-[11px] text-white/30 py-1.5">{t('harness.md.empty')}</div>
      </Show>

      {/* Create */}
      <div class="flex items-center gap-2 mt-2">
        <input
          class="flex-1 min-w-[120px] max-w-[240px] rounded-lg bg-black/20 border border-[var(--border-dim)] px-3 py-1.5 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors"
          placeholder={t('harness.md.namePlaceholder')}
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
          <Icon name="plus" size={12} /> {t('harness.md.create')}
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
            class="rounded-xl px-6 py-5 max-w-[640px] w-[92%] relative"
            style={{
              background: 'var(--dialog-bg)',
              'backdrop-filter': 'var(--popup-blur)',
              border: '1px solid var(--border-dim)',
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <div class="text-sm font-medium text-white/85 mb-3">
              {props.label} — {edit()!.name}
              <Show when={edit()!.isNew}>
                <span class="text-[11px] text-white/35"> · {t('harness.md.willCreate')}</span>
              </Show>
            </div>
            <textarea
              class="w-full rounded-lg bg-black/20 border border-[var(--border-dim)] px-3 py-2 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors resize-y min-h-[260px] font-mono leading-relaxed"
              spellcheck={false}
              placeholder={t('harness.md.placeholder')}
              value={draft()}
              onInput={(e) => setDraft(e.currentTarget.value)}
            />
            <div class="flex items-center justify-end gap-2 mt-4">
              <button class={btnDangerOutline} onClick={() => setEdit(null)}>
                {t('common.cancel')}
              </button>
              <button class={btnPrimaryOutlineSm} onClick={() => void save()}>
                {savedTick() ? t('common.saved') : t('common.save')}
              </button>
            </div>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default HarnessMarkdownCollection;
