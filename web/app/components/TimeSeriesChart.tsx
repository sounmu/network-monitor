"use client";

import { useState, useMemo, useCallback, memo } from "react";
import useSWR from "swr";
import {
  AreaChart,
  Area,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
} from "recharts";
import { fetcher, getMetricsChartRangeUrl } from "@/app/lib/api";
import {
  ChartDiskInfo,
  ChartDockerStats,
  ChartMetricsRow,
  TemperatureInfo,
} from "@/app/types/metrics";
import { formatNetworkSpeed, formatNetworkSpeedTick } from "@/app/lib/formatters";
import { mergeMetricsRows } from "@/app/lib/live-metrics";
import { useHostLiveRows } from "@/app/lib/live-metrics-store";
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

const PALETTE = [
  "hsl(220, 70%, 55%)", "hsl(160, 60%, 45%)", "hsl(30, 80%, 55%)",
  "hsl(280, 65%, 60%)", "hsl(340, 75%, 55%)", "hsl(190, 70%, 45%)",
  "hsl(50, 80%, 50%)", "hsl(0, 70%, 55%)",
];

// ─── Utilities ───────────────────────────────

function getPresetRange(minutes: number): { start: Date; end: Date } {
  const end = new Date();
  return { start: new Date(end.getTime() - minutes * 60 * 1000), end };
}

