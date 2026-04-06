import { MetricsRow, HostsApiResponse } from "@/app/types/metrics";

const API_BASE = process.env.NEXT_PUBLIC_API_URL || "http://127.0.0.1:3000";

// User auth token management
let userToken: string | null = null;

export function setUserToken(token: string | null) {
  userToken = token;
  if (typeof window !== "undefined") {
    if (token) localStorage.setItem("auth_token", token);
    else localStorage.removeItem("auth_token");
  }
}

export function getUserToken(): string | null {
  if (userToken) return userToken;
  if (typeof window !== "undefined") {
    userToken = localStorage.getItem("auth_token");
  }
  return userToken;
}

function getAuthToken(): string | undefined {
  return getUserToken() || undefined;
}

/** Handle 401 — clear token and redirect to login */
function handleUnauthorized(): never {
  setUserToken(null);
  if (typeof window !== "undefined") {
    window.location.href = "/login";
  }
  throw new Error("Session expired");
}

export const fetcher = async <T>(url: string): Promise<T> => {
  const token = getAuthToken();
  const res = await fetch(url, {
    headers: {
      Accept: "application/json",
      ...(token && { Authorization: `Bearer ${token}` }),
    },
    mode: "cors",
  });
  if (res.status === 401) handleUnauthorized();
  if (!res.ok) {
    throw new Error(`API Error: ${res.status} ${res.statusText}`);
  }
  return res.json();
};

const authHeaders = (): HeadersInit => {
  const token = getAuthToken();
  return {
    "Content-Type": "application/json",
    Accept: "application/json",
    ...(token && { Authorization: `Bearer ${token}` }),
  };
};

async function apiCall<T>(url: string, method: string, body?: unknown): Promise<T> {
  const opts: RequestInit = { method, headers: authHeaders() };
  if (body !== undefined) {
    opts.body = JSON.stringify(body);
  }
  const res = await fetch(url, opts);
  if (res.status === 401) handleUnauthorized();
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `${res.status} ${res.statusText}`);
  }
  return res.json();
}

/** GET /api/metrics/:host_key — last 50 metric rows */
export const getMetricsUrl = (hostKey: string) =>
  `${API_BASE}/api/metrics/${encodeURIComponent(hostKey)}`;

/** GET /api/metrics/:host_key?start=...&end=... — time-range based metric query */
export const getMetricsRangeUrl = (hostKey: string, start: Date, end: Date) => {
  const params = new URLSearchParams({
    start: start.toISOString(),
    end: end.toISOString(),
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

export const login = (username: string, password: string) =>
  apiCall<LoginResponse>(`${API_BASE}/api/auth/login`, "POST", { username, password });

export const setupAdmin = (username: string, password: string) =>
  apiCall<LoginResponse>(`${API_BASE}/api/auth/setup`, "POST", { username, password });

export const getMe = () =>
  apiCall<UserInfo>(`${API_BASE}/api/auth/me`, "GET");

// Batch metrics query — fetch metrics for multiple hosts in a single request
export const fetchBatchMetrics = (hostKeys: string[], start: string, end: string) =>
  apiCall<Record<string, MetricsRow[]>>(`${API_BASE}/api/metrics/batch`, "POST", {
    host_keys: hostKeys,
    start,
    end,
  });

export type { MetricsRow, HostsApiResponse };
