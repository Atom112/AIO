/**
 * Add-workspace modal — register a workspace for the currently selected harness
 * tool. Two sources: a folder picked via the native dialog, or one of the
 * existing AIO project paths (quick-add chips). Acrylic modal per AGENTS.md
 * (ProjectSettingsModal is the reference implementation).
 */
import { Component, createSignal, For, Show } from 'solid-js';
import { open } from '@tauri-apps/plugin-dialog';
import Icon from '../../../shared/components/Icon';
import { btnClose, btnPrimaryOutline, btnSecondary } from '../../../shared/components/buttonStyles';
import { projects } from '../../../core/store/store';
import { t } from '../../../core/i18n';

interface HarnessAddWorkspaceModalProps {
  onClose: () => void;
  onAdd: (path: string, name?: string) => Promise<void> | void;
}

const HarnessAddWorkspaceModal: Component<HarnessAddWorkspaceModalProps> = (props) => {
  const [path, setPath] = createSignal('');
  const [name, setName] = createSignal('');
  const [error, setError] = createSignal('');
  const [busy, setBusy] = createSignal(false);

  const pickFolder = async () => {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === 'string') {
      setPath(picked);
      setError('');
    }
  };

  const submit = async () => {
    const p = path().trim();
    if (!p) {
      setError(t('harness.addWorkspace.pathRequired'));
      return;
    }
    setBusy(true);
    try {
      await props.onAdd(p, name().trim() || undefined);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  /** AIO projects not yet registered under the current tool (quick-add). */
  const knownProjects = () => {
    const seen = new Set<string>();
    return projects().filter((p) => {
      if (seen.has(p.path)) return false;
      seen.add(p.path);
      return true;
    });
  };

  return (
    <div
      class="fixed inset-0 z-[1000] bg-black/25 flex items-center justify-center"
      onClick={() => props.onClose()}
    >
      <div
        class="rounded-xl px-6 py-5 max-w-[440px] w-[90%] relative"
        style={{
          background: 'var(--dialog-bg)',
          'backdrop-filter': 'var(--popup-blur)',
          border: '1px solid var(--border-dim)',
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <button class={`absolute top-3 right-3 ${btnClose}`} onClick={() => props.onClose()}>
          <Icon name="x" class="w-4 h-4" />
        </button>

        <div class="text-sm font-medium text-white/85 mb-4">{t('harness.addWorkspace.title')}</div>

        {/* Folder picker */}
        <div class="flex items-center gap-2 mb-2">
          <input
            class="flex-1 min-w-0 rounded-lg bg-[rgba(0,0,0,0.2)] border border-[var(--border-dim)] px-3 py-1.5 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors"
            placeholder={t('harness.addWorkspace.pathPlaceholder')}
            value={path()}
            onInput={(e) => setPath(e.currentTarget.value)}
          />
          <button
            class="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium bg-white/[0.06] border border-white/10 text-white/60 hover:bg-white/[0.1] hover:text-white/85 active:scale-95 transition-all duration-200 cursor-pointer shrink-0"
            onClick={() => void pickFolder()}
          >
            <Icon name="folder" size={12} />
            {t('harness.addWorkspace.pickFolder')}
          </button>
        </div>

        {/* Name (optional) */}
        <input
          class="w-full rounded-lg bg-[rgba(0,0,0,0.2)] border border-[var(--border-dim)] px-3 py-1.5 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors mb-3"
          placeholder={t('harness.addWorkspace.namePlaceholder')}
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
        />

        {/* Quick-add from AIO projects */}
        <Show when={knownProjects().length > 0}>
          <div class="text-[11px] text-white/35 mb-1.5">
            {t('harness.addWorkspace.fromProjects')}
          </div>
          <div class="flex flex-wrap gap-1.5 mb-3 max-h-[110px] overflow-y-auto">
            <For each={knownProjects()}>
              {(p) => (
                <button
                  class={`inline-flex items-center gap-1.5 px-2 py-1 rounded-md text-[11px] border transition-colors cursor-pointer ${
                    path() === p.path
                      ? 'border-[rgba(var(--primary-rgb),0.5)] bg-[rgba(var(--primary-rgb),0.1)] text-white'
                      : 'border-white/10 bg-white/[0.04] text-white/55 hover:text-white/85 hover:bg-white/[0.08]'
                  }`}
                  title={p.path}
                  onClick={() => {
                    setPath(p.path);
                    setError('');
                  }}
                >
                  <Icon name="folder" size={10} />
                  <span class="max-w-[160px] truncate">{p.name}</span>
                </button>
              )}
            </For>
          </div>
        </Show>

        <Show when={error()}>
          <div class="text-[11px] text-danger mb-3 break-all">{error()}</div>
        </Show>

        {/* Footer */}
        <div class="flex items-center justify-end gap-2">
          <button class={btnSecondary} onClick={() => props.onClose()}>
            {t('common.cancel')}
          </button>
          <button class={btnPrimaryOutline} disabled={busy()} onClick={() => void submit()}>
            {t('harness.addWorkspace.confirm')}
          </button>
        </div>
      </div>
    </div>
  );
};

export default HarnessAddWorkspaceModal;
