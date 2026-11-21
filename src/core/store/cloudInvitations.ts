// Presentation helpers for pending team invitations. The list itself lives in the
// cloud store; keeping the formatting pure makes it testable outside a Tauri runtime.

/** Expiry label for an invitation, always rendered in UTC for a stable string. */
export function invitationExpiryTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date
    .toISOString()
    .replace('T', ' ')
    .replace(/\.\d{3}Z$/, ' UTC');
}
