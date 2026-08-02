/**
 * 安全外链打开（D4）：
 * 仅放行 http/https，阻止 file: / 自定义 scheme 经 plugin-opener 拉起本地程序。
 * 远程 catalog/market JSON 提供的 URL 非可信，打开前必须校验。
 */
import { openUrl } from '@tauri-apps/plugin-opener';

/** 仅打开 http/https 链接；非法或非白名单 scheme 一律静默忽略。 */
export async function openSafeUrl(url?: string | null): Promise<void> {
  if (!url) return;
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    console.warn('[openSafeUrl] 无法解析的 URL，已阻止:', url);
    return;
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    console.warn('[openSafeUrl] 已阻止非 http/https 链接:', parsed.protocol);
    return;
  }
  await openUrl(url);
}
