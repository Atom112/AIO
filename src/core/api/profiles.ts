// 档案（profile）域 Tauri 命令封装：档案级模型覆盖 + 自定义子智能体档案。
import { invoke } from '@tauri-apps/api/core';
import type { CustomSubagentProfile, ProfileModelOverrideBackend } from '../store/profiles';

/** 读取档案级模型覆盖。 */
export const loadProfileModelOverrides = () =>
  invoke<ProfileModelOverrideBackend[]>('load_profile_model_overrides');

/** 保存档案级模型覆盖。 */
export const saveProfileModelOverrides = (overrides: ProfileModelOverrideBackend[]) =>
  invoke<void>('save_profile_model_overrides', { overrides });

/** 列出自定义子智能体档案。 */
export const listCustomSubagentProfiles = () =>
  invoke<CustomSubagentProfile[]>('list_custom_subagent_profiles');

/** 保存（新建或更新）一个自定义子智能体档案，返回最新列表。 */
export const saveCustomSubagentProfile = (profile: CustomSubagentProfile) =>
  invoke<CustomSubagentProfile[]>('save_custom_subagent_profile', { profile });

/** 删除一个自定义子智能体档案。 */
export const deleteCustomSubagentProfile = (profileId: string) =>
  invoke<void>('delete_custom_subagent_profile', { profileId });
