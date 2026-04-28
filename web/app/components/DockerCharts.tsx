"use client";

import { useMemo, useState } from "react";
import useSWR from "swr";
import {
  AreaChart,
  Area,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
  Legend,
} from "recharts";
import { fetcher, getMetricsRangeUrl } from "@/app/lib/api";
import { MetricsRow, DockerContainerStats } from "@/app/types/metrics";
import { useSSE } from "@/app/lib/sse-context";
import { useI18n } from "@/app/i18n/I18nContext";

// ─── Chart palette (per-container colors, cycling) ──────

const CONTAINER_COLORS = [
  "hsl(220, 70%, 55%)",  // blue
  "hsl(160, 60%, 45%)",  // green
  "hsl(30, 80%, 55%)",   // orange
  "hsl(280, 65%, 60%)",  // purple
  "hsl(340, 75%, 55%)",  // pink
  "hsl(190, 70%, 45%)",  // teal
  "hsl(50, 80%, 50%)",   // gold
  "hsl(0, 70%, 55%)",    // red
];

const tooltipStyle: React.CSSProperties = {
  background: "var(--bg-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 10,
  fontSize: 11,
  color: "var(--text-secondary)",
  padding: "8px 12px",
  boxShadow: "0 4px 12px rgba(0,0,0,0.08)",
};

interface DockerChartsProps {
  hostKey: string;
}

type MetricMode = "cpu" | "memory";

function formatAxisTime(ts: string, locale: string): string {
  const d = new Date(ts);
  const loc = locale === "ko" ? "ko-KR" : "en-US";
  return d.toLocaleTimeString(loc, { hour: "2-digit", minute: "2-digit" });
}

