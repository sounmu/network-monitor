"use client";

import { useState, useMemo, useCallback, memo } from "react";
import useSWR from "swr";
import { useSSE } from "@/app/lib/sse-context";
import {
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
  Legend,
} from "recharts";
import { fetcher, getMetricsRangeUrl } from "@/app/lib/api";
import { MetricsRow } from "@/app/types/metrics";
import { formatNetworkSpeed, formatNetworkSpeedTick } from "@/app/lib/formatters";
import DateTimePicker from "./DateTimePicker";
import { useI18n } from "@/app/i18n/I18nContext";

// ─── Types ───────────────────────────────────

type PresetKey = "1m" | "5m" | "1h" | "6h" | "12h" | "24h" | "7d" | "30d" | "custom";

interface TimeRange {
  start: Date;
  end: Date;
  preset: PresetKey;
}

type PresetButtonKey = Exclude<PresetKey, "custom">;

const PRESET_CONFIG: { key: PresetButtonKey; minutes: number }[] = [
  { key: "1m", minutes: 1 },
  { key: "5m", minutes: 5 },
  { key: "1h", minutes: 60 },
  { key: "6h", minutes: 60 * 6 },
  { key: "12h", minutes: 60 * 12 },
  { key: "24h", minutes: 60 * 24 },
  { key: "7d", minutes: 60 * 24 * 7 },
  { key: "30d", minutes: 60 * 24 * 30 },
];

// ─── Utilities ───────────────────────────────

function getPresetRange(minutes: number): { start: Date; end: Date } {
  const end = new Date();
  const start = new Date(end.getTime() - minutes * 60 * 1000);
  return { start, end };
}

