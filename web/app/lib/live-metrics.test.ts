import { describe, expect, it } from "vitest";
import type { ChartMetricsRow, HostMetricsPayload } from "@/app/types/metrics";
import {
  LIVE_METRICS_MAX_POINTS,
  appendLiveMetricRow,
  mergeMetricsRows,
} from "./live-metrics";
// Cleanup-test imports come from the store rather than `live-metrics`
// because the store owns the per-host state lifecycle.
import {
  __hasLiveMetricEntry,
  clearLiveMetricRows,
  pushLiveMetricPayload,
} from "./live-metrics-store";

function payload(timestamp: string, cpu = 10): HostMetricsPayload {
  return {
    host_key: "box:9101",
    display_name: "box",
    is_online: true,
    cpu_usage_percent: cpu,
    memory_usage_percent: 20,
    load_1min: 0.1,
    load_5min: 0.2,
    load_15min: 0.3,
    network_rate: {
      total_rx_bytes: 1000,
      total_tx_bytes: 2000,
      rx_bytes_per_sec: 10,
      tx_bytes_per_sec: 20,
    },
    cpu_cores: [],
    network_interface_rates: [],
    disks: [],
    temperatures: [],
    docker_stats: [],
    timestamp,
  };
}

function restRow(timestamp: string, id = 1): ChartMetricsRow {
  return {
    id,
    host_key: "box:9101",
    display_name: "box",
    is_online: true,
    cpu_usage_percent: 1,
    memory_usage_percent: 2,
    load_1min: 0.1,
    load_5min: 0.2,
    load_15min: 0.3,
    networks: null,
    disks: [],
    temperatures: [],
    docker_stats: [],
    timestamp,
  };
}

describe("appendLiveMetricRow", () => {
  it("keeps intermediate SSE samples until REST catches up", () => {
    let rows: readonly ChartMetricsRow[] = [];
    rows = appendLiveMetricRow(rows, payload("2026-04-25T00:00:02.000Z", 2));
    rows = appendLiveMetricRow(rows, payload("2026-04-25T00:00:12.000Z", 12));
    rows = appendLiveMetricRow(rows, payload("2026-04-25T00:00:22.000Z", 22));

    expect(rows.map((row) => row.timestamp)).toEqual([
      "2026-04-25T00:00:02.000Z",
      "2026-04-25T00:00:12.000Z",
      "2026-04-25T00:00:22.000Z",
    ]);
  });

  it("caps the live buffer", () => {
    let rows: readonly ChartMetricsRow[] = [];
    for (let i = 0; i < LIVE_METRICS_MAX_POINTS + 10; i++) {
      rows = appendLiveMetricRow(
        rows,
        payload(new Date(Date.UTC(2026, 3, 25, 0, 0, i * 2)).toISOString(), i),
      );
    }

    expect(rows).toHaveLength(LIVE_METRICS_MAX_POINTS);
    expect(rows[0].timestamp).toBe("2026-04-25T00:00:20.000Z");
  });
});

describe("mergeMetricsRows", () => {
  it("fills gaps between REST refreshes with buffered live rows", () => {
    const restRows = [restRow("2026-04-25T00:00:02.000Z", 1)];
    const liveRows = [
      restRow("2026-04-25T00:00:12.000Z", 0),
      restRow("2026-04-25T00:00:22.000Z", 0),
    ];

    const merged = mergeMetricsRows(
      restRows,
      liveRows,
      Date.parse("2026-04-25T00:00:00.000Z"),
      Date.parse("2026-04-25T00:00:30.000Z"),
    );

    expect(merged.map((row) => row.timestamp)).toEqual([
      "2026-04-25T00:00:02.000Z",
      "2026-04-25T00:00:12.000Z",
      "2026-04-25T00:00:22.000Z",
    ]);
  });

  it("prefers REST rows when a live row is already persisted nearby", () => {
    const restRows = [restRow("2026-04-25T00:00:13.000Z", 42)];
    const liveRows = [restRow("2026-04-25T00:00:12.200Z", 0)];

    const merged = mergeMetricsRows(
      restRows,
      liveRows,
      Date.parse("2026-04-25T00:00:00.000Z"),
      Date.parse("2026-04-25T00:00:30.000Z"),
    );

    expect(merged).toHaveLength(1);
    expect(merged[0].id).toBe(42);
  });

  it("filters live rows that fall outside the visible window", () => {
    const restRows = [restRow("2026-04-25T00:00:30.000Z", 1)];
    const liveRows = [
      // before-window
      restRow("2026-04-24T23:59:55.000Z", 0),
      // in-window
      restRow("2026-04-25T00:00:25.000Z", 0),
      // after-window
      restRow("2026-04-25T00:01:05.000Z", 0),
    ];

    const merged = mergeMetricsRows(
      restRows,
      liveRows,
      Date.parse("2026-04-25T00:00:00.000Z"),
      Date.parse("2026-04-25T00:01:00.000Z"),
    );

    expect(merged.map((r) => r.timestamp)).toEqual([
      "2026-04-25T00:00:25.000Z",
      "2026-04-25T00:00:30.000Z",
    ]);
  });
});

describe("appendLiveMetricRow edge cases", () => {
  it("returns the same reference when timestamp is unparseable", () => {
    const previous: readonly ChartMetricsRow[] = [];
    const next = appendLiveMetricRow(previous, payload("not-a-real-date"));
    // Same identity → caller can detect "no change" via `===` and skip
    // notifying subscribers.
    expect(next).toBe(previous);
  });

  it("preserves timestamp ordering across out-of-order arrivals", () => {
    let rows: readonly ChartMetricsRow[] = [];
    rows = appendLiveMetricRow(rows, payload("2026-04-25T00:00:30.000Z", 30));
    // Earlier timestamp lands AFTER a later one — still slots into the
    // correct position via the internal sort.
    rows = appendLiveMetricRow(rows, payload("2026-04-25T00:00:10.000Z", 10));
    rows = appendLiveMetricRow(rows, payload("2026-04-25T00:00:20.000Z", 20));

    expect(rows.map((r) => r.timestamp)).toEqual([
      "2026-04-25T00:00:10.000Z",
      "2026-04-25T00:00:20.000Z",
      "2026-04-25T00:00:30.000Z",
    ]);
  });
});

describe("live-metrics-store cleanup", () => {
  it("does not allocate a per-host buffer without an active chart subscriber", () => {
    const hostKey = "cleanup-test:9101";
    pushLiveMetricPayload({
      ...payload("2026-04-25T00:00:00.000Z"),
      host_key: hostKey,
    });
    expect(__hasLiveMetricEntry(hostKey)).toBe(false);

    clearLiveMetricRows(hostKey);
    expect(__hasLiveMetricEntry(hostKey)).toBe(false);
  });

  it("ignores corrupted SSE payloads with non-finite timestamps", () => {
    const hostKey = "corrupt-test:9101";
    pushLiveMetricPayload({
      ...payload("2026-04-25T00:00:00.000Z"),
      host_key: hostKey,
    });
    // Snapshot the buffer state before the bad event.
    const before = __hasLiveMetricEntry(hostKey);

    // Push a garbage timestamp. The append helper returns the same
    // reference, so the store should not emit / mutate.
    pushLiveMetricPayload({
      ...payload("garbage"),
      host_key: hostKey,
    });

    expect(__hasLiveMetricEntry(hostKey)).toBe(before);

    // Cleanup so subsequent runs in the same process start clean.
    clearLiveMetricRows(hostKey);
  });
});
