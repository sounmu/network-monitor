"use client";

import { useMemo } from "react";
import { Thermometer } from "lucide-react";
import { TemperatureInfo } from "@/app/types/metrics";
import { useI18n } from "@/app/i18n/I18nContext";

interface TemperatureDisplayProps {
  temperatures: TemperatureInfo[];
}

/**
 * Key sensor labels to always show (case-insensitive substring match).
 * If a "Package" sensor exists, individual "Core N" sensors are dropped
 * since Package is the summary.
 */
const KEY_PATTERNS = [
  "package",
  "tctl",
  "tdie",
  "junction",
  "composite",
  "cpu",
  "gpu",
  "nvme",
  "ssd",
  "chipset",
  "pch",
  "soc",
  "edge",
];

function filterKeyTemperatures(temps: TemperatureInfo[]): TemperatureInfo[] {
  if (temps.length <= 6) return temps;

  const hasPackage = temps.some((t) =>
    t.label.toLowerCase().includes("package")
  );

  const filtered = temps.filter((t) => {
    const lower = t.label.toLowerCase();

    // Drop individual core temps if Package exists (it's the aggregate)
    if (hasPackage && /^core \d+$/i.test(t.label.trim())) return false;

    // Drop zero / near-zero readings (often unused sensors)
    if (t.temperature_c < 1) return false;

    // Keep if matches any key pattern
    return KEY_PATTERNS.some((p) => lower.includes(p));
  });

  // If filtering removed everything, fall back to top temps by value
  if (filtered.length === 0) {
    return [...temps]
      .filter((t) => t.temperature_c >= 1)
      .sort((a, b) => b.temperature_c - a.temperature_c)
      .slice(0, 6);
  }

  // Deduplicate by label (keep higher temp if duplicated)
  const seen = new Map<string, TemperatureInfo>();
  for (const t of filtered) {
    const existing = seen.get(t.label);
    if (!existing || t.temperature_c > existing.temperature_c) {
      seen.set(t.label, t);
    }
  }

  return Array.from(seen.values());
}

function getTempColor(tempC: number): string {
  if (tempC < 60) return "var(--accent-green)";
  if (tempC <= 80) return "var(--accent-yellow)";
  return "var(--accent-red)";
}

export default function TemperatureDisplay({ temperatures }: TemperatureDisplayProps) {
  const { t } = useI18n();

  const keyTemps = useMemo(() => filterKeyTemperatures(temperatures), [temperatures]);

  if (!temperatures || temperatures.length === 0) {
    return (
      <div style={{ textAlign: "center", padding: "24px 0", color: "var(--text-muted)" }}>
        <Thermometer size={28} style={{ margin: "0 auto 8px", opacity: 0.4 }} />
        <div style={{ fontSize: 13 }}>{t.temperature.noData}</div>
      </div>
    );
  }

  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "repeat(auto-fill, minmax(160px, 1fr))",
        gap: 8,
      }}
    >
      {keyTemps.map((sensor, idx) => {
        const color = getTempColor(sensor.temperature_c);
        return (
          <div
            key={`${sensor.label}-${idx}`}
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              gap: 8,
              padding: "8px 12px",
              borderRadius: 8,
              background: "var(--bg-card-hover)",
            }}
          >
            <span
              style={{
                fontSize: 12,
                color: "var(--text-secondary)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
              title={sensor.label}
            >
              {sensor.label}
            </span>
            <span
              className="font-mono"
              style={{
                fontSize: 13,
                fontWeight: 700,
                color,
                flexShrink: 0,
              }}
            >
              {sensor.temperature_c.toFixed(0)}°
            </span>
          </div>
        );
      })}
    </div>
  );
}
