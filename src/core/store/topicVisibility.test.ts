import { describe, expect, it } from 'vitest';
import { canRunTopicInCloud, canToggleTopicVisibility, isTopicReadOnly } from './topicVisibility';

const bound = { signedIn: true, workspaceId: 'ws1', userId: 'me' };

describe('conversation visibility rules', () => {
  it("marks teammates' shared conversations read-only", () => {
    expect(isTopicReadOnly({ ownerUserId: 'other', visibility: 'workspace' }, bound)).toBe(true);
    expect(isTopicReadOnly({ ownerUserId: 'me', visibility: 'workspace' }, bound)).toBe(false);
    expect(isTopicReadOnly({ visibility: 'private' }, bound)).toBe(false);
    // Signed out: local copies stay fully editable.
    expect(
      isTopicReadOnly(
        { ownerUserId: 'other' },
        { signedIn: false, workspaceId: null, userId: null },
      ),
    ).toBe(false);
  });

  it('lets only the owner toggle sharing', () => {
    expect(canToggleTopicVisibility({ visibility: 'private' }, bound)).toBe(true);
    expect(canToggleTopicVisibility({ ownerUserId: 'me', visibility: 'workspace' }, bound)).toBe(
      true,
    );
    expect(canToggleTopicVisibility({ ownerUserId: 'other', visibility: 'workspace' }, bound)).toBe(
      false,
    );
    expect(
      canToggleTopicVisibility(
        { visibility: 'private' },
        { signedIn: true, workspaceId: null, userId: 'me' },
      ),
    ).toBe(false);
  });

  it('allows cloud runs only on shared conversations the caller owns', () => {
    expect(canRunTopicInCloud({ visibility: 'workspace' }, bound)).toBe(true);
    expect(canRunTopicInCloud({ ownerUserId: 'me', visibility: 'workspace' }, bound)).toBe(true);
    expect(canRunTopicInCloud({ ownerUserId: 'other', visibility: 'workspace' }, bound)).toBe(
      false,
    );
    expect(canRunTopicInCloud({ visibility: 'private' }, bound)).toBe(false);
    expect(canRunTopicInCloud({ visibility: 'workspace' }, { ...bound, workspaceId: null })).toBe(
      false,
    );
  });
});
