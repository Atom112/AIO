// 应用配置域 Tauri 命令封装（与 commands/config.rs 对应）。
import { invoke } from '@tauri-apps/api/core';
import type { ActivatedModel, AppConfig } from '../store/types';

/** 读取应用配置（读取失败由调用方决定回退策略）。 */
export const loadAppConfig = () => invoke<AppConfig>('load_app_config');

/** 保存应用配置。 */
export const saveAppConfig = (config: AppConfig) =>
  invoke<void>('save_app_config', { config: config as unknown as Record<string, unknown> });

/** 设置系统自启动（true = 随系统启动）。 */
export const setAutoStart = (enabled: boolean) => invoke<void>('set_auto_start', { enabled });

/** 读取头像数据源（返回 data URL）。 */
export const readAvatarSource = (path: string) => invoke<string>('read_avatar_source', { path });

/** 上传（保存）用户头像，返回落盘路径。 */
export const uploadAvatar = (dataUrl: string) => invoke<string>('upload_avatar', { dataUrl });

/** 加载激活模型列表（含本地引擎注入的条目）。 */
export const loadActivatedModels = () => invoke<ActivatedModel[]>('load_activated_models');

/** 加载 provider 配置文件（含自定义供应商与模型）。 */
export const loadProviderConfigs = () =>
  invoke<{ providers: Record<string, import('../utils/models').ProviderConfig> }>(
    'load_provider_configs',
  );
