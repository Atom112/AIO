// Visibility rules for conversations that can be shared with a team workspace.
// Kept pure so the chat UI and tests share one definition.

export interface TopicVisibilityInput {
  ownerUserId?: string | null;
  visibility?: 'private' | 'workspace';
}

export interface SessionVisibilityInput {
  signedIn: boolean;
  workspaceId: string | null;
  userId: string | null;
}

/**
 * A conversation is read-only when it was shared by another member.
 *
 * Local-only conversations (`ownerUserId` absent) stay editable, and a signed-out
 * installation keeps full local control over its own copies.
 */
export function isTopicReadOnly(
  topic: TopicVisibilityInput | null | undefined,
  session: SessionVisibilityInput,
): boolean {
  if (!topic?.ownerUserId) return false;
  if (!session.signedIn || !session.userId) return false;
  return topic.ownerUserId !== session.userId;
}

/** Only the owner may share or un-share; unbound installations cannot share at all. */
export function canToggleTopicVisibility(
  topic: TopicVisibilityInput | null | undefined,
  session: SessionVisibilityInput,
): boolean {
  if (!session.signedIn || !session.workspaceId || !topic) return false;
  return !topic.ownerUserId || topic.ownerUserId === session.userId;
}

/** Cloud runs need a shared conversation owned by the caller. */
export function canRunTopicInCloud(
  topic: TopicVisibilityInput | null | undefined,
  session: SessionVisibilityInput,
): boolean {
  if (!session.signedIn || !session.workspaceId) return false;
  if (topic?.visibility !== 'workspace') return false;
  return !topic.ownerUserId || topic.ownerUserId === session.userId;
}