export default function DockerCharts({ hostKey }: DockerChartsProps) {
  const { t, locale } = useI18n();
  const { statusMap } = useSSE();
  const [mode, setMode] = useState<MetricMode>("cpu");

  // Fetch last 1 hour of data (SWR deduplicates with TimeSeriesChart if same range)
  const swrKey = useMemo(() => {
    const end = new Date();
    const start = new Date(end.getTime() - 60 * 60 * 1000);
    return getMetricsRangeUrl(hostKey, start, end);
  }, [hostKey]);

  const { data: rows = [] } = useSWR<MetricsRow[]>(swrKey, fetcher, {
    revalidateOnFocus: false,
    refreshInterval: 0,
    dedupingInterval: 30000,
    keepPreviousData: true,
  });

  // Live docker_stats from SSE for the latest point.
  // Wrapped in useMemo so the `[] fallback` doesn't allocate a new array
  // on every render — that would invalidate the downstream useMemo deps.
  const liveStats = useMemo(
    () => statusMap[hostKey]?.docker_stats ?? [],
    [statusMap, hostKey]
  );

  // Build per-container time-series from historical MetricsRow.docker_stats
  const { chartData, containerNames } = useMemo(() => {
    const sorted = [...rows].sort(
      (a, b) => new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime()
    );

    // Discover all container names across all rows
    const nameSet = new Set<string>();
    for (const row of sorted) {
      const stats: DockerContainerStats[] | null =
        row.docker_stats as unknown as DockerContainerStats[] | null;
      if (stats) {
        for (const s of stats) nameSet.add(s.container_name);
      }
    }
    // Also include live containers
    for (const s of liveStats) nameSet.add(s.container_name);

    const names = Array.from(nameSet).sort();

    // Build chart data: each point is { time, "container1": value, "container2": value, ... }
    const data: Record<string, string | number>[] = [];

    for (const row of sorted) {
      const stats: DockerContainerStats[] | null =
        row.docker_stats as unknown as DockerContainerStats[] | null;
      if (!stats || stats.length === 0) continue;

      const point: Record<string, string | number> = { time: row.timestamp };
      const statsMap = new Map<string, DockerContainerStats>();
      for (const s of stats) statsMap.set(s.container_name, s);

      for (const name of names) {
        const s = statsMap.get(name);
        if (s) {
          point[name] = mode === "cpu"
            ? +s.cpu_percent.toFixed(2)
            : s.memory_usage_mb;
        }
      }
      data.push(point);
    }

    // Append live SSE point if newer
    if (liveStats.length > 0) {
      const lastTs = data.length > 0 ? new Date(data[data.length - 1].time as string).getTime() : 0;
      const liveTs = statusMap[hostKey]?.last_seen;
      if (liveTs && new Date(liveTs).getTime() > lastTs) {
        const point: Record<string, string | number> = { time: liveTs };
        for (const s of liveStats) {
          point[s.container_name] = mode === "cpu"
            ? +s.cpu_percent.toFixed(2)
            : s.memory_usage_mb;
        }
        data.push(point);
      }
    }

    return { chartData: data, containerNames: names };
  }, [rows, liveStats, mode, hostKey, statusMap]);

  if (containerNames.length === 0) return null;

  const yFormatter = mode === "cpu"
    ? (v: number) => `${v.toFixed(1)}%`
    : (v: number) => `${v}`;

  return (
    <div>
      {/* Mode toggle */}
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 12 }}>
        <div
          style={{
            display: "inline-flex",
            borderRadius: 8,
            border: "1px solid var(--border-subtle)",
            overflow: "hidden",
            fontSize: 12,
            fontWeight: 600,
          }}
        >
          <button
            onClick={() => setMode("cpu")}
            style={{
              padding: "5px 16px",
              border: "none",
              cursor: "pointer",
              background: mode === "cpu" ? "var(--accent-blue)" : "transparent",
              color: mode === "cpu" ? "var(--text-on-accent, #fff)" : "var(--text-muted)",
              transition: "all 0.15s ease",
            }}
          >
            CPU
          </button>
          <button
            onClick={() => setMode("memory")}
            style={{
              padding: "5px 16px",
              border: "none",
              borderLeft: "1px solid var(--border-subtle)",
              cursor: "pointer",
              background: mode === "memory" ? "var(--accent-purple)" : "transparent",
              color: mode === "memory" ? "var(--text-on-accent, #fff)" : "var(--text-muted)",
              transition: "all 0.15s ease",
            }}
          >
            Memory
          </button>
        </div>
        <span style={{ fontSize: 11, color: "var(--text-muted)" }}>
          {containerNames.length} containers
        </span>
      </div>

      {/* Chart */}
      {chartData.length === 0 ? (
        <div
          style={{
            height: 200,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            color: "var(--text-muted)",
            fontSize: 12,
          }}
        >
          {t.chart.noData}
        </div>
      ) : (
        <ResponsiveContainer width="100%" height={240}>
          <AreaChart data={chartData} margin={{ top: 4, right: 6, bottom: 0, left: -8 }}>
            <defs>
              {containerNames.map((name, idx) => {
                const color = CONTAINER_COLORS[idx % CONTAINER_COLORS.length];
                return (
                  <linearGradient key={name} id={`docker-grad-${idx}`} x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor={color} stopOpacity={0.25} />
                    <stop offset="100%" stopColor={color} stopOpacity={0.02} />
                  </linearGradient>
                );
              })}
            </defs>
            <CartesianGrid vertical={false} strokeDasharray="3 3" stroke="var(--bg-card-hover)" />
            <XAxis
              dataKey="time"
              tickFormatter={(val) => formatAxisTime(val, locale)}
              tick={{ fill: "var(--text-muted)", fontSize: 10 }}
              tickLine={false}
              axisLine={{ stroke: "var(--border-subtle)" }}
              interval="preserveStartEnd"
              minTickGap={40}
            />
            <YAxis
              tick={{ fill: "var(--text-muted)", fontSize: 10 }}
              tickLine={false}
              axisLine={false}
              tickFormatter={yFormatter}
              width={52}
              minTickGap={18}
            />
            <Tooltip
              contentStyle={tooltipStyle}
              labelFormatter={(label) =>
                new Date(label as string).toLocaleString(locale === "ko" ? "ko-KR" : "en-US")
              }
              formatter={(value: unknown) => {
                const num = typeof value === "number" ? value : Number(value);
                return mode === "cpu" ? `${num.toFixed(2)}%` : `${num} MB`;
              }}
            />
            <Legend
              wrapperStyle={{ fontSize: 11, color: "var(--text-secondary)", paddingTop: 6 }}
            />
            {containerNames.map((name, idx) => {
              const color = CONTAINER_COLORS[idx % CONTAINER_COLORS.length];
              return (
                <Area
                  key={name}
                  type="monotone"
                  dataKey={name}
                  stroke={color}
                  strokeWidth={1.5}
                  fill={`url(#docker-grad-${idx})`}
                  dot={false}
                  activeDot={{ r: 3, fill: color, stroke: "var(--bg-card)", strokeWidth: 2 }}
                  isAnimationActive={false}
                  connectNulls
                />
              );
            })}
          </AreaChart>
        </ResponsiveContainer>
      )}
    </div>
  );
}
