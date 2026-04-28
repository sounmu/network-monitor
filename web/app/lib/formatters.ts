/**
 * Convert raw network speed (Byte/s) to a human-readable unit
 *
 * @param bytesPerSec  Raw value in Byte/s (number | null | undefined)
 * @returns            String in the form "0 KB/s" / "12.3 KB/s" / "4.5 MB/s" / "1.2 GB/s"
 */
export function formatNetworkSpeed(bytesPerSec: number | null | undefined): string {
  if (bytesPerSec == null || !isFinite(bytesPerSec) || isNaN(bytesPerSec) || bytesPerSec < 0) {
    return "0 KB/s";
  }

  const kb = bytesPerSec / 1024;

  if (kb < 1024) {
    // KB/s range: 1 decimal place (values below 0.1 show as 1 digit)
    return `${kb < 0.05 ? "0" : kb.toFixed(1)} KB/s`;
  }

  const mb = kb / 1024;
  if (mb < 1024) {
    return `${mb.toFixed(2)} MB/s`;
  }

  const gb = mb / 1024;
  return `${gb.toFixed(2)} GB/s`;
}

/**
 * Convert raw bytes to a human-readable unit (no "/s" suffix).
 * Used for cumulative byte counters (e.g., container network totals).
 */
export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || !isFinite(bytes) || isNaN(bytes) || bytes < 0) {
    return "0 B";
  }
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb.toFixed(1)} MB`;
  const gb = mb / 1024;
  return `${gb.toFixed(2)} GB`;
}

/**
 * Axis tick only — abbreviated to keep labels short
 */
export function formatNetworkSpeedTick(bytesPerSec: number | null | undefined): string {
  if (bytesPerSec == null || !isFinite(bytesPerSec) || isNaN(bytesPerSec) || bytesPerSec < 0) {
    return "0";
  }

  const kb = bytesPerSec / 1024;

  if (kb < 1024) {
    return `${kb < 0.05 ? "0" : kb.toFixed(1)} KB/s`;
  }

  const mb = kb / 1024;
  if (mb < 1024) {
    return `${mb.toFixed(1)} MB/s`;
  }

  const gb = mb / 1024;
  return `${gb.toFixed(1)} GB/s`;
}
