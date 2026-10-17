/**
 * Harness tool structured config form (settings detail page): renders the
 * tool's curated config keys (typed descriptors from the backend) as form
 * rows — text / enum dropdown / switch / number / rule chips / kv pairs.
 * Every change writes through to the tool's own config file
 * (harness_tool_structured_set; JSON null removes the key → tool default),
 * so the form is the GUI equivalent of hand-editing the CLI's settings file.
 */
import { Component, createSignal, For, onMount, Show } from 'solid-js';
import Dropdown from '../../../shared/components/Dropdown';
import Switch from '../../../shared/components/Switch';
import Icon from '../../../shared/components/Icon';
import { btnBadge, btnPrimaryOutlineSm } from '../../../shared/components/buttonStyles';
import {
  harnessToolStructuredGet,
  harnessToolStructuredSet,
  type HarnessStructuredKey,
  type HarnessStructuredValue,
} from '../../../core/api/harness';
import { t, type TranslationKey } from '../../../core/i18n';

interface HarnessToolStructuredFormProps {
  toolId: string;
}

/** i18n label for a known config key path (falls back to the registry label). */
const KEY_LABELS: Record<string, TranslationKey> = {
  model: 'harness.key.model',
  'model.name': 'harness.key.model',
  'permissions.defaultMode': 'harness.key.permissionMode',
  alwaysThinkingEnabled: 'harness.key.alwaysThinking',
  cleanupPeriodDays: 'harness.key.cleanupDays',
  'permissions.allow': 'harness.key.allow',
  'permissions.ask': 'harness.key.ask',
  'permissions.deny': 'harness.key.deny',
  env: 'harness.key.env',
  'tools.approvalMode': 'harness.key.approvalMode',
  'context.fileName': 'harness.key.contextFileName',
  autoupdate: 'harness.key.autoupdate',
};

const labelFor = (k: HarnessStructuredKey) => {
  const i18nKey = KEY_LABELS[k.key];
  return i18nKey ? t(i18nKey) : k.label;
};

const rowClass = 'flex items-center gap-3 py-2.5 px-3 rounded-lg transition-colors duration-150';

/** Scalar value of a str/bool/num key (undefined = unset). */
const scalar = (v: HarnessStructuredValue): string | boolean | number | undefined =>
  v.kind === 'str' || v.kind === 'bool' || v.kind === 'num' ? v.value : undefined;

