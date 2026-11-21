import { Show } from 'solid-js';
import {
  cloudAnnouncements,
  cloudAnnouncementDismissed,
  dismissCloudAnnouncement,
} from '../../core/store/cloud';
import { pickAnnouncement } from '../../core/store/cloudAnnouncements';
import { t } from '../../core/i18n';
import { Icon } from './Icon';

/** Newest undismissed team announcement, shown under the nav bar. */
export function AnnouncementBanner() {
  const current = () => pickAnnouncement(cloudAnnouncements(), cloudAnnouncementDismissed());
  const tone = (severity: string) =>
    severity === 'critical'
      ? 'border-danger/50 text-danger'
      : severity === 'warning'
        ? 'border-warn/50 text-warn'
        : 'border-pri-50 text-pri';

  return (
    <Show when={current()}>
      {(item) => (
        <div
          class={`mx-4 mt-2 flex items-start gap-3 rounded-lg border bg-white/[0.06] px-3 py-2 ${tone(item().severity)}`}
        >
          <div class="flex-1 min-w-0">
            <div class="flex items-center gap-2 text-xs font-medium">
              <span>{item().title}</span>
              <span class="text-[rgba(var(--text-base-rgb),0.45)]">
                {item().scope === 'server'
                  ? t('cloud.announcement.scope.server')
                  : t('cloud.announcement.scope.workspace')}
              </span>
            </div>
            <div class="mt-1 text-xs whitespace-pre-wrap text-[rgba(var(--text-base-rgb),0.88)]">
              {item().body}
            </div>
          </div>
          <button
            type="button"
            class="text-[rgba(var(--text-base-rgb),0.55)] hover:text-[rgba(var(--text-base-rgb),0.9)]"
            title={t('cloud.announcement.dismiss')}
            aria-label={t('cloud.announcement.dismiss')}
            onClick={() => dismissCloudAnnouncement(item().id)}
          >
            <Icon name="x" size={14} />
          </button>
        </div>
      )}
    </Show>
  );
}
