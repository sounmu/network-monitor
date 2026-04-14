// Types based on Rust MetricsRow (actual API response from DB)

/// Cumulative traffic aggregated from physical interfaces by the agent
export interface NetworkTotal {
  total_rx_bytes: number;
  total_tx_bytes: number;
}

export interface LoadAverage {
  one_min: number;
  five_min: number;
  fifteen_min: number;
}

export interface DockerContainer {
  container_name: string;
  image: string;
  state: string;   // "running" | "exited" | "dead" ...
  status: string;  // e.g. "Up 2 hours"
}

export interface PortStatus {
  port: number;
  is_open: boolean;
}

export interface DiskInfo {
  name: string;
  mount_point: string;
  total_gb: number;
  available_gb: number;
  usage_percent: number;
}

// Each row in the GET /api/metrics/:host_key response
// 1:1 mapping with the Rust MetricsRow struct
export interface MetricsRow {
  id: number;
  /** Unique identifier based on target URL — may be null for pre-migration data */
  host_key: string | null;
  /** Hostname for UI display */
  display_name: string | null;
  is_online: boolean;
  cpu_usage_percent: number;
  memory_usage_percent: number;
  load_1min: number;
  load_5min: number;
  load_15min: number;
  networks: NetworkTotal | null;
  docker_containers: DockerContainer[] | null;
  ports: PortStatus[] | null;
  disks: DiskInfo[] | null;
  processes: ProcessInfo[] | null;
  temperatures: TemperatureInfo[] | null;
  gpus: GpuInfo[] | null;
  timestamp: string;
}

// GET /api/hosts response
export interface HostSummary {
  host_key: string;
  display_name: string;
  is_online: boolean;
  last_seen: string | null;
}

export interface HostsApiResponse {
  hosts: HostSummary[];
}

// ──────────────────────────────────────────
// SSE event payload types (1:1 mapping with Rust sse_payloads.rs)
// ──────────────────────────────────────────

/// Per-second network throughput calculated server-side (aggregated from physical interfaces)
export interface NetworkRate {
  rx_bytes_per_sec: number;
  tx_bytes_per_sec: number;
}

/// event: metrics payload — CPU, memory, network speed (every 10s)
export interface HostMetricsPayload {
  /** Unique identifier based on target URL — prevents display_name collisions */
  host_key: string;
  /** Actual hostname reported by the agent — for UI display */
  display_name: string;
  is_online: boolean;
  cpu_usage_percent: number;
  memory_usage_percent: number;
  load_1min: number;
  load_5min: number;
  load_15min: number;
  network_rate: NetworkRate;
  timestamp: string;
}

/// event: status payload — Docker and port status (on initial connection + on change)
export interface HostStatusPayload {
  /** Unique identifier based on target URL — prevents display_name collisions */
  host_key: string;
  /** Actual hostname reported by the agent — for UI display */
  display_name: string;
  is_online: boolean;
  last_seen: string;
  docker_containers: DockerContainer[];
  ports: PortStatus[];
  disks: DiskInfo[];
  processes: ProcessInfo[];
  temperatures: TemperatureInfo[];
  gpus: GpuInfo[];
}

export interface ProcessInfo {
  pid: number;
  name: string;
  cpu_usage: number;
  memory_mb: number;
}

export interface TemperatureInfo {
  label: string;
  temperature_c: number;
}

export interface GpuInfo {
  name: string;
  gpu_usage_percent: number;
  memory_used_mb: number;
  memory_total_mb: number;
  temperature_c: number;
  power_watts?: number;
  frequency_mhz?: number;
}

// Chart-specific normalized type (MetricsRow -> normalized)
export interface NormalizedMetrics {
  timestamp: string;
  cpu: number;
  ram: number;
  load_1min: number;
  load_5min: number;
  load_15min: number;
  network: NetworkTotal | null;
  docker: DockerContainer[];
  ports: PortStatus[];
  is_online: boolean;
}