const HarnessToolStructuredForm: Component<HarnessToolStructuredFormProps> = (props) => {
  const [keys, setKeys] = createSignal<HarnessStructuredKey[]>([]);
  const [error, setError] = createSignal('');
  const [busyKey, setBusyKey] = createSignal('');
  const [filter, setFilter] = createSignal('');
  // Group expansion state; curated catalogs (≤20 keys) start expanded, the
  // omp full catalog (~450 keys) starts collapsed with the filter box as the
  // primary discovery path.
  const [expanded, setExpanded] = createSignal<Record<string, boolean> | null>(null);

  const reload = async () => {
    try {
      setKeys(await harnessToolStructuredGet(props.toolId));
    } catch (e) {
      setError(String(e));
    }
  };
  onMount(() => void reload());

  const apply = async (key: string, value: unknown) => {
    setError('');
    setBusyKey(key);
    try {
      await harnessToolStructuredSet(props.toolId, key, value);
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyKey('');
    }
  };

  /** Group name = key prefix before the first dot (omp's own vocabulary). */
  const groupOf = (key: string) => (key.includes('.') ? key.slice(0, key.indexOf('.')) : 'general');

  /** Filtered keys → ordered groups of keys. */
  const groups = () => {
    const needle = filter().trim().toLowerCase();
    const list = keys().filter(
      (k) =>
        !needle ||
        k.key.toLowerCase().includes(needle) ||
        labelFor(k).toLowerCase().includes(needle) ||
        (k.description ?? '').toLowerCase().includes(needle),
    );
    const map = new Map<string, HarnessStructuredKey[]>();
    for (const k of list) {
      const g = groupOf(k.key);
      if (!map.has(g)) map.set(g, []);
      map.get(g)!.push(k);
    }
    return [...map.entries()];
  };

  const isExpanded = (group: string) => {
    if (filter().trim()) return true; // filtering auto-expands matches
    if (expanded() === null) return keys().length <= 20; // curated default
    return !!expanded()![group];
  };
  const toggleGroup = (group: string) => {
    const base = expanded() ?? Object.fromEntries(groups().map(([g]) => [g, keys().length <= 20]));
    setExpanded({ ...base, [group]: !base[group] });
  };

  return (
    <Show when={keys().length > 0}>
      <div
        class="rounded-xl p-2"
        style={{
          background: 'rgba(var(--text-base-rgb),0.03)',
          border: '1px solid var(--border-dim)',
        }}
      >
        {/* Filter box for large catalogs (omp's full listing). */}
        <Show when={keys().length > 20}>
          <div class="px-3 pb-1 pt-2">
            <input
              class="w-full rounded-lg bg-black/20 border border-[var(--border-dim)] px-3 py-1.5 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors"
              placeholder={t('harness.key.filterPlaceholder')}
              value={filter()}
              onInput={(e) => setFilter(e.currentTarget.value)}
            />
          </div>
        </Show>

        <For each={groups()}>
          {([group, groupKeys]) => (
            <div class="mb-1">
              {/* Group header (collapsible) */}
              <button
                type="button"
                class="w-full flex items-center gap-1.5 px-3 py-1.5 rounded-md cursor-pointer border-none bg-transparent text-[11px] font-bold uppercase tracking-wider text-[rgba(var(--text-base-rgb),0.4)] hover:text-[rgba(var(--text-base-rgb),0.7)] transition-colors"
                onClick={() => toggleGroup(group)}
              >
                <Icon
                  name={isExpanded(group) ? 'caret-down' : 'chevron-right'}
                  size={12}
                  class="shrink-0"
                />
                <span class="flex-1 text-left truncate">{group}</span>
                <span class="text-[10px] font-medium normal-case tracking-normal text-white/25 shrink-0">
                  {groupKeys.length}
                </span>
              </button>
              <Show when={isExpanded(group)}>
                <For each={groupKeys}>
                  {(k) => (
                    <KeyRow k={k} busy={busyKey() === k.key} onApply={(v) => apply(k.key, v)} />
                  )}
                </For>
              </Show>
            </div>
          )}
        </For>
        <Show when={groups().length === 0}>
          <div class="px-3 py-3 text-xs text-white/30">{t('harness.key.noMatches')}</div>
        </Show>

        <Show when={error()}>
          <div class="text-[11px] text-danger px-3 py-2 break-all">{error()}</div>
        </Show>
      </div>
    </Show>
  );
};

/** One key's form row: label (with description tooltip) + control + unset. */
const KeyRow: Component<{
  k: HarnessStructuredKey;
  busy: boolean;
  onApply: (value: unknown) => void;
}> = (props) => {
  const k = () => props.k;
  return (
    <div class={rowClass}>
      {/* Label (omp catalog: description tooltip from the tool itself) */}
      <div
        class="w-[150px] shrink-0 text-xs text-white/60 truncate"
        title={k().description ?? k().key}
      >
        {labelFor(k())}
      </div>

      {/* Control by kind */}
      <div class="flex-1 min-w-0 flex items-center gap-2 flex-wrap">
        <Show
          when={k().kind === 'string' || k().kind === 'enum'}
          fallback={
            <Show
              when={k().kind === 'bool'}
              fallback={
                <Show
                  when={k().kind === 'number'}
                  fallback={
                    <Show
                      when={k().kind === 'string-list'}
                      fallback={
                        <KeyValueEditor
                          k={k()}
                          busy={props.busy}
                          onApply={(v) => props.onApply(v)}
                        />
                      }
                    >
                      <StringListEditor
                        k={k()}
                        busy={props.busy}
                        onApply={(v) => props.onApply(v)}
                      />
                    </Show>
                  }
                >
                  <NumberEditor k={k()} busy={props.busy} onApply={(v) => props.onApply(v)} />
                </Show>
              }
            >
              <Switch checked={scalar(k().value) === true} onChange={(v) => props.onApply(v)} />
            </Show>
          }
        >
          <Show
            when={k().kind === 'enum'}
            fallback={<TextEditor k={k()} busy={props.busy} onApply={(v) => props.onApply(v)} />}
          >
            <Dropdown
              variant="ghost"
              class="w-[170px]"
              value={(scalar(k().value) as string) ?? ''}
              onChange={(v) => props.onApply(v === '' ? null : v)}
              options={[
                { value: '', label: t('harness.key.unset') },
                ...k().options.map((o) => ({ value: o, label: o })),
              ]}
            />
          </Show>
        </Show>
      </div>

      {/* Unset (revert to tool default) — only when a value is set. */}
      <Show when={scalar(k().value) !== undefined && scalar(k().value) !== null}>
        <button
          class="text-white/25 hover:text-white/70 transition-colors cursor-pointer border-none bg-transparent p-1 shrink-0"
          title={t('harness.key.restoreDefault')}
          onClick={() => props.onApply(null)}
        >
          <Icon name="rotate-ccw" size={12} />
        </button>
      </Show>
    </div>
  );
};

