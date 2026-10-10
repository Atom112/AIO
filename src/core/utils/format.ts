// Shared formatting helpers — single source of truth repo-wide.

/** Human-readable file size: KB (<1MB, integer, min 1KB) / MB (<1GB, 1 decimal) / GB (2 decimals).
 *  Non-positive input returns an empty string (callers use Show to check and hide). */
export function formatSize(bytes: number): string {
  if (bytes <= 0) return '';
  if (bytes < 1024 * 1024) return Math.max(1, Math.round(bytes / 1024)) + ' KB';
  if (bytes < 1024 * 1024 * 1024) return (bytes / 1024 / 1024).toFixed(1) + ' MB';
  return (bytes / 1024 / 1024 / 1024).toFixed(2) + ' GB';
}
