// Profile-level model overrides + custom subagent profiles (depends only on invoke).

import { createSignal } from 'solid-js';
import {
  deleteCustomSubagentProfile as apiDeleteCustomSubagentProfile,
  listCustomSubagentProfiles as apiListCustomSubagentProfiles,
  loadProfileModelOverrides as apiLoadProfileModelOverrides,
  saveCustomSubagentProfile as apiSaveCustomSubagentProfile,
  saveProfileModelOverrides as apiSaveProfileModelOverrides,
} from '../api/profiles';
import { modelKey } from './models';
import { allAvailableModels } from './chat';

// ====== Per-Profile Model Overrides ======

/** Profile model override loaded from the backend. */
export interface ProfileModelOverrideBackend {
  profileId: string;
  modelId: string;
  apiUrl: string;
  apiKey: string;
}

/**
 * Per-profile model override map.
 * Key = profile id ("explorer" | "coder" | "general"),
 * Value = composite modelKey (model_id@api_url). Empty string = no override (use parent model).
 */
export const [profileModelOverrides, setProfileModelOverrides] = createSignal<
  Record<string, string>
>({});

/** Load per-profile model overrides from the backend at app startup. */
export const initProfileModelOverrides = async () => {
  try {
    const overrides = await apiLoadProfileModelOverrides();
    const map: Record<string, string> = {};
    for (const o of overrides) {
      if (o.profileId && o.modelId) {
        map[o.profileId] = o.modelId.includes('@') ? o.modelId : `${o.modelId}@${o.apiUrl}`;
      }
    }
    setProfileModelOverrides(map);
  } catch (e) {
    console.warn('Failed to load profile model overrides:', e);
  }
};

/** Persist the current per-profile model overrides to the backend. */
export const saveProfileModelOverrides = async () => {
  const map = profileModelOverrides();
  const overrides: ProfileModelOverrideBackend[] = [];
  for (const [profileId, mKey] of Object.entries(map)) {
    if (!mKey) continue;
    const model = allAvailableModels().find((m) => modelKey(m) === mKey);
    if (model) {
      overrides.push({
        profileId,
        modelId: model.model_id,
        apiUrl: model.api_url,
        // FE-12: never persist the API key — the backend only needs profile → model
        // identity on load; the live key is resolved fresh from the provider config
        // at every turn (ChatPage passes resolvedOverrides with the current key).
        apiKey: '',
      });
    }
  }
  await apiSaveProfileModelOverrides(overrides);
};

// ====== Custom Subagent Profiles ======

/** User-defined subagent profile. */
export interface CustomSubagentProfile {
  id: string;
  name: string;
  description: string;
  allowedTools: string[];
  deniedTools: string[];
  systemPromptExtension: string;
}

export const [customSubagentProfiles, setCustomSubagentProfiles] = createSignal<
  CustomSubagentProfile[]
>([]);

/** Load custom subagent profiles from the backend at app startup. */
export const initCustomSubagentProfiles = async () => {
  try {
    const list = await apiListCustomSubagentProfiles();
    setCustomSubagentProfiles(list);
  } catch (e) {
    console.warn('Failed to load custom subagent profiles:', e);
  }
};

/** Save (insert or update) a custom subagent profile. */
export const saveCustomSubagentProfile = async (profile: CustomSubagentProfile) => {
  await apiSaveCustomSubagentProfile(profile);
  const list = await apiListCustomSubagentProfiles();
  setCustomSubagentProfiles(list);
};

/** Delete a custom subagent profile by id. */
export const deleteCustomSubagentProfile = async (profileId: string) => {
  await apiDeleteCustomSubagentProfile(profileId);
  setCustomSubagentProfiles((prev) => prev.filter((p) => p.id !== profileId));
};
