"use client";

import { LoadAverage } from "@/app/types/metrics";
import { useI18n } from "@/app/i18n/I18nContext";

interface LoadGaugeProps {
  load: LoadAverage;
  cpuCount?: number;
}

function GaugeBar({
  label,
  value,
  maxValue,
}: {
  label: string;
  value: number;
  maxValue: number;
}) {
  const pct = Math.min((value / maxValue) * 100, 100);
  const getColor = () => {
    if (pct < 50) return "var(--accent-green)";
    if (pct < 80) return "var(--accent-yellow)";
    return "var(--accent-red)";
  };

  return (
    <div style={{ marginBottom: 14 }}>
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "baseline",
          marginBottom: 6,
        }}
      >
        <span style={{ fontSize: 12, color: "var(--text-secondary)", fontWeight: 500 }}>
          {label}
        </span>
        <span
          className="font-mono"
          style={{ fontSize: 15, fontWeight: 700, color: getColor() }}
        >
          {value.toFixed(2)}
        </span>
      </div>
      <div
        style={{
          background: "var(--bg-card-hover)",
          borderRadius: 6,
          height: 8,
          overflow: "hidden",
          position: "relative",
        }}
      >
        <div
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            height: "100%",
            width: `${pct}%`,
            background: getColor(),
            borderRadius: 6,
            transition: "width 0.5s ease, background 0.3s ease",
          }}
        />
      </div>
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          marginTop: 4,
        }}
      >
        <span style={{ fontSize: 10, color: "var(--text-muted)" }}>0</span>
        <span style={{ fontSize: 10, color: "var(--text-muted)" }}>{maxValue}</span>
      </div>
    </div>
  );
}

export default function LoadGauge({ load, cpuCount = 4 }: LoadGaugeProps) {
  const { t } = useI18n();
  return (
    <div>
      <GaugeBar label={t.loadGauge.load1mAvg} value={load.one_min} maxValue={cpuCount} />
      <GaugeBar label={t.loadGauge.load5mAvg} value={load.five_min} maxValue={cpuCount} />
      <GaugeBar label={t.loadGauge.load15mAvg} value={load.fifteen_min} maxValue={cpuCount} />
      <div style={{ fontSize: 11, color: "var(--text-muted)", marginTop: 4 }}>
        {t.loadGauge.cpuCoreReference
          .replace(/\{cpuCount\}/g, String(cpuCount))}
      </div>
    </div>
  );
}
