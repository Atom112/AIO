/**
 * @file ProjectCreateModal.tsx
 * @description New-project modal: name input + directory picker. Supports enter/exit transition animations.
 */
import { createSignal, onMount } from 'solid-js';
import { createProject, validateProjectPath } from '../../../core/api/project';
import { open } from '@tauri-apps/plugin-dialog';
import { reportError, t } from '../../../core/i18n';
import { btnGhost, btnPrimaryOutline, btnSecondary } from '../../../shared/components/buttonStyles';

interface Props {
  onClose: () => void;
  onCreated: (projectId: string) => void;
}

export default function ProjectCreateModal(props: Props) {
  const [name, setName] = createSignal('');
  const [path, setPath] = createSignal('');
  const [error, setError] = createSignal('');
  const [loading, setLoading] = createSignal(false);
  const [isExiting, setIsExiting] = createSignal(false);
  const [isEntering, setIsEntering] = createSignal(true);

  // Enter animation: drop the entering state on the next frame
  onMount(() => {
    requestAnimationFrame(() => setIsEntering(false));
  });

  const handleClose = () => {
    setIsExiting(true);
    setTimeout(() => {
      setIsExiting(false);
      props.onClose();
    }, 300);
  };

  const handleSelectDir = async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (selected && typeof selected === 'string') {
        setPath(selected);
        // Auto-fill the name with the directory name
        const dirName = selected.split(/[/\\]/).filter(Boolean).pop() || '';
        if (!name()) setName(dirName);

        // Validate the path
        try {
          const result = await validateProjectPath(selected);
          if (!result.valid) {
            console.warn('[project] invalid path:', result.reason);
            setError(t('project.invalidPath'));
          } else {
            setPath(result.canonicalPath || selected);
            setError('');
          }
        } catch {
          setError('');
        }
      }
    } catch (e) {
      console.warn('Failed to pick directory:', e);
    }
  };

  const handleCreate = async () => {
    if (!name().trim()) {
      setError(t('project.requiredName'));
      return;
    }
    if (!path()) {
      setError(t('project.requiredPath'));
      return;
    }
    setLoading(true);
    setError('');
    try {
      const project = await createProject(name().trim(), path());
      props.onCreated(project.id);
    } catch (e) {
      setError(reportError('error.save', e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div
      classList={{
        'opacity-0 pointer-events-none': isExiting() || isEntering(),
        'opacity-100': !isExiting() && !isEntering(),
      }}
      class="fixed inset-0 z-[2000] flex items-center justify-center bg-black/25 transition-all duration-200 ease-out"
      onClick={handleClose}
    >
      <div
        classList={{
          'scale-95 opacity-0': isExiting() || isEntering(),
          'scale-100 opacity-100': !isExiting() && !isEntering(),
        }}
        class="backdrop-blur-[24px] border border-white/15 rounded-xl shadow-2xl w-full max-w-md p-6 transition-all duration-500 ease-out transform"
        style={{
          background: 'var(--dialog-bg)',
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 class="text-lg font-semibold text-white mb-4">{t('project.new')}</h2>

        {/* Project name */}
        <label class="block text-sm text-white/60 mb-1.5">{t('project.name')}</label>
        <input
          type="text"
          class="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white
                           text-sm placeholder-white/30 focus:outline-none focus:border-[var(--color-pri)] mb-4"
          placeholder={t('project.namePlaceholder')}
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
          onKeyDown={(e) => e.key === 'Enter' && handleCreate()}
        />

        {/* Project directory */}
        <label class="block text-sm text-white/60 mb-1.5">{t('project.path')}</label>
        <div class="flex gap-2 mb-1">
          <input
            type="text"
            class="flex-1 px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white
                               text-sm placeholder-white/30 focus:outline-none focus:border-[var(--color-pri)]"
            placeholder={t('project.pathPlaceholder')}
            value={path()}
            readOnly
          />
          <button class={btnSecondary} onClick={handleSelectDir}>
            {t('project.browse')}
          </button>
        </div>

        {/* Error message */}
        {error() && <p class="text-red-400 text-xs mb-3">{error()}</p>}

        <p class="text-white/30 text-xs mb-4">{t('project.aioFolderNotice')}</p>

        {/* Buttons */}
        <div class="flex justify-end gap-3">
          <button class={'px-4 py-2 text-sm ' + btnGhost} onClick={handleClose}>
            {t('common.cancel')}
          </button>
          <button class={btnPrimaryOutline} onClick={handleCreate} disabled={loading()}>
            {loading() ? t('project.creating') : t('project.create')}
          </button>
        </div>
      </div>
    </div>
  );
}
