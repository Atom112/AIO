// Profile domain Tauri command wrappers: profile-level model overrides + custom subagent profiles.
import { invoke } from '@tauri-apps/api/core';
import type { CustomSubagentProfile, ProfileModelOverrideBackend } from '../store/profiles';

/** Load profile-level model overrides. */
export const loadProfileModelOverrides = () =>
  invoke<ProfileModelOverrideBackend[]>('load_profile_model_overrides');

/** Save profile-level model overrides. */
export const saveProfileModelOverrides = (overrides: ProfileModelOverrideBackend[]) =>
  invoke<void>('save_profile_model_overrides', { overrides });

/** List custom subagent profiles. */
export const listCustomSubagentProfiles = () =>
  invoke<CustomSubagentProfile[]>('list_custom_subagent_profiles');

/** Save (create or update) a custom subagent profile; returns the latest list. */
export const saveCustomSubagentProfile = (profile: CustomSubagentProfile) =>
  invoke<CustomSubagentProfile[]>('save_custom_subagent_profile', { profile });

/** Delete a custom subagent profile. */
export const deleteCustomSubagentProfile = (profileId: string) =>
  invoke<void>('delete_custom_subagent_profile', { profileId });
