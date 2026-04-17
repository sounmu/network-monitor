import { MetricsRow } from "@/app/types/metrics";

// Default to localhost (not 127.0.0.1) so the browser treats the frontend
// (localhost:3001) and API (localhost:3000) as same-site. This is required
// for SameSite=Strict cookies to be sent on fetch requests.
const API_BASE = process.env.NEXT_PUBLIC_API_URL || "http://localhost:3000";

// ── Access token (memory-only) ──────────────────────────────────────────
// The short-lived access JWT lives only in a module-level variable and the
// React context. It is never written to localStorage. On page reload, the
// init flow calls /api/auth/refresh (which reads the httpOnly cookie) and
// receives a fresh token.

let accessToken: string | null = null;

export function setAccessToken(token: string | null) {
  accessToken = token;
}

export function getAccessToken(): string | null {
  return accessToken;
}

/** @deprecated — only exists to remove stale keys from older versions. */
export function clearLegacyStorage() {
  if (typeof window !== "undefined") {
    localStorage.removeItem("auth_token");
  }
}

// ── Backward-compat aliases used by AuthContext during the migration ──
// These will be removed once all callsites switch to the new names.
export const setUserToken = setAccessToken;
export const getUserToken = getAccessToken;

// ── Singleflight refresh ────────────────────────────────────────────────
// A single POST /api/auth/refresh is shared across every caller:
//   * AuthContext.init on page reload (tryRefreshSession)
//   * 401 retry in fetcher/apiCall (silentRefresh)
//   * React 18 StrictMode double-effect (same mount cycle fires twice)
// Without this, two concurrent requests present the same cookie → the
// server rotates on the first one, then treats the second as reuse
// detection → revokes the entire family → user logged out.
let inflightRefresh: Promise<LoginResponse | null> | null = null;

async function doRefreshOnce(): Promise<LoginResponse | null> {
  // Bound the wait. A hung refresh call would otherwise freeze the entire
  // AuthProvider (isLoading=true → null render) until the browser's default
  // fetch timeout fires, minutes later. "No session" is the safe fallback.
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), 4000);
  try {
    const res = await fetch(`${API_BASE}/api/auth/refresh`, {
      method: "POST",
      credentials: "include",
      headers: { Accept: "application/json" },
      signal: controller.signal,
    });
    if (!res.ok) return null;
    const body: LoginResponse = await res.json();
    setAccessToken(body.token);
    return body;
  } catch {
    return null;
  } finally {
    clearTimeout(timeoutId);
  }
}

/** Coalesce concurrent refresh calls into one network round-trip. */
function singleflightRefresh(): Promise<LoginResponse | null> {
  if (inflightRefresh) return inflightRefresh;
  inflightRefresh = doRefreshOnce().finally(() => {
    inflightRefresh = null;
  });
  return inflightRefresh;
}

/** Boolean wrapper for the 401-retry path in fetcher/apiCall. */
async function silentRefresh(): Promise<boolean> {
  return (await singleflightRefresh()) !== null;
}

// ── Unauthorized handler ────────────────────────────────────────────────

function handleUnauthorized(): never {
  setAccessToken(null);
  if (typeof window !== "undefined") {
    // Guard against redirect loop: if we are already on /login, do not
    // trigger another hard reload. AuthContext's render guard prevents
    // protected children from mounting, so this path is a last resort
    // for fetch calls that slip through.
    if (!window.location.pathname.startsWith("/login")) {
      window.location.href = "/login";
    }
  }
  throw new Error("Session expired");
}

// ── Core fetch helpers ──────────────────────────────────────────────────
// All helpers include `credentials: "include"` so the refresh cookie is
// always sent to /api/auth/* paths (the cookie's `Path=/api/auth`
// attribute scopes it).

function authHeaders(): HeadersInit {
  const token = getAccessToken();
  return {
    "Content-Type": "application/json",
    Accept: "application/json",
    ...(token && { Authorization: `Bearer ${token}` }),
  };
}

export const fetcher = async <T>(url: string): Promise<T> => {
  const doFetch = async () => {
    const token = getAccessToken();
    return fetch(url, {
      headers: {
        Accept: "application/json",
        ...(token && { Authorization: `Bearer ${token}` }),
      },
      credentials: "include",
      mode: "cors",
    });
  };

  let res = await doFetch();
  // One silent refresh attempt on 401 before giving up.
  // 403 is a permission error, NOT a session problem — don't refresh/logout.
  if (res.status === 401) {
    const refreshed = await silentRefresh();
    if (refreshed) {
      res = await doFetch();
    }
    if (res.status === 401) handleUnauthorized();
  }
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new ApiError(res.status, text || `API Error: ${res.status} ${res.statusText}`);
  }
  return res.json();
};

