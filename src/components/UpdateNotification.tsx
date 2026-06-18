import { Component, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Transition } from 'solid-transition-group';
import Icon from './Icon';
import {
    appUpdateAvailable,
    appUpdateInfo,
    appUpdateDismissed,
    setAppUpdateDismissed,
    appUpdateDownloading,
    setAppUpdateDownloading,
    appUpdateProgress,
    setAppUpdateProgress,
    appUpdateReady,
    setAppUpdateReady,
    setIgnoredUpdateVersion,
} from '../store/store';

/**
 * 左下角应用更新提示 Toast
 *
 * 显示条件：
 * 1. 启动时或手动检查发现新版本 (appUpdateAvailable() = true)
 * 2. 用户在本会话中尚未手动关闭 (appUpdateDismissed() = false)
 * 3. 未处于"已是最新"的隐藏态
 *
 * 操作：
 * - 立即更新 → 调用后端 `install_app_update`，期间显示进度条
 * - 稍后     → 记录到 localStorage 并关闭浮层
 * - 重启应用 → 下载完成后调用 `restart_app`
 */
const UpdateNotification: Component = () => {
    const [error, setError] = createSignal<string | null>(null);
    let unlistenProgress: (() => void) | null = null;

    /**
     * 处理"立即更新"：流式下载并安装，期间显示进度
     */
    const handleUpdate = async () => {
        const info = appUpdateInfo();
        if (!info) return;

        setError(null);
        setAppUpdateDownloading(true);
        setAppUpdateProgress(0);
        setAppUpdateReady(false);

        try {
            await invoke('install_app_update');
            setAppUpdateProgress(1);
            setAppUpdateReady(true);
        } catch (e) {
            console.error('下载更新失败:', e);
            setError(typeof e === 'string' ? e : '更新下载失败，请稍后重试');
        } finally {
            setAppUpdateDownloading(false);
        }
    };

    /**
     * 处理"稍后"：把版本号写入 localStorage 并关闭浮层
     */
    const handleDismiss = () => {
        const info = appUpdateInfo();
        if (info?.version) {
            setIgnoredUpdateVersion(info.version);
        }
        setAppUpdateDismissed(true);
    };

    /**
     * 处理"重启应用"：调用后端 `restart_app` 触发更新安装
     */
    const handleRestart = async () => {
        try {
            await invoke('restart_app');
        } catch (e) {
            console.error('重启失败:', e);
            setError(typeof e === 'string' ? e : '重启失败，请手动关闭并重新打开应用');
        }
    };

    onMount(async () => {
        // 监听后端推送的下载进度事件 (0.0 ~ 1.0)
        try {
            unlistenProgress = await listen<number>('app-update-progress', (event) => {
                const pct = typeof event.payload === 'number' ? event.payload : 0;
                setAppUpdateProgress(Math.max(0, Math.min(1, pct)));
            });
        } catch (e) {
            console.warn('监听 app-update-progress 失败:', e);
        }
    });

    onCleanup(() => {
        if (unlistenProgress) unlistenProgress();
    });

    return (
        <Transition name="update-toast">
            <Show when={appUpdateAvailable() && !appUpdateDismissed() && appUpdateInfo()}>
                <div
                    class="fixed bottom-5 left-5 z-[9999] w-[340px] rounded-2xl overflow-hidden select-none"
                    style={{
                        background: 'rgba(18, 22, 35, 0.88)',
                        'backdrop-filter': 'blur(40px)',
                        '-webkit-backdrop-filter': 'blur(40px)',
                        border: '1px solid rgba(255, 255, 255, 0.08)',
                        'box-shadow': '0 12px 40px rgba(0, 0, 0, 0.45)',
                    }}
                >
                    {/* 顶部装饰条 */}
                    <div
                        class="h-[3px] w-full"
                        style={{
                            background: 'linear-gradient(90deg, var(--primary-color), color-mix(in srgb, var(--primary-color), transparent 30%))',
                        }}
                    />

                    <div class="p-4 flex flex-col gap-3">
                        {/* 标题行 */}
                        <div class="flex items-center gap-2.5">
                            <div
                                class="w-9 h-9 rounded-xl flex items-center justify-center shrink-0"
                                style={{
                                    background: 'rgba(var(--primary-rgb), 0.18)',
                                    color: 'var(--primary-color)',
                                }}
                            >
                                <Icon src="/icons/app-logo/top.svg" class="w-5 h-5" />
                            </div>
                            <div class="flex-1 min-w-0">
                                <div class="text-sm font-semibold text-white leading-tight">
                                    发现新版本 v{appUpdateInfo()!.version}
                                </div>
                                <div class="text-[11px] text-[#888] mt-0.5">
                                    当前版本 v{appUpdateInfo()!.currentVersion || '?'} · 点击立即更新
                                </div>
                            </div>
                        </div>

                        {/* Release notes 摘要（仅当有内容时显示） */}
                        <Show when={appUpdateInfo()!.notes}>
                            <div
                                class="text-[12px] text-[#bbb] leading-relaxed rounded-lg p-2.5 max-h-[88px] overflow-y-auto"
                                style={{
                                    background: 'rgba(255, 255, 255, 0.04)',
                                    border: '1px solid rgba(255, 255, 255, 0.05)',
                                    'white-space': 'pre-wrap',
                                    'word-break': 'break-word',
                                }}
                            >
                                {appUpdateInfo()!.notes!.length > 220
                                    ? appUpdateInfo()!.notes!.slice(0, 220) + '…'
                                    : appUpdateInfo()!.notes}
                            </div>
                        </Show>

                        {/* 错误信息 */}
                        <Show when={error()}>
                            <div
                                class="text-[12px] rounded-lg px-3 py-2"
                                style={{
                                    background: 'rgba(255, 80, 80, 0.1)',
                                    color: '#ff8080',
                                    border: '1px solid rgba(255, 80, 80, 0.2)',
                                }}
                            >
                                {error()}
                            </div>
                        </Show>

                        {/* 进度条：仅在下载中或等待重启时显示 */}
                        <Show when={appUpdateDownloading() || appUpdateReady()}>
                            <div class="flex flex-col gap-1.5">
                                <div class="flex justify-between text-[11px] text-[#888]">
                                    <span>
                                        {appUpdateReady() ? '下载完成，等待重启' : '正在下载更新…'}
                                    </span>
                                    <span class="font-mono">
                                        {Math.round(appUpdateProgress() * 100)}%
                                    </span>
                                </div>
                                <div
                                    class="h-1.5 rounded-full overflow-hidden"
                                    style={{ background: 'rgba(255, 255, 255, 0.08)' }}
                                >
                                    <div
                                        class="h-full rounded-full transition-all duration-200"
                                        style={{
                                            width: `${Math.round(appUpdateProgress() * 100)}%`,
                                            background:
                                                'linear-gradient(90deg, var(--primary-color), color-mix(in srgb, var(--primary-color), white 30%))',
                                        }}
                                    />
                                </div>
                            </div>
                        </Show>

                        {/* 操作按钮 */}
                        <div class="flex gap-2 mt-1">
                            <Show
                                when={!appUpdateReady()}
                                fallback={
                                    <button
                                        class="flex-1 py-2 rounded-lg text-sm font-semibold cursor-pointer border-none text-black transition-all duration-200 hover:opacity-90 active:scale-95"
                                        style={{
                                            background:
                                                'linear-gradient(135deg, var(--primary-color), color-mix(in srgb, var(--primary-color), white 20%))',
                                        }}
                                        onClick={handleRestart}
                                    >
                                        重启应用
                                    </button>
                                }
                            >
                                <button
                                    class="flex-1 py-2 rounded-lg text-sm font-semibold cursor-pointer border-none text-black transition-all duration-200 hover:opacity-90 active:scale-95 disabled:opacity-50 disabled:cursor-not-allowed"
                                    style={{
                                        background:
                                            'linear-gradient(135deg, var(--primary-color), color-mix(in srgb, var(--primary-color), white 20%))',
                                    }}
                                    disabled={appUpdateDownloading()}
                                    onClick={handleUpdate}
                                >
                                    {appUpdateDownloading() ? '下载中…' : '立即更新'}
                                </button>
                                <button
                                    class="px-4 py-2 rounded-lg text-sm cursor-pointer transition-all duration-200 hover:bg-white/10 active:scale-95"
                                    style={{
                                        background: 'rgba(255, 255, 255, 0.05)',
                                        color: 'rgba(255, 255, 255, 0.7)',
                                        border: '1px solid rgba(255, 255, 255, 0.06)',
                                    }}
                                    onClick={handleDismiss}
                                >
                                    稍后
                                </button>
                            </Show>
                        </div>
                    </div>
                </div>
            </Show>
        </Transition>
    );
};

export default UpdateNotification;
