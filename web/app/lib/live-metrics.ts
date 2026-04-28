import type { ChartMetricsRow, HostMetricsPayload } from "@/app/types/metrics";

export const LIVE_METRICS_BUFFER_MS = 6 * 60 * 1000;
export const LIVE_METRICS_MAX_POINTS = 64;

const REST_LIVE_DEDUPE_MS = 1500;
const SECOND_MS = 1000;

function timestampMs(timestamp: string): number {
  return new Date(timestamp).getTime();
}

function rowTimestampMs(row: Pick<ChartMetricsRow, "timestamp">): number {
  return timestampMs(row.timestamp);
}

function secondBucket(tsMs: number): number {
  return Math.round(tsMs / SECOND_MS);
}

function addToSecondIndex(index: Map<number, number[]>, tsMs: number) {
  const bucket = secondBucket(tsMs);
  const entries = index.get(bucket);
  if (entries) {
    entries.push(tsMs);
  } else {
    index.set(bucket, [tsMs]);
  }
}

function hasNearbyTimestamp(index: Map<number, number[]>, tsMs: number): boolean {
  const bucket = secondBucket(tsMs);
  for (let b = bucket - 1; b <= bucket + 1; b++) {
    const entries = index.get(b);
    if (entries?.some((candidate) => Math.abs(candidate - tsMs) <= REST_LIVE_DEDUPE_MS)) {
      return true;
    }
  }
  return false;
}

export function liveMetricsToRow(liveMetrics: HostMetricsPayload): ChartMetricsRow {
  return {
    id: 0,
    host_key: liveMetrics.host_key,
    display_name: liveMetrics.display_name,
    is_online: liveMetrics.is_online,
    cpu_usage_percent: liveMetrics.cpu_usage_percent,
    memory_usage_percent: liveMetrics.memory_usage_percent,
    load_1min: liveMetrics.load_1min,
    load_5min: liveMetrics.load_5min,
    load_15min: liveMetrics.load_15min,
    networks: {
      total_rx_bytes: liveMetrics.network_rate.total_rx_bytes,
      total_tx_bytes: liveMetrics.network_rate.total_tx_bytes,
      rx_bytes_per_sec: liveMetrics.network_rate.rx_bytes_per_sec,
      tx_bytes_per_sec: liveMetrics.network_rate.tx_bytes_per_sec,
    },
    disks: (liveMetrics.disks ?? []).map((d) => ({
      name: d.name,
      mount_point: d.mount_point,
      usage_percent: d.usage_percent,
      read_bytes_per_sec: d.read_bytes_per_sec,
      write_bytes_per_sec: d.write_bytes_per_sec,
    })),
    temperatures: liveMetrics.temperatures ?? [],
    docker_stats: (liveMetrics.docker_stats ?? []).map((s) => ({
      container_name: s.container_name,
      cpu_percent: s.cpu_percent,
      memory_usage_mb: s.memory_usage_mb,
    })),
    timestamp: liveMetrics.timestamp,
  };
}

export function appendLiveMetricRow(
  previousRows: readonly ChartMetricsRow[],
  liveMetrics: HostMetricsPayload,
): readonly ChartMetricsRow[] {
  const row = liveMetricsToRow(liveMetrics);
  const rowTs = rowTimestampMs(row);
  // Invalid timestamp (NaN / Infinity from a corrupted SSE payload) means
  // this row cannot be ordered against others. Return the *same* reference
  // so callers can detect "nothing changed" via `===` and skip notifying
  // subscribers — avoids a spurious `useSyncExternalStore` re-render on
  // every malformed event.
  if (!Number.isFinite(rowTs)) return previousRows;

  const cutoffTs = rowTs - LIVE_METRICS_BUFFER_MS;
  const nextRows: ChartMetricsRow[] = [];

  for (const previous of previousRows) {
    const previousTs = rowTimestampMs(previous);
    if (!Number.isFinite(previousTs)) continue;
    if (previousTs < cutoffTs) continue;
    if (Math.abs(previousTs - rowTs) <= REST_LIVE_DEDUPE_MS) continue;
    nextRows.push(previous);
  }

  nextRows.push(row);
  nextRows.sort((a, b) => rowTimestampMs(a) - rowTimestampMs(b));

  if (nextRows.length > LIVE_METRICS_MAX_POINTS) {
    return nextRows.slice(nextRows.length - LIVE_METRICS_MAX_POINTS);
  }
  return nextRows;
}

export function mergeMetricsRows(
  restRows: readonly ChartMetricsRow[],
  liveRows: readonly ChartMetricsRow[],
  rangeStartMs: number,
  rangeEndMs: number,
): readonly ChartMetricsRow[] {
  if (liveRows.length === 0) return restRows;

  const secondIndex = new Map<number, number[]>();
  for (const row of restRows) {
    const ts = rowTimestampMs(row);
    if (Number.isFinite(ts)) {
      addToSecondIndex(secondIndex, ts);
    }
  }

  const visibleLiveRows: ChartMetricsRow[] = [];
  for (const row of liveRows) {
    const ts = rowTimestampMs(row);
    if (!Number.isFinite(ts)) continue;
    if (ts < rangeStartMs || ts > rangeEndMs) continue;
    if (hasNearbyTimestamp(secondIndex, ts)) continue;

    visibleLiveRows.push(row);
    addToSecondIndex(secondIndex, ts);
  }

  if (visibleLiveRows.length === 0) return restRows;

  return [...restRows, ...visibleLiveRows].sort(
    (a, b) => rowTimestampMs(a) - rowTimestampMs(b),
  );
}
