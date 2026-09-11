// 应用更新域 Tauri 命令封装（与 commands/update.rs 对应）。
// CheckUpdateResult 此前在 Layout.tsx 与 AppSettings.tsx 中各自重复定义，现收敛于此。
import { invoke } from '@tauri-apps/api/core';

/** 可用更新信息（镜像 commands/update.rs 的 AppUpdateInfo）。 */
export interface AppUpdateInfo {
  /** 最新版本号（已去掉 v 前缀） */
  version: string;
  currentVersion: string;
  /** Release notes（Markdown 原文） */
  notes?: string;
  /** 发布时间（ISO8601） */
  pubDate?: string;
}

/**
 * check_app_update 的返回结果（镜像 commands/update.rs 的 CheckUpdateResult）。
 * 判别字段 kind：up_to_date / update_available / service_not_ready / network / failed。
 */
export type CheckUpdateResult =
  | { kind: 'up_to_date'; current_version: string }
  | { kind: 'update_available'; info: AppUpdateInfo }
  | { kind: 'service_not_ready'; current_version: string; endpoint: string; reason: string }
  | { kind: 'network'; current_version: string; endpoint: string; reason: string }
  | { kind: 'failed'; current_version: string; endpoint: string; reason: string };

/** 检查应用更新。 */
export const checkAppUpdate = () => invoke<CheckUpdateResult>('check_app_update');

/** 下载并安装更新（进度经 app-update-progress 事件回报）。 */
export const installAppUpdate = () => invoke<void>('install_app_update');

/** 重启应用（安装更新后）。 */
export const restartApp = () => invoke<void>('restart_app');