function formatAxisTime(ts: string, rangeHours: number, locale: string): string {
  const d = new Date(ts);
  const localeStr = locale === "ko" ? "ko-KR" : "en-US";
  if (rangeHours <= 1) {
    return d.toLocaleTimeString(localeStr, { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  }
  if (rangeHours <= 24) {
    return d.toLocaleTimeString(localeStr, { hour: "2-digit", minute: "2-digit" });
  }
  return d.toLocaleDateString(localeStr, { month: "short", day: "numeric" });
}

/**
 * Auto-scaling Y-axis domain calculation
 * - Pads 10% below the data minimum and 10% above the maximum
 * - Ensures a minFloor to avoid overly granular decimal axes when values are near zero
 */
function autoYDomain(
  data: Record<string, unknown>[],
  dataKey: string,
  minFloor = 0
): [number, number] | ["auto", "auto"] {
  if (!data.length) return ["auto", "auto"];

  let min = Infinity;
  let max = -Infinity;
  let hasValue = false;

  for (let i = 0; i < data.length; i++) {
    const v = data[i][dataKey];
    if (typeof v === "number" && isFinite(v)) {
      if (v < min) min = v;
      if (v > max) max = v;
      hasValue = true;
    }
  }

  if (!hasValue) return ["auto", "auto"];

  const range = max - min;
  const padding = range < 0.01 ? 0.5 : range * 0.12;

  const lower = Math.max(min - padding, minFloor);
  const upper = max + padding;

  return [lower, upper > lower ? upper : lower + 1];
}

/**
 * Auto domain for multiple network dataKeys
 */
function autoYDomainMulti(
  data: Record<string, unknown>[],
  dataKeys: string[],
  minFloor = 0
): [number, number] | ["auto", "auto"] {
  if (!data.length) return ["auto", "auto"];

  let min = Infinity;
  let max = -Infinity;
  let hasValue = false;

  for (let i = 0; i < data.length; i++) {
    const d = data[i];
    for (let j = 0; j < dataKeys.length; j++) {
      const v = d[dataKeys[j]];
      if (typeof v === "number" && isFinite(v)) {
        if (v < min) min = v;
        if (v > max) max = v;
        hasValue = true;
      }
    }
  }

  if (!hasValue) return ["auto", "auto"];

  const range = max - min;
  const padding = range < 0.01 ? 0.5 : range * 0.12;
  const lower = Math.max(min - padding, minFloor);
  const upper = max + padding;
  return [lower, upper > lower ? upper : lower + 1];
}

// ─── Tooltip style ───────────────────────────

const tooltipStyle: React.CSSProperties = {
  background: "var(--bg-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 8,
  fontSize: 11,
  color: "var(--text-secondary)",
  padding: "8px 12px",
  boxShadow: "0 4px 12px rgba(0,0,0,0.08)",
};

// ─── Shared mini chart card ──────────────────

interface MiniChartCardProps {
  title: string;
  color: string;
  isLoading: boolean;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  data: Record<string, any>[];
  dataKey: string | string[];
  colors?: string[];
  rangeHours: number;
  yTickFormatter?: (val: number) => string;
  tooltipFormatter?: (val: number) => [string, string];
  yUnit?: string;
  yDomain?: [number, number] | ["auto", "auto"];
  span2?: boolean;
}

const MiniChartCard = memo(function MiniChartCard({
  title,
  color,
  isLoading,
  data,
  dataKey,
  colors,
  rangeHours,
  yTickFormatter,
  tooltipFormatter,
  yUnit,
  yDomain,
  span2 = false,
}: MiniChartCardProps) {
  const { t, locale } = useI18n();
  const keys = useMemo(
    () => (Array.isArray(dataKey) ? dataKey : [dataKey]),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [JSON.stringify(dataKey)]
  );
  const lineColors = colors ?? [color];

  const domain = useMemo(() => {
    if (yDomain) return yDomain;
    if (keys.length === 1) return autoYDomain(data, keys[0]);
    return autoYDomainMulti(data, keys);
  }, [data, yDomain, keys]);

  return (
    <div
      className="glass-card"
      style={{
        padding: "16px 18px",
        gridColumn: span2 ? "1 / -1" : undefined,
      }}
    >
      <div
        style={{
          fontSize: 12,
          fontWeight: 700,
          color: "var(--text-secondary)",
          marginBottom: 10,
          display: "flex",
          alignItems: "center",
          gap: 6,
        }}
      >
        <span
          style={{
            width: 8,
            height: 8,
            borderRadius: 4,
            background: color,
            display: "inline-block",
            flexShrink: 0,
          }}
        />
        {title}
      </div>

      {isLoading ? (
        <div className="skeleton" style={{ height: 160 }} />
      ) : data.length === 0 ? (
        <div
          style={{
            height: 160,
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
        <ResponsiveContainer width="100%" height={160}>
          <LineChart data={data} margin={{ top: 4, right: 6, bottom: 0, left: -8 }}>
            <CartesianGrid strokeDasharray="3 3" stroke="var(--bg-card-hover)" />
            <XAxis
              dataKey="time"
              tickFormatter={(val) => formatAxisTime(val, rangeHours, locale)}
              tick={{ fill: "var(--text-muted)", fontSize: 10 }}
              tickLine={false}
              axisLine={{ stroke: "var(--border-subtle)" }}
              interval="preserveStartEnd"
              minTickGap={60}
            />
            <YAxis
              domain={domain}
              tick={{ fill: "var(--text-muted)", fontSize: 10 }}
              tickLine={false}
              axisLine={{ stroke: "var(--border-subtle)" }}
              tickFormatter={yTickFormatter}
              unit={yTickFormatter ? undefined : yUnit}
              width={yTickFormatter ? 68 : 48}
              minTickGap={18}
            />
            <Tooltip
              contentStyle={tooltipStyle}
              labelFormatter={(label) => new Date(label as string).toLocaleString(locale === "ko" ? "ko-KR" : "en-US")}
              formatter={
                tooltipFormatter
                  ? (value: unknown) => {
                      const num = typeof value === "number" ? value : Number(value);
                      return tooltipFormatter(num);
                    }
                  : undefined
              }
            />
            {keys.length > 1 && (
              <Legend wrapperStyle={{ fontSize: 11, color: "var(--text-secondary)", paddingTop: 6 }} />
            )}
            {keys.map((k, idx) => (
              <Line
                key={k}
                type="linear"
                dataKey={k}
                stroke={lineColors[idx] ?? color}
                strokeWidth={1.8}
                dot={false}
                activeDot={{ r: 3, fill: lineColors[idx] ?? color, stroke: "var(--bg-card)", strokeWidth: 2 }}
                isAnimationActive={false}
              />
            ))}
          </LineChart>
        </ResponsiveContainer>
      )}
    </div>
  );
});

// ─── Main component ─────────────────────────

interface TimeSeriesChartProps {
  hostKey: string;
}

export default function TimeSeriesChart({ hostKey }: TimeSeriesChartProps) {
  const { t } = useI18n();
  const [range, setRange] = useState<TimeRange>(() => {
    const { start, end } = getPresetRange(60);
    return { start, end, preset: "1h" };
  });

  const { metricsMap } = useSSE();
  const liveMetrics = metricsMap[hostKey] ?? null;

  const swrKey = useMemo(
    () => getMetricsRangeUrl(hostKey, range.start, range.end),
    [hostKey, range]
  );

  const { data: rows = [], isValidating } = useSWR<MetricsRow[]>(swrKey, fetcher, {
    revalidateOnFocus: false,
    revalidateOnReconnect: false,
    refreshInterval: 0,
    dedupingInterval: 30000,
    keepPreviousData: true,
  });

  // Merge REST data + SSE latest data point
  const allRows = useMemo(() => {
    if (!liveMetrics) return rows;

    const lastRestTs = rows.length > 0
      ? new Date(rows[rows.length - 1].timestamp).getTime()
      : 0;
    const liveTs = new Date(liveMetrics.timestamp).getTime();
    if (liveTs <= lastRestTs) return rows;

    const syntheticRow: MetricsRow = {
      id: 0,
      host_key: liveMetrics.host_key,
      display_name: liveMetrics.display_name,
      is_online: liveMetrics.is_online,
      cpu_usage_percent: liveMetrics.cpu_usage_percent,
      memory_usage_percent: liveMetrics.memory_usage_percent,
      load_1min: liveMetrics.load_1min,
      load_5min: liveMetrics.load_5min,
      load_15min: liveMetrics.load_15min,
      networks: null,
      docker_containers: null,
      ports: null,
      disks: null,
      processes: null,
      temperatures: null,
      gpus: null,
      timestamp: liveMetrics.timestamp,
    };
    return [...rows, syntheticRow];
  }, [rows, liveMetrics]);

  const isInitialLoading = allRows.length === 0 && isValidating;

  const rangeHours = useMemo(
    () => (range.end.getTime() - range.start.getTime()) / (1000 * 60 * 60),
    [range]
  );

  const onPresetClick = useCallback((minutes: number, key: PresetKey) => {
    const { start, end } = getPresetRange(minutes);
    setRange({ start, end, preset: key });
  }, []);

  const onCustomStartChange = useCallback((date: Date) => {
    setRange((prev) => ({ ...prev, start: date, preset: "custom" }));
  }, []);

  const onCustomEndChange = useCallback((date: Date) => {
    setRange((prev) => ({ ...prev, end: date, preset: "custom" }));
  }, []);

  // ─── Chart data transformation (single-pass) ──
  // All chart datasets derived in one useMemo to avoid 5 separate dependency checks
  // and 5 separate iterations over the sorted array.

  // Stable i18n label references — avoids recalculating all chart data on locale change
  const loadLabels = useMemo(
    () => ({ l1: t.chart.load1m, l5: t.chart.load5m, l15: t.chart.load15m }),
    [t.chart.load1m, t.chart.load5m, t.chart.load15m]
  );

  const { cpuData, ramData, rxData, txData, loadData, sorted } = useMemo(() => {
    const s = [...allRows].sort(
      (a, b) => new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime()
    );

    const cpu: { time: string; "CPU (%)": number }[] = [];
    const ram: { time: string; "RAM (%)": number }[] = [];
    const load: Record<string, string | number>[] = [];
    const rx: { time: string; RX: number }[] = [];
    const tx: { time: string; TX: number }[] = [];

    for (let i = 0; i < s.length; i++) {
      const r = s[i];
      cpu.push({ time: r.timestamp, "CPU (%)": +r.cpu_usage_percent.toFixed(1) });
      ram.push({ time: r.timestamp, "RAM (%)": +r.memory_usage_percent.toFixed(1) });
      load.push({
        time: r.timestamp,
        [loadLabels.l1]: +r.load_1min.toFixed(2),
        [loadLabels.l5]: +r.load_5min.toFixed(2),
        [loadLabels.l15]: +r.load_15min.toFixed(2),
      });

      // Network delta (requires previous row)
      if (i > 0) {
        const prev = s[i - 1];
        const currNet = r.networks;
        const prevNet = prev.networks;
        if (currNet && prevNet) {
          const dtSec = Math.max(
            (new Date(r.timestamp).getTime() - new Date(prev.timestamp).getTime()) / 1000,
            1,
          );
          const rxDelta = currNet.total_rx_bytes - prevNet.total_rx_bytes;
          const txDelta = currNet.total_tx_bytes - prevNet.total_tx_bytes;
          rx.push({ time: r.timestamp, RX: rxDelta >= 0 ? +(rxDelta / dtSec).toFixed(0) : 0 });
          tx.push({ time: r.timestamp, TX: txDelta >= 0 ? +(txDelta / dtSec).toFixed(0) : 0 });
        }
      }
    }

    return { cpuData: cpu, ramData: ram, rxData: rx, txData: tx, loadData: load, sorted: s };
  }, [allRows, loadLabels]);

  // Memoize Y-axis domains — stabilizes array references so MiniChartCard's memo works
  const { cpuDomain, ramDomain } = useMemo(() => ({
    cpuDomain: autoYDomain(cpuData, "CPU (%)", 0),
    ramDomain: autoYDomain(ramData, "RAM (%)", 0),
  }), [cpuData, ramData]);

  // Latest summary: prefer SSE live data, fall back to last item of sorted array
  const latestFromRows = sorted.length > 0 ? sorted[sorted.length - 1] : null;
  const latest = liveMetrics ?? latestFromRows;

  return (
    <div>
      {/* Current status summary card */}
      {latest && (
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(auto-fit, minmax(130px, 1fr))",
            gap: 10,
            marginBottom: 14,
          }}
        >
          <SummaryCard
            label="CPU"
            value={`${latest.cpu_usage_percent.toFixed(1)}%`}
            color={thresholdColor(latest.cpu_usage_percent, 50, 80)}
          />
          <SummaryCard
            label="RAM"
            value={`${latest.memory_usage_percent.toFixed(1)}%`}
            color={thresholdColor(latest.memory_usage_percent)}
          />
          <SummaryCard
            label="Load (1m)"
            value={latest.load_1min.toFixed(2)}
            color="var(--accent-purple)"
          />
          <SummaryCard
            label={t.chart.dataPoints}
            value={`${allRows.length}`}
            color="var(--accent-blue)"
          />
        </div>
      )}

      {/* Time range controls */}
      <div className="time-controls">
        {PRESET_CONFIG.map(({ key, minutes }) => (
          <button
            key={key}
            className={`preset-btn ${range.preset === key ? "active" : ""}`}
            onClick={() => onPresetClick(minutes, key)}
          >
            {t.chart.presets[key]}
          </button>
        ))}
        <div
          style={{ width: 1, height: 24, background: "var(--border-subtle)", margin: "0 4px" }}
        />
        <DateTimePicker value={range.start} onChange={onCustomStartChange} />
        <span style={{ color: "var(--text-muted)", fontSize: 13 }}>~</span>
        <DateTimePicker value={range.end} onChange={onCustomEndChange} />
      </div>

      {/* 5-chart grid */}
      <div className="chart-grid">
        <MiniChartCard
          title={t.chart.cpuUsage}
          color="var(--accent-blue)"
          isLoading={isInitialLoading}
          data={cpuData}
          dataKey="CPU (%)"
          rangeHours={rangeHours}
          yTickFormatter={fmtPercent}
          yDomain={cpuDomain}
        />

        <MiniChartCard
          title={t.chart.ramUsage}
          color="var(--accent-cyan)"
          isLoading={isInitialLoading}
          data={ramData}
          dataKey="RAM (%)"
          rangeHours={rangeHours}
          yTickFormatter={fmtPercent}
          yDomain={ramDomain}
        />

        <MiniChartCard
          title="Network In (RX)"
          color="var(--accent-green)"
          isLoading={isInitialLoading}
          data={rxData}
          dataKey="RX"
          rangeHours={rangeHours}
          yTickFormatter={formatNetworkSpeedTick}
          tooltipFormatter={fmtRxTooltip}
        />

        <MiniChartCard
          title="Network Out (TX)"
          color="var(--accent-yellow)"
          isLoading={isInitialLoading}
          data={txData}
          dataKey="TX"
          rangeHours={rangeHours}
          yTickFormatter={formatNetworkSpeedTick}
          tooltipFormatter={fmtTxTooltip}
        />

        <MiniChartCard
          title="Load Average"
          color="var(--accent-blue)"
          colors={["var(--accent-blue)", "var(--accent-purple)", "var(--accent-yellow)"]}
          isLoading={isInitialLoading}
          data={loadData}
          dataKey={[loadLabels.l1, loadLabels.l5, loadLabels.l15]}
          rangeHours={rangeHours}
          yTickFormatter={fmtLoad}
          span2
        />
      </div>
    </div>
  );
}

// ─── Stable formatter references (avoids re-creating inline closures on every render) ──

const fmtPercent = (v: number) => `${v.toFixed(1)}%`;
const fmtLoad = (v: number) => v.toFixed(2);
const fmtRxTooltip = (v: number): [string, string] => [formatNetworkSpeed(v), "RX"];
const fmtTxTooltip = (v: number): [string, string] => [formatNetworkSpeed(v), "TX"];

// ─── Sub-components ─────────────────────────

const SummaryCard = memo(function SummaryCard({
  label,
  value,
  color,
}: {
  label: string;
  value: string;
  color: string;
}) {
  return (
    <div
      style={{
        background: "var(--bg-card)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 8,
        padding: "12px 14px",
      }}
    >
      <div
        style={{
          fontSize: 11,
          color: "var(--text-muted)",
          fontWeight: 500,
          marginBottom: 4,
        }}
      >
        {label}
      </div>
      <div
        style={{ fontSize: 20, fontWeight: 700, color, lineHeight: 1, fontFamily: "var(--font-mono), monospace" }}
      >
        {value}
      </div>
    </div>
  );
});

function thresholdColor(pct: number, warn = 65, danger = 85): string {
  if (pct >= danger) return "var(--accent-red)";
  if (pct >= warn) return "var(--accent-yellow)";
  return "var(--accent-green)";
}
