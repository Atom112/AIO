/**
 * @file Layout.tsx
 * @description 应用的通用布局组件, 整窗为统一亚克力面板(NavBar + 页面共享同块玻璃),
 * 背景为静态多色渐变(不受主题色影响), 主题色仅作用于按钮/开关等交互元素.
 */
import NavBar from "./components/NavBar";
import UpdateNotification from "./components/UpdateNotification";
import { Transition } from "solid-transition-group";
import { Component, onCleanup, onMount, ParentProps } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import {
    appUpdateAvailable,
    setAppUpdateAvailable,
    setAppUpdateInfo,
    setAppUpdateDismissed,
    setAppUpdateDownloading,
    setAppUpdateProgress,
    setAppUpdateReady,
    getIgnoredUpdateVersion,
} from "./store/store";

/**
 * Layout 组件
 * @param props - 包含 children (当前匹配路由指向的组件内容)
 * @returns 返回一个包含通用导航和过渡动画的内容区域
 */
const Layout: Component<ParentProps> = (props) => {
    /**
     * 应用启动时自动检查更新
     * 流程：延迟 1.5s（避开首屏渲染高峰）→ 调用 check_app_update
     *      → 若发现新版本且未被用户忽略则点亮左下角 Toast
     */
    onMount(async () => {
        const currentVersion = await getVersion().catch(() => '');

        // 启动时重置一次性状态（防止上次会话残留）
        setAppUpdateAvailable(false);
        setAppUpdateDismissed(false);
        setAppUpdateDownloading(false);
        setAppUpdateProgress(0);
        setAppUpdateReady(false);

        setTimeout(async () => {
            try {
                type CheckUpdateResult =
                    | { kind: 'up_to_date'; current_version: string }
                    | { kind: 'update_available'; info: { version: string; current_version: string; notes?: string; pub_date?: string } }
                    | { kind: 'service_not_ready'; current_version: string; endpoint: string; reason: string }
                    | { kind: 'network'; current_version: string; endpoint: string; reason: string }
                    | { kind: 'failed'; current_version: string; endpoint: string; reason: string };

                const result = await invoke<CheckUpdateResult>('check_app_update');

                if (result.kind === 'update_available') {
                    const info = result.info;
                    // 用户已经点过"稍后"过该版本就不再提示
                    if (getIgnoredUpdateVersion() === info.version) return;

                    setAppUpdateInfo({
                        version: info.version,
                        currentVersion: info.current_version || currentVersion,
                        notes: info.notes,
                        pubDate: info.pub_date,
                    });
                    setAppUpdateAvailable(true);
                }
                // 其他情况：up_to_date / service_not_ready / network / failed → 静默不打扰
            } catch (e) {
                // 网络失败静默处理，不打扰用户
                console.warn('启动时检查更新失败:', e);
            }
        }, 1500);
    });

    onCleanup(() => {
        // 清理逻辑预留
    });

    return (
        <div
            class="app-container h-screen flex flex-col overflow-hidden rounded-xl"
            style={{
                background: [
                    "radial-gradient(ellipse 100% 70% at 25% 15%, rgba(70, 120, 200, 0.45), transparent 60%)",
                    "radial-gradient(ellipse 80% 90% at 70% 85%, rgba(50, 90, 140, 0.35), transparent 55%)",
                    "linear-gradient(135deg, #1e3a5f 0%, #2d2d5e 30%, #3d2d5a 50%, #2a2050 70%, #1a2540 100%)",
                ].join(", "),
            }}
        >
            <NavBar />
            <main class="flex-1 relative overflow-hidden">
                <Transition name="page-fade">
                    {props.children}
                </Transition>
            </main>
            <UpdateNotification />
        </div>
    );
};

/**
 * 简单 SemVer 版本号比较
 * @returns 1 if a>b, -1 if a<b, 0 if equal
 */
function compareVersions(a: string, b: string): number {
    const pa = a.replace(/^v/i, '').split('.').map((n) => parseInt(n, 10) || 0);
    const pb = b.replace(/^v/i, '').split('.').map((n) => parseInt(n, 10) || 0);
    const len = Math.max(pa.length, pb.length);
    for (let i = 0; i < len; i++) {
        const x = pa[i] ?? 0;
        const y = pb[i] ?? 0;
        if (x > y) return 1;
        if (x < y) return -1;
    }
    return 0;
}

export default Layout;
