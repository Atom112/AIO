import { describe, expect, it } from 'vitest';
import { invitationExpiryTime } from './cloudInvitations';

describe('invitation expiry label', () => {
  it('renders a UTC timestamp with seconds', () => {
    expect(invitationExpiryTime('2026-12-01T08:30:00.000Z')).toBe('2026-12-01 08:30:00 UTC');
  });

  it('returns the raw value when it cannot be parsed', () => {
    expect(invitationExpiryTime('not-a-date')).toBe('not-a-date');
  });
});