/** Single-line text (string keys): commit on Enter/blur; empty = unset. */
const TextEditor: Component<{
  k: HarnessStructuredKey;
  busy: boolean;
  onApply: (value: string | null) => void;
}> = (props) => {
  const [draft, setDraft] = createSignal<string | null>(null);
  const shown = () => (draft() === null ? ((scalar(props.k.value) as string) ?? '') : draft()!);
  const commit = () => {
    const text = shown().trim();
    const current = (scalar(props.k.value) as string) ?? '';
    if (text !== current) props.onApply(text === '' ? null : text);
    setDraft(null);
  };
  return (
    <input
      class="flex-1 min-w-[160px] max-w-[320px] rounded-lg bg-black/20 border border-[var(--border-dim)] px-3 py-1.5 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors"
      placeholder={t('harness.key.unset')}
      disabled={props.busy}
      value={shown()}
      onInput={(e) => setDraft(e.currentTarget.value)}
      onBlur={() => commit()}
      onKeyDown={(e) => {
        if (e.key === 'Enter') commit();
        if (e.key === 'Escape') setDraft(null);
      }}
    />
  );
};

/** Number keys: commit on Enter/blur; empty = unset. */
const NumberEditor: Component<{
  k: HarnessStructuredKey;
  busy: boolean;
  onApply: (value: number | null) => void;
}> = (props) => {
  const [draft, setDraft] = createSignal<string | null>(null);
  const shown = () =>
    draft() === null
      ? scalar(props.k.value) !== undefined
        ? String(scalar(props.k.value))
        : ''
      : draft()!;
  const commit = () => {
    const text = shown().trim();
    const current = scalar(props.k.value);
    const parsed = text === '' ? null : Number(text);
    const same = parsed === null ? current === undefined : current === parsed;
    if (!same) props.onApply(parsed);
    setDraft(null);
  };
  return (
    <input
      type="number"
      class="w-[120px] rounded-lg bg-black/20 border border-[var(--border-dim)] px-3 py-1.5 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors"
      placeholder={t('harness.key.unset')}
      disabled={props.busy}
      value={shown()}
      onInput={(e) => setDraft(e.currentTarget.value)}
      onBlur={() => commit()}
      onKeyDown={(e) => {
        if (e.key === 'Enter') commit();
        if (e.key === 'Escape') setDraft(null);
      }}
    />
  );
};

