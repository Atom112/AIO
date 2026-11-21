import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CloudAnnouncement } from '../api/cloud';

// The module reads localStorage lazily, but a memory stub must still exist before
// the imports resolve so no test depends on a browser environment.
const memory = new Map<string, string>();
vi.stubGlobal('localStorage', {
  getItem: (key: string) => memory.get(key) ?? null,
  setItem: (key: string, value: string) => void memory.set(key, value),
  removeItem: (key: string) => void memory.delete(key),
});

const { dismissedAnnouncementIds, dismissAnnouncement, pickAnnouncement } =
  await import('./cloudAnnouncements');

const DISMISSED_KEY = 'cloud-dismissed-announcements';

function announcement(id: string, startsAt: string): CloudAnnouncement {
  return {
    id,
    severity: 'info',
    scope: 'server',
    workspaceId: null,
    title: `Notice ${id}`,
    body: 'body',
    startsAt,
    endsAt: null,
  };
}

beforeEach(() => {
  memory.clear();
});

describe('picking the visible announcement', () => {
  it('returns the newest by start time', () => {
    const items = [
      announcement('older', '2026-01-01T00:00:00.000Z'),
      announcement('newest', '2026-03-01T00:00:00.000Z'),
      announcement('middle', '2026-02-01T00:00:00.000Z'),
    ];
    expect(pickAnnouncement(items, [])?.id).toBe('newest');
  });

  it('skips dismissed ids and returns null when everything is hidden', () => {
    const items = [
      announcement('a', '2026-01-01T00:00:00.000Z'),
      announcement('b', '2026-02-01T00:00:00.000Z'),
    ];
    expect(pickAnnouncement(items, ['b'])?.id).toBe('a');
    expect(pickAnnouncement(items, ['a', 'b'])).toBeNull();
  });

  it('returns null for an empty list', () => {
    expect(pickAnnouncement([], [])).toBeNull();
  });
});

describe('dismissal bookkeeping', () => {
  it('appends without duplicating an id', () => {
    dismissAnnouncement('a');
    dismissAnnouncement('b');
    dismissAnnouncement('a');
    expect(dismissedAnnouncementIds()).toEqual(['b', 'a']);
  });

  it('tolerates a corrupt stored value', () => {
    memory.set(DISMISSED_KEY, 'not-json');
    expect(dismissedAnnouncementIds()).toEqual([]);
    // A dismissal after the corrupt value still persists cleanly.
    dismissAnnouncement('a');
    expect(dismissedAnnouncementIds()).toEqual(['a']);
  });
});
