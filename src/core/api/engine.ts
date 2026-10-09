// Local inference engine domain Tauri command wrappers (mirrors commands/engine.rs).
import { invoke } from '@tauri-apps/api/core';
import type { EngineScanEntry } from '../store/engine';

/** Local engine start parameters. */
export interface StartLocalEngineParams {
  modelPath: string;
  port: number;
  gpuLayers: number;
  engineType: string;
  trustRemoteCode: boolean;
}

/** Whether an engine of the given type is already running. */
export const isLocalServerRunning = (engineType: string) =>
  invoke<boolean>('is_local_server_running', { engineType });

/** Start a local inference engine process. */
export const startLocalServer = (params: StartLocalEngineParams) =>
  invoke<void>('start_local_server', params as unknown as Record<string, unknown>);

/** Stop the local engine process of the given type. */
export const stopLocalServer = (engineType: string) =>
  invoke<void>('stop_local_server', { engineType });

/** Probe external/remote engine health (apiUrl + engineType decide the health endpoint; engineType null means cloud). */
export const probeEngineHealth = (apiUrl: string, engineType: string | null) =>
  invoke<boolean>('probe_engine_health', { apiUrl, engineType });

/** Scan the system for installed inference engines (called once at app startup). */
export const scanInstalledEngines = () => invoke<EngineScanEntry[]>('scan_installed_engines');
