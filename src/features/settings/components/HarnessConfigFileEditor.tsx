/**
 * One config-file editor card (harness tool settings): loads the slot's
 * content, edits it as a raw text area, and saves through the validated
 * write path (format re-parse → one-time pre-AIO backup → stale-mtime guard →
 * atomic write). The raw editor is the parity escape hatch: anything the CLI
 * can configure, this can edit byte-faithfully.
 */
import { Component, createSignal, onMount, Show } from 'solid-js';
import {
  btnDangerOutline,
  btnPrimaryOutlineSm,
  btnSecondarySm,
} from '../../../shared/components/buttonStyles';
import {
  harnessToolConfigRead,
  harnessToolConfigRestore,
  harnessToolConfigWrite,
  type HarnessToolConfigSlot,
} from '../../../core/api/harness';
import { t } from '../../../core/i18n';

interface HarnessConfigFileEditorProps {
  toolId: string;
  slot: HarnessToolConfigSlot;
}

const HarnessConfigFileEditor: Component<HarnessConfigFileEditorProps> = (props) => {
  const [content, setContent] = createSignal('');
  const [loadedModified, setLoadedModified] = createSignal<number | null>(null);
  const [savedTick, setSavedTick] = createSignal(false);
  const [restoredTick, setRestoredTick] = createSignal(false);
  const [error, setError] = createSignal('');
  const [hasBackup, setHasBackup] = createSignal(false);

  const reload = async () => {
    setError('');
    try {
      const d = await harnessToolConfigRead(props.toolId, props.slot.slotId);
      setContent(d.content);
      setLoadedModified(d.modified ?? null);
    } catch (e) {
      setError(String(e));
    }
  };
  onMount(() => {
    setHasBackup(props.slot.hasBackup);
    void reload();
  });

  const save = async () => {
    setError('');
    try {
      await harnessToolConfigWrite(
        props.toolId,
        props.slot.slotId,
        content(),
        loadedModified() ?? undefined,
      );
      setSavedTick(true);
      setTimeout(() => setSavedTick(false), 1500);
      await reload();
      setHasBackup(true);
    } catch (e) {
      setError(String(e));
    }
  };

  const restore = async () => {
    setError('');
    try {
      await harnessToolConfigRestore(props.toolId, props.slot.slotId);
      setRestoredTick(true);
      setTimeout(() => setRestoredTick(false), 1500);
      await reload();
    } catch (e) {
      setError(String(e));
    }
  };

  const formatBadge = () => props.slot.format.toUpperCase();

  return (
    <div
      class="rounded-xl p-4"
      style={{
        background: 'rgba(var(--text-base-rgb),0.03)',
        border: '1px solid var(--border-dim)',
      }}
    >
      {/* Card header: label + format badge + path + state */}
      <div class="flex items-center gap-2 mb-1 flex-wrap">
        <span class="text-sm font-medium text-white/85">{props.slot.label}</span>
        <span
          class="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-mono border border-white/10"
          style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}
        >
          {formatBadge()}
        </span>
        <Show when={!props.slot.exists}>
          <span class="text-[11px] text-warn">{t('harness.conf.slotMissing')}</span>
        </Show>
      </div>
      <div class="text-[11px] text-white/35 mb-2 break-all font-mono" title={props.slot.path}>
        {props.slot.path}
      </div>

      {/* Note: YAML comment loss warning */}
      <div class="items-center gap-3 mb-2 text-[11px] text-white/35 flex">
        <Show when={props.slot.commentsLostOnWrite}>
          <span class="text-warn">{t('harness.conf.commentsLost')}</span>
        </Show>
      </div>

      {/* Editor */}
      <textarea
        class="w-full rounded-lg bg-[rgba(0,0,0,0.25)] border border-[var(--border-dim)] px-3 py-2 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors resize-y min-h-[220px] font-mono leading-relaxed"
        spellcheck={false}
        placeholder={t('harness.conf.editorPlaceholder')}
        value={content()}
        onInput={(e) => setContent(e.currentTarget.value)}
      />

      {/* Actions */}
      <div class="flex items-center gap-2 mt-2">
        <button class={btnPrimaryOutlineSm} onClick={() => void save()}>
          {savedTick() ? t('common.saved') : t('common.save')}
        </button>
        <button class={btnSecondarySm} onClick={() => void reload()}>
          {t('harness.conf.reload')}
        </button>
        <Show when={hasBackup()}>
          <button class={btnDangerOutline} onClick={() => void restore()}>
            {restoredTick() ? t('common.saved') : t('harness.conf.restoreBackup')}
          </button>
        </Show>
      </div>

      <Show when={error()}>
        <div class="text-[11px] text-danger mt-2 break-all">{error()}</div>
      </Show>
    </div>
  );
};

export default HarnessConfigFileEditor;