async function apiCall<T>(url: string, method: string, body?: unknown): Promise<T> {
  const doFetch = async () => {
    const opts: RequestInit = {
      method,
      headers: authHeaders(),
      credentials: "include",
    };
    if (body !== undefined) {
      opts.body = JSON.stringify(body);
    }
    return fetch(url, opts);
  };

  let res = await doFetch();
  // Mirror `fetcher`: 401 triggers a silent refresh; 403 bubbles as a typed
  // permission error so callers can distinguish "your session is dead" from
  // "you don't have permission for this action" without logging the user out.
  if (res.status === 401) {
    const refreshed = await silentRefresh();
    if (refreshed) {
      res = await doFetch();
    }
    if (res.status === 401) handleUnauthorized();
  }
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new ApiError(res.status, text || `${res.status} ${res.statusText}`);
  }
  return res.json();
}

/** GET /api/metrics/:host_key — last 50 metric rows */
export const getMetricsUrl = (hostKey: string) =>
  `${API_BASE}/api/metrics/${encodeURIComponent(hostKey)}`;

/** GET /api/metrics/:host_key?start=...&end=... — time-range based metric query.
 * Timestamps are rounded to the nearest minute so that requests made seconds
 * apart produce the same URL, enabling both SWR deduplication and server-side
 * cache hits (server rounds to 5-minute boundaries). */
export const getMetricsRangeUrl = (hostKey: string, start: Date, end: Date) => {
  // Floor start to minute, ceil end to minute
  const startRounded = new Date(Math.floor(start.getTime() / 60000) * 60000);
  const endRounded = new Date(Math.ceil(end.getTime() / 60000) * 60000);
  const params = new URLSearchParams({
    start: startRounded.toISOString(),
    end: endRounded.toISOString(),
  });
  return `${API_BASE}/api/metrics/${encodeURIComponent(hostKey)}?${params.toString()}`;
};

/** GET /api/hosts — all hosts list (includes is_online) */
export const getHostsUrl = () => `${API_BASE}/api/hosts`;

// ── Host CRUD (DB-driven) ──

export interface HostConfig {
  host_key: string;
  display_name: string;
  scrape_interval_secs: number;
  load_threshold: number;
  ports: number[];
  containers: string[];
  created_at?: string;
  updated_at?: string;
}

export interface UpdateHostRequest {
  display_name?: string;
  scrape_interval_secs?: number;
  load_threshold?: number;
  ports?: number[];
  containers?: string[];
}

export const getHostConfig = (hostKey: string) =>
  fetcher<HostConfig>(`${API_BASE}/api/hosts/${encodeURIComponent(hostKey)}`);

export const createHost = (host: Omit<HostConfig, "created_at" | "updated_at">) =>
  apiCall<HostConfig>(`${API_BASE}/api/hosts`, "POST", host);

export const updateHost = (hostKey: string, body: UpdateHostRequest) =>
  apiCall<HostConfig>(`${API_BASE}/api/hosts/${encodeURIComponent(hostKey)}`, "PUT", body);

export const deleteHost = async (hostKey: string): Promise<void> => {
  await apiCall<unknown>(`${API_BASE}/api/hosts/${encodeURIComponent(hostKey)}`, "DELETE");
};

// ── Alert Config CRUD ──

export interface AlertConfigRow {
  id: number;
  host_key: string | null;
  metric_type: "cpu" | "memory" | "disk";
  enabled: boolean;
  threshold: number;
  sustained_secs: number;
  cooldown_secs: number;
  updated_at: string;
}

export interface UpsertAlertRequest {
  metric_type: "cpu" | "memory" | "disk";
  enabled: boolean;
  threshold: number;
  sustained_secs: number;
  cooldown_secs: number;
}

export const getAlertConfigsUrl = () => `${API_BASE}/api/alert-configs`;
export const getHostAlertConfigsUrl = (hostKey: string) =>
  `${API_BASE}/api/alert-configs/${encodeURIComponent(hostKey)}`;

