/**
 * Safe external URL opening:
 * Allows only http/https; blocks file: and custom schemes that could launch local
 * programs via plugin-opener. Remote catalog/market JSON URLs are untrusted and must be validated before opening.
 */
import { openUrl } from '@tauri-apps/plugin-opener';

/** Opens only http/https links; invalid or non-whitelisted schemes are silently ignored. */
export async function openSafeUrl(url?: string | null): Promise<void> {
  if (!url) return;
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    console.warn('[openSafeUrl] unparseable URL blocked:', url);
    return;
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    console.warn('[openSafeUrl] non-http/https link blocked:', parsed.protocol);
    return;
  }
  await openUrl(url);
}
