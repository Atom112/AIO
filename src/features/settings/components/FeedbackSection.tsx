/**
 * FeedbackSection — the "About & Feedback" block in App Settings.
 *
 * Phase 0 of the alpha feedback plan (no server, no website): everything flows
 * through GitHub. The user can:
 * - Copy the privacy-safe environment snapshot (single line, no keys)
 * - Open a pre-filled GitHub Issue (bug / feature / alpha feedback) or
 *   Discussions in the system browser
 * - Export a diagnostic zip (snapshot + config slot states) to Downloads and
 *   attach it to an issue themselves — nothing is uploaded automatically.
 */
import { Component, createSignal, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import { btnSecondarySm, btnPrimaryOutlineSm } from '../../../shared/components/buttonStyles';
import {
  getFeedbackSnapshot,
  exportDiagnosticPackage,
  type FeedbackSnapshot,
} from '../../../core/api/feedback';
import { openSafeUrl } from '../../../core/utils/safeUrl';
import { locale, t } from '../../../core/i18n';

const REPO_URL = 'https://github.com/Atom112/AIO';

const FeedbackSection: Component = () => {
  const [snapshot, setSnapshot] = createSignal<FeedbackSnapshot | null>(null);
  const [copied, setCopied] = createSignal(false);
  const [exported, setExported] = createSignal<string | null>(null);
  const [exporting, setExporting] = createSignal(false);

  const loadSnapshot = async () => {
    try {
      setSnapshot(await getFeedbackSnapshot(locale()));
    } catch (e) {
      console.error('[feedback] snapshot failed:', e);
    }
  };

  // Load on mount (cheap: one sysinfo pass + probe cache hits)
  void loadSnapshot();

  const copySnapshot = () => {
    const s = snapshot()?.summary;
    if (!s) return;
    navigator.clipboard.writeText(s).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  const openLink = (path: string) => {
    void openSafeUrl(`${REPO_URL}${path}`);
  };

  const openBugReport = () => {
    const s = snapshot()?.summary ?? '';
    openLink(`/issues/new?template=bug_report.yml&title=${encodeURIComponent('[bug] ')}`);
    // Snapshot goes to the clipboard as a convenience (the form's first field
    // says to paste it there).
    if (s) navigator.clipboard.writeText(s).catch(() => undefined);
  };

  const openAlphaFeedback = () => {
    openLink(`/issues/new?template=alpha_feedback.yml`);
  };

  const openFeatureRequest = () => {
    openLink(`/issues/new?template=feature_request.yml`);
  };

  const openDiscussions = () => {
    openLink(`/discussions`);
  };

  const handleExport = async () => {
    if (exporting()) return;
    setExporting(true);
    setExported(null);
    try {
      const path = await exportDiagnosticPackage(locale());
      setExported(path);
      setTimeout(() => setExported(null), 4000);
    } catch (e) {
      console.error('[feedback] export failed:', e);
    } finally {
      setExporting(false);
    }
  };

  return (
    <div
      class="rounded-xl p-5 mt-4"
      style={{
        background: 'rgba(var(--text-base-rgb),0.03)',
        border: '1px solid var(--border-dim)',
      }}
    >
      {/* Title */}
      <div class="flex items-center gap-2 mb-3">
        <Icon name="chat" size={16} class="text-white/50" />
        <span class="text-sm font-medium text-white/80">{t('feedback.title')}</span>
      </div>

      {/* Environment snapshot */}
      <div class="mb-4">
        <div class="text-[11px] text-white/40 mb-1">{t('feedback.envHint')}</div>
        <div
          class="flex items-center gap-2 rounded-lg px-3 py-2 font-mono text-[11px] text-white/55 cursor-pointer transition-colors hover:text-white/75"
          style={{
            background: 'rgba(var(--text-base-rgb),0.04)',
            border: '1px solid var(--border-dim)',
          }}
          onClick={copySnapshot}
          title={t('feedback.copySnapshot')}
        >
          <span class="flex-1 truncate">{snapshot()?.summary ?? t('feedback.loading')}</span>
          <Icon name={copied() ? 'check' : 'copy'} size={13} class="shrink-0 text-white/30" />
        </div>
      </div>

      {/* GitHub channel buttons */}
      <div class="flex flex-wrap gap-2 mb-4">
        <button class={btnPrimaryOutlineSm} onClick={openBugReport}>
          <Icon name="bug" size={12} /> {t('feedback.reportBug')}
        </button>
        <button class={btnSecondarySm} onClick={openAlphaFeedback}>
          <Icon name="sparkles" size={12} /> {t('feedback.alphaFeedback')}
        </button>
        <button class={btnSecondarySm} onClick={openFeatureRequest}>
          <Icon name="lightbulb" size={12} /> {t('feedback.featureRequest')}
        </button>
        <button class={btnSecondarySm} onClick={openDiscussions}>
          <Icon name="chat" size={12} /> {t('feedback.discussions')}
        </button>
      </div>

      {/* Diagnostic export */}
      <div class="flex items-center gap-3">
        <button class={btnSecondarySm} onClick={() => void handleExport()} disabled={exporting()}>
          <Icon name="download" size={12} />
          {exporting() ? t('feedback.exporting') : t('feedback.exportDiagnostics')}
        </button>
        <Show when={exported()}>
          <span class="text-[11px] text-white/35 truncate max-w-[300px]">
            {t('feedback.exportedTo')} {exported()}
          </span>
        </Show>
      </div>
      <div class="text-[10px] text-white/25 mt-1.5 leading-4">{t('feedback.privacyNote')}</div>
    </div>
  );
};

export default FeedbackSection;