/** Rule chips (string-list keys): add/remove applies immediately. */
const StringListEditor: Component<{
  k: HarnessStructuredKey;
  busy: boolean;
  onApply: (value: string[] | null) => void;
}> = (props) => {
  const [draft, setDraft] = createSignal('');
  const items = () => (props.k.value.kind === 'strList' ? props.k.value.value : []);
  const add = () => {
    const text = draft().trim();
    if (!text || items().includes(text)) return;
    props.onApply([...items(), text]);
    setDraft('');
  };
  const remove = (item: string) => {
    const next = items().filter((x) => x !== item);
    props.onApply(next.length === 0 ? null : next);
  };
  return (
    <div class="flex-1 min-w-0 flex flex-col gap-1.5">
      <div class="flex flex-wrap gap-1.5">
        <For each={items()}>
          {(item) => (
            <span class="inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-mono border border-white/10 bg-white/[0.04] text-white/70">
              <span class="max-w-[260px] truncate">{item}</span>
              <button
                class="text-white/30 hover:text-danger transition-colors cursor-pointer border-none bg-transparent p-0 flex items-center"
                disabled={props.busy}
                onClick={() => remove(item)}
              >
                <Icon name="x" size={10} />
              </button>
            </span>
          )}
        </For>
      </div>
      <div class="flex items-center gap-1.5">
        <input
          class="flex-1 min-w-[140px] max-w-[320px] rounded-lg bg-black/20 border border-[var(--border-dim)] px-3 py-1.5 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors font-mono"
          placeholder={t('harness.key.rulePlaceholder')}
          disabled={props.busy}
          value={draft()}
          onInput={(e) => setDraft(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') add();
          }}
        />
        <button class={btnBadge} disabled={props.busy || !draft().trim()} onClick={add}>
          {t('harness.key.addItem')}
        </button>
      </div>
    </div>
  );
};

/** Key/value pairs (env keys): local edit + per-block save. */
const KeyValueEditor: Component<{
  k: HarnessStructuredKey;
  busy: boolean;
  onApply: (value: Record<string, string> | null) => void;
}> = (props) => {
  const [rows, setRows] = createSignal<[string, string][]>([]);
  const [dirty, setDirty] = createSignal(false);
  const [savedTick, setSavedTick] = createSignal(false);
  onMount(() => {
    const current = props.k.value.kind === 'kv' ? props.k.value.value : [];
    if (current.length > 0) setRows(current.map(([a, b]) => [a, b]));
  });
  const save = () => {
    const cleaned = rows().filter(([a, b]) => a.trim() && b.trim());
    const obj: Record<string, string> = {};
    for (const [a, b] of cleaned) obj[a.trim()] = b.trim();
    props.onApply(Object.keys(obj).length === 0 ? null : obj);
    setDirty(false);
    setSavedTick(true);
    setTimeout(() => setSavedTick(false), 1500);
  };
  return (
    <div class="flex-1 min-w-0 flex flex-col gap-1.5">
      <For each={rows()}>
        {(_, i) => {
          const name = () => rows()[i()]?.[0] ?? '';
          const val = () => rows()[i()]?.[1] ?? '';
          return (
            <div class="flex items-center gap-1.5">
              <input
                class="w-[38%] rounded-lg bg-black/20 border border-[var(--border-dim)] px-3 py-1.5 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors font-mono"
                placeholder={t('harness.key.kvKey')}
                value={name()}
                onInput={(e) => {
                  setRows((prev) =>
                    prev.map((r, j) => (j === i() ? [e.currentTarget.value, r[1]] : r)),
                  );
                  setDirty(true);
                }}
              />
              <input
                class="flex-1 rounded-lg bg-black/20 border border-[var(--border-dim)] px-3 py-1.5 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors font-mono"
                placeholder={t('harness.key.kvValue')}
                value={val()}
                onInput={(e) => {
                  setRows((prev) =>
                    prev.map((r, j) => (j === i() ? [r[0], e.currentTarget.value] : r)),
                  );
                  setDirty(true);
                }}
              />
              <button
                class="text-white/30 hover:text-danger transition-colors cursor-pointer border-none bg-transparent p-1 flex items-center"
                onClick={() => {
                  setRows((prev) => prev.filter((_, j) => j !== i()));
                  setDirty(true);
                }}
              >
                <Icon name="x" size={11} />
              </button>
            </div>
          );
        }}
      </For>
      <div class="flex items-center gap-1.5">
        <button
          class={btnBadge}
          onClick={() => {
            setRows((prev) => [...prev, ['', '']]);
            setDirty(true);
          }}
        >
          <Icon name="plus" size={11} /> {t('harness.key.addItem')}
        </button>
        <Show when={dirty()}>
          <button class={btnPrimaryOutlineSm} disabled={props.busy} onClick={save}>
            {savedTick() ? t('common.saved') : t('common.save')}
          </button>
        </Show>
      </div>
    </div>
  );
};

export default HarnessToolStructuredForm;