export const updateGlobalAlertConfigs = (body: UpsertAlertRequest[]) =>
  apiCall<AlertConfigRow[]>(`${API_BASE}/api/alert-configs`, "PUT", body);

export const updateHostAlertConfigs = (hostKey: string, body: UpsertAlertRequest[]) =>
  apiCall<AlertConfigRow[]>(`${API_BASE}/api/alert-configs/${encodeURIComponent(hostKey)}`, "PUT", body);

export const deleteHostAlertConfigs = (hostKey: string) =>
  apiCall<unknown>(`${API_BASE}/api/alert-configs/${encodeURIComponent(hostKey)}`, "DELETE");

// ── Notification Channels CRUD ──

export interface NotificationChannel {
  id: number;
  name: string;
  channel_type: "discord" | "slack" | "email";
  enabled: boolean;
  config: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

export const getNotificationChannelsUrl = () => `${API_BASE}/api/notification-channels`;

export const createNotificationChannel = (body: { name: string; channel_type: string; enabled?: boolean; config: Record<string, unknown> }) =>
  apiCall<NotificationChannel>(`${API_BASE}/api/notification-channels`, "POST", body);

export const updateNotificationChannel = (id: number, body: { name?: string; enabled?: boolean; config?: Record<string, unknown> }) =>
  apiCall<NotificationChannel>(`${API_BASE}/api/notification-channels/${id}`, "PUT", body);

export const deleteNotificationChannel = (id: number) =>
  apiCall<unknown>(`${API_BASE}/api/notification-channels/${id}`, "DELETE");

export const testNotificationChannel = (id: number) =>
  apiCall<{ success: boolean }>(`${API_BASE}/api/notification-channels/${id}/test`, "POST");

// ── Alert History ──

export interface AlertHistoryRow {
  id: number;
  host_key: string;
  alert_type: string;
  message: string;
  created_at: string;
}

export const getAlertHistoryUrl = (hostKey?: string, limit = 50) => {
  const params = new URLSearchParams();
  if (hostKey) params.set("host_key", hostKey);
  params.set("limit", String(limit));
  return `${API_BASE}/api/alert-history?${params.toString()}`;
};

// ── Uptime ──

export interface UptimePoint {
  day: string;
  total_count: number;
  online_count: number;
  uptime_pct: number;
}

export interface UptimeSummary {
  host_key: string;
  overall_pct: number;
  daily: UptimePoint[];
}

export const getUptimeUrl = (hostKey: string, days = 30) =>
  `${API_BASE}/api/uptime/${encodeURIComponent(hostKey)}?days=${days}`;

// ── Public Status ──

export interface PublicHostStatus {
  host_key: string;
  display_name: string;
  is_online: boolean;
  uptime_7d: number;
}

export const getPublicStatusUrl = () => `${API_BASE}/api/public/status`;

// ── HTTP Monitors ──

export interface HttpMonitor {
  id: number;
  name: string;
  url: string;
  method: string;
  expected_status: number;
  interval_secs: number;
  timeout_ms: number;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

export interface HttpMonitorSummary {
  monitor_id: number;
  latest_status_code: number | null;
  latest_response_time_ms: number | null;
  latest_error: string | null;
  total_checks: number;
  successful_checks: number;
  uptime_pct: number;
}

export const getHttpMonitorsUrl = () => `${API_BASE}/api/http-monitors`;
export const getHttpSummariesUrl = () => `${API_BASE}/api/http-monitors/summaries`;
export const createHttpMonitor = (body: { name: string; url: string; method?: string; expected_status?: number; interval_secs?: number; timeout_ms?: number }) =>
  apiCall<HttpMonitor>(`${API_BASE}/api/http-monitors`, "POST", body);
export const updateHttpMonitor = (id: number, body: Record<string, unknown>) =>
  apiCall<HttpMonitor>(`${API_BASE}/api/http-monitors/${id}`, "PUT", body);
export const deleteHttpMonitor = (id: number) =>
  apiCall<unknown>(`${API_BASE}/api/http-monitors/${id}`, "DELETE");

// ── Ping Monitors ──

export interface PingMonitor {
  id: number;
  name: string;
  host: string;
  interval_secs: number;
  timeout_ms: number;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

export interface PingMonitorSummary {
  monitor_id: number;
  latest_rtt_ms: number | null;
  latest_success: boolean | null;
  latest_error: string | null;
  total_checks: number;
  successful_checks: number;
  uptime_pct: number;
}

export const getPingMonitorsUrl = () => `${API_BASE}/api/ping-monitors`;
export const getPingSummariesUrl = () => `${API_BASE}/api/ping-monitors/summaries`;
export const createPingMonitor = (body: { name: string; host: string; interval_secs?: number; timeout_ms?: number }) =>
  apiCall<PingMonitor>(`${API_BASE}/api/ping-monitors`, "POST", body);
export const updatePingMonitor = (id: number, body: Record<string, unknown>) =>
  apiCall<PingMonitor>(`${API_BASE}/api/ping-monitors/${id}`, "PUT", body);
export const deletePingMonitor = (id: number) =>
  apiCall<unknown>(`${API_BASE}/api/ping-monitors/${id}`, "DELETE");

// ── Dashboard Layout ──

export interface DashboardWidget {
  id: string;
  type: "host_status" | "cpu_chart" | "memory_chart" | "alert_feed" | "uptime_overview" | "http_monitor";
  host_key?: string;
  monitor_id?: number;
  title?: string;
}

export const getDashboardUrl = () => `${API_BASE}/api/dashboard`;

export const saveDashboard = (widgets: DashboardWidget[]) =>
  apiCall<unknown>(`${API_BASE}/api/dashboard`, "PUT", { widgets });

// ── Auth ──

export interface UserInfo {
  id: number;
  username: string;
  role: string;
}

export interface LoginResponse {
  token: string;
  user: UserInfo;
}

export interface AuthStatus {
  setup_required: boolean;
}

export const getAuthStatusUrl = () => `${API_BASE}/api/auth/status`;

/** Custom error class that preserves HTTP status code for login error handling */
export class ApiError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

export const login = async (username: string, password: string): Promise<LoginResponse> => {
  const res = await fetch(`${API_BASE}/api/auth/login`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    credentials: "include",
    body: JSON.stringify({ username, password }),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new ApiError(res.status, text || `${res.status} ${res.statusText}`);
  }
  return res.json();
};

export const setupAdmin = async (username: string, password: string): Promise<LoginResponse> => {
  const res = await fetch(`${API_BASE}/api/auth/setup`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    credentials: "include",
    body: JSON.stringify({ username, password }),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new ApiError(res.status, text || `${res.status} ${res.statusText}`);
  }
  return res.json();
};

export const getMe = () =>
  apiCall<UserInfo>(`${API_BASE}/api/auth/me`, "GET");

/** Response from POST /api/auth/sse-ticket — short-lived opaque ticket. */
export interface SseTicketResponse {
  ticket: string;
  expires_in_secs: number;
}

/**
 * Request a single-use SSE ticket. The returned ticket is consumed atomically
 * on the next /api/stream handshake and expires after `expires_in_secs`
 * whether consumed or not.
 *
 * Must be called immediately before each EventSource connection / reconnection
 * — tickets are single-use, so re-issuing on every reconnect is required.
 */
export const issueSseTicket = () =>
  apiCall<SseTicketResponse>(`${API_BASE}/api/auth/sse-ticket`, "POST");

/**
 * Tell the server to revoke every JWT it has ever issued to the caller.
 * Best-effort: clients must still clear local state regardless of the
 * outcome (network down, server restarting, etc.). Never throws — the
 * UI should fall through to the local logout cleanup unconditionally.
 */
export const serverLogout = async (): Promise<void> => {
  const token = getAccessToken();
  try {
    await fetch(`${API_BASE}/api/auth/logout`, {
      method: "POST",
      credentials: "include",
      headers: {
        Accept: "application/json",
        ...(token && { Authorization: `Bearer ${token}` }),
      },
    });
  } catch {
    // Swallow — the local logout path will still clear the token and redirect.
  }
};

/**
 * Try to restore the session by calling /api/auth/refresh. Used on
 * page load when no in-memory access token exists (memory-only tokens
 * are lost on reload). Delegates to the shared singleflight so that
 * React 18 StrictMode double-effects and concurrent 401 retries all
 * coalesce into one network call.
 */
export const tryRefreshSession = singleflightRefresh;

// Batch metrics query — fetch metrics for multiple hosts in a single request
export const fetchBatchMetrics = (hostKeys: string[], start: string, end: string) =>
  apiCall<Record<string, MetricsRow[]>>(`${API_BASE}/api/metrics/batch`, "POST", {
    host_keys: hostKeys,
    start,
    end,
  });

export type { MetricsRow };
