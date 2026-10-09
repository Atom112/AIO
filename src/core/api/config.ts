// 应用配置域 Tauri 命令封装（与 commands/config.rs 对应）。
import { invoke } from '@tauri-apps/api/core';
import type { ActivatedModel, AppConfig } from '../store/types';

/** 读取应用配置（读取失败由调用方决定回退策略）。 */
export const loadAppConfig = () => invoke<AppConfig>('load_app_config');

/** 保存应用配置。 */
export const saveAppConfig = (config: AppConfig) =>
  invoke<void>('save_app_config', { config: config as unknown as Record<string, unknown> });

/** 同步前端 locale 到后端（错误消息等用户可见文案的语言）。 */
export const setBackendLocale = (locale: string) => invoke<void>('set_locale', { locale });

/** 设置系统自启动（true = 随系统启动）。 */
export const setAutoStart = (enabled: boolean) => invoke<void>('set_auto_start', { enabled });

/** 加载激活模型列表（含本地引擎注入的条目）。 */
export const loadActivatedModels = () => invoke<ActivatedModel[]>('load_activated_models');

/** 加载 provider 配置文件（含自定义供应商与模型）。 */
export const loadProviderConfigs = () =>
  invoke<{ providers: Record<string, import('../utils/models').ProviderConfig> }>(
    'load_provider_configs',
  );
