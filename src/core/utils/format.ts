// 通用格式化辅助 —— 全仓唯一来源（历史上有 3 套互不兼容的 formatSize 实现）。

/** 人类可读文件大小：KB（<1MB，整数，最小 1KB）/ MB（<1GB，1 位小数）/ GB（2 位小数）。
 *  非正数返回空串（调用方用 Show 判空隐藏）。 */
export function formatSize(bytes: number): string {
  if (bytes <= 0) return '';
  if (bytes < 1024 * 1024) return Math.max(1, Math.round(bytes / 1024)) + ' KB';
  if (bytes < 1024 * 1024 * 1024) return (bytes / 1024 / 1024).toFixed(1) + ' MB';
  return (bytes / 1024 / 1024 / 1024).toFixed(2) + ' GB';
}
