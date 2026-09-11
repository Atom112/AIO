// 本地推理引擎域 Tauri 命令封装（与 commands/engine.rs 对应）。
import { invoke } from '@tauri-apps/api/core';
import type { EngineScanEntry } from '../store/engine';

/** 本地引擎启动参数。 */
export interface StartLocalEngineParams {
  modelPath: string;
  port: number;
  gpuLayers: number;
  engineType: string;
  trustRemoteCode: boolean;
}

/** 指定类型的引擎是否已在运行。 */
export const isLocalServerRunning = (engineType: string) =>
  invoke<boolean>('is_local_server_running', { engineType });

/** 启动一个本地推理引擎进程。 */
export const startLocalServer = (params: StartLocalEngineParams) =>
  invoke<void>('start_local_server', params as unknown as Record<string, unknown>);

/** 停止指定类型的本地引擎进程。 */
export const stopLocalServer = (engineType: string) =>
  invoke<void>('stop_local_server', { engineType });

/** 探测外部/远端引擎健康（apiUrl + engineType 判定健康端点；engineType 为 null 表示云端）。 */
export const probeEngineHealth = (apiUrl: string, engineType: string | null) =>
  invoke<boolean>('probe_engine_health', { apiUrl, engineType });

/** 扫描系统上已安装的推理引擎（应用启动时调用一次）。 */
export const scanInstalledEngines = () => invoke<EngineScanEntry[]>('scan_installed_engines');
