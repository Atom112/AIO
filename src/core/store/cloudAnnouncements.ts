// Dismissal bookkeeping for team announcements. Announcements are server-authored
// and non-critical: a failed fetch keeps the previous list, and dismissal is local
// to this installation so signing out never loses what the user already read.
import type { CloudAnnouncement } from '../api/cloud';

const DISMISSED_KEY = 'cloud-dismissed-announcements';
const MAX_DISMISSED = 100;

/** Reads dismissed announcement ids, tolerating a corrupt or missing entry. */
export function dismissedAnnouncementIds(): string[] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(DISMISSED_KEY) ?? '[]');
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((id): id is string => typeof id === 'string').slice(-MAX_DISMISSED);
  } catch {
    return [];
  }
}

/** Persists a dismissal, keeping the list bounded. */
export function dismissAnnouncement(id: string): void {
  const next = [...dismissedAnnouncementIds().filter((entry) => entry !== id), id].slice(
    -MAX_DISMISSED,
  );
  localStorage.setItem(DISMISSED_KEY, JSON.stringify(next));
}

/** Newest still-visible announcement, or null when everything is dismissed. */
export function pickAnnouncement(
  items: readonly CloudAnnouncement[],
  dismissed: readonly string[],
): CloudAnnouncement | null {
  const hidden = new Set(dismissed);
  const visible = items
    .filter((item) => !hidden.has(item.id))
    .sort((a, b) => Date.parse(b.startsAt) - Date.parse(a.startsAt));
  return visible[0] ?? null;
}
