/**
 * Host status determination utility
 *
 * 3-tier determination based on SSE architecture:
 *
 * 1. is_online === false (explicit server down determination) -> offline
 * 2. last_seen timestamp-based safety net (fallback for SSE connection loss, etc.)
 *    - online:  last received <= 1 scrape interval (<= 10s)
 *    - pending: last received 10s ~ 30s (1~3 intervals missed)
 *    - offline: last received > 30s (3+ intervals missed)
 */

export type HostStatus = "online" | "pending" | "offline";

/** Must match scrape_interval_secs in config */
const SCRAPE_INTERVAL_SEC = 10;
const PENDING_THRESHOLD_SEC = SCRAPE_INTERVAL_SEC;        // > 10s -> pending
const OFFLINE_THRESHOLD_SEC = SCRAPE_INTERVAL_SEC * 3;   // > 30s -> offline

/**
 * Determine host status based on SSE payload.
 *
 * @param lastSeen  - Last received timestamp (ISO string or null)
 * @param isOnline  - Online status determined by the server (scraper) (is_online from SSE payload).
 *                    If undefined, determination is based on timestamp only (backward compatible).
 */
export function getHostStatus(
  lastSeen: string | null,
  isOnline?: boolean,
): HostStatus {
  // Server explicitly determined as down — takes priority over timestamp calculation
  // However, if last_seen is absent, it has never been scraped, so return "pending" (status unknown)
  if (isOnline === false) {
    return lastSeen ? "offline" : "pending";
  }

  // No timestamp means unobserved state -> pending (was previously offline, but semantically inaccurate)
  if (!lastSeen) return "pending";

  const diffSec = (Date.now() - new Date(lastSeen).getTime()) / 1000;

  if (diffSec <= PENDING_THRESHOLD_SEC) return "online";
  if (diffSec <= OFFLINE_THRESHOLD_SEC) return "pending";
  return "offline";
}

/** Colors per status */
export const STATUS_COLORS: Record<HostStatus, { accent: string; bg: string; border: string }> = {
  online:  { accent: "var(--accent-green)",  bg: "var(--status-online-bg)", border: "var(--badge-online-border)" },
  pending: { accent: "var(--accent-yellow)", bg: "var(--badge-pending-bg)", border: "var(--badge-pending-border)" },
  offline: { accent: "var(--accent-red)",    bg: "var(--status-offline-bg)", border: "var(--badge-offline-border)" },
};

/** Labels per status */
export const STATUS_LABELS: Record<HostStatus, string> = {
  online:  "Online",
  pending: "Pending",
  offline: "Offline",
};

/** Badge CSS class per status */
export const STATUS_BADGE_CLASS: Record<HostStatus, string> = {
  online:  "badge-online",
  pending: "badge-pending",
  offline: "badge-offline",
};

/** Pulse dot CSS class per status */
export const STATUS_DOT_CLASS: Record<HostStatus, string> = {
  online:  "pulse-dot green",
  pending: "pulse-dot yellow",
  offline: "pulse-dot red",
};

/** Sub-labels per status (for Sidebar, server list) */
export const STATUS_SUB_LABELS: Record<HostStatus, string> = {
  online:  "Active Node",
  pending: "Reconnecting...",
  offline: "Offline Node",
};
