// 档案级模型覆盖 + 自定义子智能体档案 —— 从 store.ts 拆出（仅依赖 invoke）。

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

/** 从后端加载的 profile 模型覆盖信息 */
export interface ProfileModelOverrideBackend {
  profileId: string;
  modelId: string;
  apiUrl: string;
  apiKey: string;
}

/**
 * Per-profile 模型覆盖映射。
 * Key = profile id ("explorer" | "coder" | "general")，
 * Value = 复合键 modelKey (model_id@api_url)。空字符串 = 无覆盖（使用父模型）。
 */
export const [profileModelOverrides, setProfileModelOverrides] = createSignal<
  Record<string, string>
>({});

/** 应用启动时从后端加载 per-profile 模型覆盖配置 */
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

/** 持久化当前 per-profile 模型覆盖到后端 */
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
        apiKey: model.api_key,
      });
    }
  }
  await apiSaveProfileModelOverrides(overrides);
};

// ====== Custom Subagent Profiles ======

/** 用户自定义子智能体配置文件 */
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

/** 应用启动时从后端加载自定义子智能体配置文件 */
export const initCustomSubagentProfiles = async () => {
  try {
    const list = await apiListCustomSubagentProfiles();
    setCustomSubagentProfiles(list);
  } catch (e) {
    console.warn('Failed to load custom subagent profiles:', e);
  }
};

/** 保存（插入或更新）一个自定义子智能体配置文件 */
export const saveCustomSubagentProfile = async (profile: CustomSubagentProfile) => {
  await apiSaveCustomSubagentProfile(profile);
  const list = await apiListCustomSubagentProfiles();
  setCustomSubagentProfiles(list);
};

/** 按 id 删除一个自定义子智能体配置文件 */
export const deleteCustomSubagentProfile = async (profileId: string) => {
  await apiDeleteCustomSubagentProfile(profileId);
  setCustomSubagentProfiles((prev) => prev.filter((p) => p.id !== profileId));
};
