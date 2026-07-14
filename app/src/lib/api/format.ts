// Human-sized numbers for the UI. Pure string work, no Tauri import.

const KB = 1024;
const MB = 1024 * 1024;

/**
 * A file size a person can read at a glance: bytes, then kilobytes, then
 * megabytes. The unit adapts because a 2 KB Markdown book that renders as
 * "0.0 MB" reads like a bug, and because "1024 KB" is a number nobody wants -
 * so the switch to MB happens at 1000 KB, one step early on purpose. Kilobytes
 * lose their decimal once the number carries itself (20 KB, not 20.4 KB);
 * megabytes keep theirs, because that is where a book's size actually lives.
 */
export function formatSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  if (bytes < KB) return `${Math.round(bytes)} B`;

  const kb = bytes / KB;
  if (kb < 1000) return `${kb < 10 ? kb.toFixed(1) : Math.round(kb)} KB`;

  return `${(bytes / MB).toFixed(1)} MB`;
}