// Recharts hands the tick formatter a numeric epoch ms (the value
// from the X-axis `domain`). The previous shape converted that number
// to an ISO string at the call site, then this function called
// `new Date(isoString)` — a pointless round-trip through string
// formatting and parsing on every tick render. Take `number` directly
// so `new Date(ms)` is a single allocation per tick.
function formatAxisTime(ts: number, rangeHours: number, locale: string): string {
  const d = new Date(ts);
  const loc = locale === "ko" ? "ko-KR" : "en-US";
  if (rangeHours <= 1) return d.toLocaleTimeString(loc, { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  if (rangeHours <= 24) return d.toLocaleTimeString(loc, { hour: "2-digit", minute: "2-digit" });
  // > 24 h (7d / 30d / wide custom): drop the time component. Combined
  // hour+minute labels here got truncated by the rotated tick width and
  // added no useful resolution at tick spacings of several hours.
  return d.toLocaleDateString(loc, { month: "short", day: "numeric" });
}

/**
 * Generate evenly spaced ticks based on the **selected time range**, not data points.
 * This guarantees consistent spacing regardless of data density or gaps.
 */
function generateTimeTicks(rangeStart: Date, rangeEnd: Date, count: number): number[] {
  const startMs = rangeStart.getTime();
  const endMs = rangeEnd.getTime();
  const step = (endMs - startMs) / count;
  const ticks: number[] = [];
  for (let i = 0; i <= count; i++) {
    ticks.push(startMs + step * i);
  }
  return ticks;
}

function autoYDomainMulti(
  data: Record<string, unknown>[],
  dataKeys: string[],
  minFloor = 0
): [number, number] | ["auto", "auto"] {
  if (!data.length) return ["auto", "auto"];
  let min = Infinity, max = -Infinity, hasValue = false;
  for (const d of data) {
    for (const k of dataKeys) {
      const v = d[k];
      if (typeof v === "number" && isFinite(v)) {
        if (v < min) min = v;
        if (v > max) max = v;
        hasValue = true;
      }
    }
  }
  if (!hasValue) return ["auto", "auto"];
  const range = max - min;
  const pad = range < 0.01 ? 0.5 : range * 0.12;
  const lower = Math.max(min - pad, minFloor);
  const upper = max + pad;
  return [lower, upper > lower ? upper : lower + 1];
}

function pickCpuTemp(temps: TemperatureInfo[]): TemperatureInfo | null {
  if (!temps || temps.length === 0) return null;
  for (const p of ["package", "tctl", "tdie", "cpu"]) {
    const found = temps.find((t) => t.label.toLowerCase().includes(p) && t.temperature_c > 0);
    if (found) return found;
  }
  return temps.reduce((a, b) => (b.temperature_c > a.temperature_c ? b : a), temps[0]);
}

// ─── Styles ───────────────────────────────

const tooltipStyle: React.CSSProperties = {
  background: "var(--bg-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 10,
  fontSize: 11,
  color: "var(--text-secondary)",
  padding: "8px 12px",
  boxShadow: "0 4px 12px rgba(0,0,0,0.08)",
};

// ─── ChartCard ──────────────────────────────

interface ChartCardProps {
  title: string;
  color: string;
  isLoading: boolean;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  data: Record<string, any>[];
  dataKey: string | string[];
  colors?: string[];
  rangeHours: number;
  timeTicks: number[];
  yTickFormatter?: (val: number) => string;
  tooltipFormatter?: (val: number) => [string, string];
  yUnit?: string;
  yDomain?: [number, number] | ["auto", "auto"];
  span2?: boolean;
  height?: number;
  curveType?: "monotone" | "linear" | "stepAfter";
}

const ChartCard = memo(function ChartCard({
  title, color, isLoading, data, dataKey, colors, rangeHours, timeTicks,
  yTickFormatter, tooltipFormatter, yUnit, yDomain, span2 = false, height = 192,
  curveType = "monotone",
}: ChartCardProps) {
  const { t, locale } = useI18n();
  // Stringify the dataKey into a stable dep so `useMemo` recomputes only
  // when the underlying keys actually change. The previous shape spread
  // `dataKey` into the deps array directly, which made React see a new
  // deps array identity on every render — defeating the memoisation — and
  // is also a hooks-rule violation because the deps length varied with
  // the prop (see the now-removed `eslint-disable` comment).
  const keysSignature = Array.isArray(dataKey) ? dataKey.join("|") : dataKey;
  const keys = useMemo(
    () => (Array.isArray(dataKey) ? dataKey : [dataKey]),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [keysSignature]
  );
  const lineColors = colors ?? [color];
  const domain = useMemo(() => {
    if (yDomain) return yDomain;
    return autoYDomainMulti(data, keys);
  }, [data, yDomain, keys]);

  return (
    <div
      className="glass-card"
      style={{ padding: "16px 18px", gridColumn: span2 ? "1 / -1" : undefined }}
    >
      <div style={{ fontSize: 13, fontWeight: 600, color: "var(--text-secondary)", marginBottom: 12 }}>
        {title}
      </div>
      {isLoading ? (
        <div className="skeleton" style={{ height }} />
      ) : data.length === 0 ? (
        <div style={{ height, display: "flex", alignItems: "center", justifyContent: "center", color: "var(--text-muted)", fontSize: 12 }}>
          {t.chart.noData}
        </div>
      ) : (
        <ResponsiveContainer width="100%" height={height}>
          <AreaChart data={data} margin={{ top: 4, right: 6, bottom: 0, left: -8 }}>
            <defs>
              {keys.map((k, idx) => {
                const c = lineColors[idx % lineColors.length] ?? color;
                return (
                  <linearGradient key={k} id={`g-${k.replace(/[\s()%/]/g, "")}`} x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor={c} stopOpacity={keys.length > 4 ? 0.15 : 0.3} />
                    <stop offset="100%" stopColor={c} stopOpacity={0.02} />
                  </linearGradient>
                );
              })}
            </defs>
            <CartesianGrid vertical={false} strokeDasharray="3 3" stroke="var(--bg-card-hover)" />
            <XAxis
              dataKey="ts"
              type="number"
              scale="time"
              domain={[timeTicks[0], timeTicks[timeTicks.length - 1]]}
              ticks={timeTicks}
              // Force Recharts to HONOR the domain as a strict bound.
              // The default (`false`) treats `domain` as a minimum
              // range and silently expands it to fit any data points
              // outside — exactly what was happening on live presets
              // because `getMetricsChartRangeUrl` floors start and ceils
              // end to the nearest minute, producing a 2–3 min fetch
              // window for a 1 min chart. The ticks landed in the
              // middle of the expanded domain instead of spanning it.
              allowDataOverflow={true}
              // Disable the tick auto-drop heuristic (default
              // `preserveEnd` drops labels on width fluctuations,
              // producing a 5→3→5 flicker). `generateTimeTicks`
              // produces ~6 ticks so physical overlap is a non-issue.
              interval={0}
              minTickGap={40}
              tickFormatter={(val) => formatAxisTime(val as number, rangeHours, locale)}
              tick={{ fill: "var(--text-muted)", fontSize: 10 }}
              tickLine={false}
              axisLine={{ stroke: "var(--border-subtle)" }}
            />
            <YAxis
              domain={domain}
              tick={{ fill: "var(--text-muted)", fontSize: 10 }}
              tickLine={false}
              axisLine={false}
              tickFormatter={yTickFormatter}
              unit={yTickFormatter ? undefined : yUnit}
              width={yTickFormatter ? 68 : 48}
              minTickGap={18}
            />
            <Tooltip
              contentStyle={tooltipStyle}
              labelFormatter={(label) => new Date(label as number).toLocaleString(locale === "ko" ? "ko-KR" : "en-US")}
              formatter={
                tooltipFormatter
                  ? (value: unknown, name: string | number | undefined) => {
                      const num = typeof value === "number" ? value : Number(value);
                      const [formatted] = tooltipFormatter(num);
                      return [formatted, name ?? ""];
                    }
                  : undefined
              }
              itemSorter={(item) => -(typeof item.value === "number" ? item.value : 0)}
            />
            {keys.map((k, idx) => (
              <Area
                key={k}
                type={curveType}
                dataKey={k}
                stroke={lineColors[idx % lineColors.length] ?? color}
                strokeWidth={1.5}
                fill={`url(#g-${k.replace(/[\s()%/]/g, "")})`}
                dot={false}
                activeDot={{ r: 3, fill: lineColors[idx % lineColors.length] ?? color, stroke: "var(--bg-card)", strokeWidth: 2 }}
                isAnimationActive={false}
                connectNulls
              />
            ))}
          </AreaChart>
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

  const liveRows = useHostLiveRows(hostKey);
  const latestLiveTimestamp = liveRows.at(-1)?.timestamp ?? null;

  // For live presets (1m, 5m) the window rolls in step with **incoming
  // data**, not wall clock — Grafana-realtime style. The chart's right
  // edge anchors to the most recent SSE-pushed live-row timestamp
  // (every ~10 s), so each shift coincides with new content arriving
  // rather than the chart drifting on its own between scrapes.
  //
  // Earlier wall-clock-based cadences (1 s `nowTick` setInterval and
  // 5 s coarse grid) both produced the same complaint at different
  // pitches — small frequent stutter vs. larger periodic jumps. The
  // root cause was decoupling chart movement from data arrival; once
  // those are tied together, every motion has a meaning ("a new
  // sample landed") and the chart looks intentional, not jittery.
  //
  // Initial mount fallback uses `Date.now()` captured by the lazy
  // `useState` initializer; once an SSE event lands the anchor jumps
  // to that timestamp and stays in sync from there.
  const isLivePreset = range.preset === "1m" || range.preset === "5m";
  const [initialAnchorTs] = useState<number>(() => Date.now());
  const liveAnchorTs = useMemo(() => {
    if (!isLivePreset) return null;
    if (latestLiveTimestamp) {
      return new Date(latestLiveTimestamp).getTime();
    }
    return initialAnchorTs;
  }, [isLivePreset, latestLiveTimestamp, initialAnchorTs]);

  const effectiveRange = useMemo<TimeRange>(() => {
    if (!isLivePreset || liveAnchorTs == null) return range;
    const windowMs = range.preset === "1m" ? 60_000 : 300_000;
    return {
      start: new Date(liveAnchorTs - windowMs),
      end: new Date(liveAnchorTs),
      preset: range.preset,
    };
  }, [range, isLivePreset, liveAnchorTs]);

  const swrKey = useMemo(
    () =>
      getMetricsChartRangeUrl(
        hostKey,
        effectiveRange.start,
        effectiveRange.end,
        // Round-up granularity for the SWR cache key. Live presets used
        // to round to 10 s, which meant the URL rotated **every 10 s of
        // wall clock** — and SWR treats a different URL as a different
        // cache entry. Combined with `keepPreviousData: false`, that
        // emptied `rows` for the duration of each fetch (~hundreds of
        // ms) and produced a visible flicker right after every SSE
        // synthetic point landed.
        //
        // Bumping live presets to 30 s thirds the rotation rate. Recent
        // SSE samples are held in a tiny per-chart ring buffer, so the
        // REST baseline can lag briefly without dropping an in-between
        // point from the visible series.
        isLivePreset ? 30 : 60,
      ),
    [hostKey, effectiveRange, isLivePreset]
  );

  const { data: rows = [], isValidating } = useSWR<ChartMetricsRow[]>(swrKey, fetcher, {
    revalidateOnFocus: false,
    revalidateOnReconnect: false,
    refreshInterval: 0,
    dedupingInterval: 30000,
    // `keepPreviousData: true` even on live presets. The live-row merge
    // below filters by the visible window and de-dupes against REST rows,
    // so a stale baseline cannot draw the live points in the wrong place.
    // Dropping `rows` on every key rotation produced a 10-s-period flicker
    // while the fresh fetch was in flight.
    keepPreviousData: true,
  });

  const allRows = useMemo(
    () =>
      mergeMetricsRows(
        rows,
        liveRows,
        effectiveRange.start.getTime(),
        effectiveRange.end.getTime(),
      ),
    [rows, liveRows, effectiveRange],
  );

  const isInitialLoading = allRows.length === 0 && isValidating;

  const rangeHours = useMemo(
    () => (effectiveRange.end.getTime() - effectiveRange.start.getTime()) / (1000 * 60 * 60),
    [effectiveRange]
  );

  // Evenly spaced time ticks based on the *effective* (rolled) range
  // so the rightmost tick label tracks "now" for live presets.
  const timeTicks = useMemo(
    () => generateTimeTicks(effectiveRange.start, effectiveRange.end, 5),
    [effectiveRange]
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

  // ─── Data extraction (single pass) ──────────────
  const chartData = useMemo(() => {
    const s = [...allRows].sort(
      (a, b) => new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime()
    );

    const cpu: { ts: number; "CPU (%)": number }[] = [];
    const ram: { ts: number; "RAM (%)": number }[] = [];
    const net: { ts: number; RX: number; TX: number }[] = [];
    const diskUsageNames = new Set<string>();
    const diskUsageData: Record<string, number>[] = [];
    const diskIo: { ts: number; Read: number; Write: number }[] = [];
    const tempData: { ts: number; "CPU Temp": number }[] = [];
    const dockerCpuNames = new Set<string>();
    const dockerCpuData: Record<string, number>[] = [];
    const dockerMemNames = new Set<string>();
    const dockerMemData: Record<string, number>[] = [];

    for (let i = 0; i < s.length; i++) {
      const r = s[i];
      const tsMs = new Date(r.timestamp).getTime();

      cpu.push({ ts: tsMs, "CPU (%)": +r.cpu_usage_percent.toFixed(1) });
      ram.push({ ts: tsMs, "RAM (%)": +r.memory_usage_percent.toFixed(1) });

      // Network Bandwidth — backend-provided rate preferred.
      //
      // The backend now emits `rx_bytes_per_sec` / `tx_bytes_per_sec`
      // alongside the cumulative `total_*_bytes` counters (raw rows carry
      // either the agent-reported rate or the server delta fallback; rollup
      // rows carry the 5 min bucket average). When rate fields are present we
      // push every sample directly — including the first one, which
      // eliminates the left-edge gap the old delta-based path had on
      // 1 m / 5 m live presets.
      //
      // The `i > 0` delta-fallback branch survives for rows that pre-
      // date the 0002 rollup migration (or agents still on the old
      // version): they expose only the counters, so we differentiate
      // them here just like before.
      const currNet = r.networks;
      if (
        currNet &&
        typeof currNet.rx_bytes_per_sec === "number" &&
        typeof currNet.tx_bytes_per_sec === "number"
      ) {
        net.push({
          ts: tsMs,
          RX: +currNet.rx_bytes_per_sec.toFixed(0),
          TX: +currNet.tx_bytes_per_sec.toFixed(0),
        });
      } else if (i > 0) {
        const prev = s[i - 1];
        const prevNet = prev.networks;
        if (currNet && prevNet) {
          const dt = Math.max((tsMs - new Date(prev.timestamp).getTime()) / 1000, 1);
          const rxD = currNet.total_rx_bytes - prevNet.total_rx_bytes;
          const txD = currNet.total_tx_bytes - prevNet.total_tx_bytes;
          net.push({ ts: tsMs, RX: rxD >= 0 ? +(rxD / dt).toFixed(0) : 0, TX: txD >= 0 ? +(txD / dt).toFixed(0) : 0 });
        }
      }

      // Disk usage + I/O
      const disks = r.disks as unknown as ChartDiskInfo[] | null;
      if (disks && disks.length > 0) {
        const uPoint: Record<string, number> = { ts: tsMs };
        let totalRead = 0;
        let totalWrite = 0;
        for (const d of disks) {
          const label = d.mount_point || d.name;
          diskUsageNames.add(label);
          uPoint[label] = +d.usage_percent.toFixed(1);
          totalRead += d.read_bytes_per_sec ?? 0;
          totalWrite += d.write_bytes_per_sec ?? 0;
        }
        diskUsageData.push(uPoint);
        diskIo.push({ ts: tsMs, Read: +totalRead.toFixed(0), Write: +totalWrite.toFixed(0) });
      }

      // Docker container stats (CPU% + Memory MB)
      const dStats = r.docker_stats as unknown as ChartDockerStats[] | null;
      if (dStats && dStats.length > 0) {
        const cpuPt: Record<string, number> = { ts: tsMs };
        const memPt: Record<string, number> = { ts: tsMs };
        for (const ds of dStats) {
          dockerCpuNames.add(ds.container_name);
          dockerMemNames.add(ds.container_name);
          cpuPt[ds.container_name] = +ds.cpu_percent.toFixed(2);
          memPt[ds.container_name] = ds.memory_usage_mb;
        }
        dockerCpuData.push(cpuPt);
        dockerMemData.push(memPt);
      }

      // Temperature — single CPU sensor
      const temps = r.temperatures as unknown as TemperatureInfo[] | null;
      if (temps && temps.length > 0) {
        const main = pickCpuTemp(temps);
        if (main && main.temperature_c > 0) {
          tempData.push({ ts: tsMs, "CPU Temp": +main.temperature_c.toFixed(1) });
        }
      }
    }

    return {
      cpu, ram, net,
      diskUsageData, diskUsageKeys: [...diskUsageNames],
      diskIo,
      dockerCpuData, dockerCpuKeys: [...dockerCpuNames],
      dockerMemData, dockerMemKeys: [...dockerMemNames],
      tempData,
    };
    // `allRows` is itself derived from `[rows, liveMetrics]`, so including
    // `liveMetrics` in this deps array is redundant — every identity change
    // of `liveMetrics` already flows through `allRows` and would re-fire
    // this memo anyway. Dropping it matches the hooks-exhaustive-deps lint.
  }, [allRows]);

  const cpuDomain = useMemo(() => autoYDomainMulti(chartData.cpu, ["CPU (%)"], 0), [chartData.cpu]);
  const ramDomain = useMemo(() => autoYDomainMulti(chartData.ram, ["RAM (%)"], 0), [chartData.ram]);

  const curveType: "monotone" | "linear" = isLivePreset ? "linear" : "monotone";

  return (
    <div>
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
        <div style={{ width: 1, height: 24, background: "var(--border-subtle)", margin: "0 4px" }} />
        <DateTimePicker value={range.start} onChange={onCustomStartChange} />
        <span style={{ color: "var(--text-muted)", fontSize: 13 }}>~</span>
        <DateTimePicker value={range.end} onChange={onCustomEndChange} />
      </div>

      {/* Chart grid */}
      <div className="chart-grid">
        <ChartCard
          title={t.chart.cpuUsage}
          color="var(--accent-blue)"
          isLoading={isInitialLoading}
          data={chartData.cpu}
          dataKey="CPU (%)"
          rangeHours={rangeHours}
          timeTicks={timeTicks}
          yTickFormatter={fmtPercent}
          yDomain={cpuDomain}
          curveType={curveType}
        />
        <ChartCard
          title={t.chart.ramUsage}
          color="var(--accent-purple)"
          isLoading={isInitialLoading}
          data={chartData.ram}
          dataKey="RAM (%)"
          rangeHours={rangeHours}
          timeTicks={timeTicks}
          yTickFormatter={fmtPercent}
          yDomain={ramDomain}
          curveType={curveType}
        />

        {/* Network Bandwidth (RX + TX) */}
        <ChartCard
          title={t.chart.networkBandwidth}
          color="var(--accent-green)"
          colors={["var(--accent-green)", "var(--accent-blue)"]}
          isLoading={isInitialLoading}
          data={chartData.net}
          dataKey={["RX", "TX"]}
          rangeHours={rangeHours}
          timeTicks={timeTicks}
          yTickFormatter={formatNetworkSpeedTick}
          tooltipFormatter={fmtNetTooltip}
          curveType={curveType}
        />

        {/* CPU Temperature */}
        {chartData.tempData.length > 0 && (
          <ChartCard
            title={t.chart.cpuTemperature}
            color="var(--accent-red)"
            isLoading={isInitialLoading}
            data={chartData.tempData}
            dataKey="CPU Temp"
            rangeHours={rangeHours}
            timeTicks={timeTicks}
            yTickFormatter={fmtTemp}
            curveType={curveType}
          />
        )}

        {/* Disk Usage */}
        {chartData.diskUsageKeys.length > 0 && (
          <ChartCard
            title={t.host.diskUsage}
            color="var(--accent-yellow)"
            colors={PALETTE}
            isLoading={isInitialLoading}
            data={chartData.diskUsageData}
            dataKey={chartData.diskUsageKeys}
            rangeHours={rangeHours}
            timeTicks={timeTicks}
            yTickFormatter={fmtPercent}
            curveType={curveType}
          />
        )}

        {/* Disk I/O (Read + Write) */}
        {chartData.diskIo.length > 0 && (
          <ChartCard
            title={t.chart.diskIo}
            color="var(--accent-cyan)"
            colors={["var(--accent-cyan)", "var(--accent-purple)"]}
            isLoading={isInitialLoading}
            data={chartData.diskIo}
            dataKey={["Read", "Write"]}
            rangeHours={rangeHours}
            timeTicks={timeTicks}
            yTickFormatter={formatNetworkSpeedTick}
            tooltipFormatter={fmtIoTooltip}
            curveType={curveType}
          />
        )}

        {/* Docker CPU Usage */}
        {chartData.dockerCpuKeys.length > 0 && (
          <ChartCard
            title={t.chart.dockerCpuUsage}
            color={PALETTE[0]}
            colors={PALETTE}
            isLoading={isInitialLoading}
            data={chartData.dockerCpuData}
            dataKey={chartData.dockerCpuKeys}
            rangeHours={rangeHours}
            timeTicks={timeTicks}
            yTickFormatter={fmtPercent}
            curveType={curveType}
          />
        )}

        {/* Docker Memory */}
        {chartData.dockerMemKeys.length > 0 && (
          <ChartCard
            title={t.chart.dockerMemory}
            color={PALETTE[2]}
            colors={PALETTE}
            isLoading={isInitialLoading}
            data={chartData.dockerMemData}
            dataKey={chartData.dockerMemKeys}
            rangeHours={rangeHours}
            timeTicks={timeTicks}
            yTickFormatter={fmtMb}
            curveType={curveType}
          />
        )}
      </div>
    </div>
  );
}

// ─── Stable formatters ──

const fmtPercent = (v: number) => `${v.toFixed(1)}%`;
const fmtTemp = (v: number) => `${v.toFixed(0)}°C`;
const fmtMb = (v: number) => v >= 1024 ? `${(v / 1024).toFixed(1)}G` : `${v.toFixed(0)}M`;
const fmtNetTooltip = (v: number): [string, string] => [formatNetworkSpeed(v), ""];
const fmtIoTooltip = (v: number): [string, string] => [formatNetworkSpeed(v), ""];
