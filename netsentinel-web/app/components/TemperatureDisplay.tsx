"use client";

import { Thermometer } from "lucide-react";
import { TemperatureInfo } from "@/app/types/metrics";
import { useI18n } from "@/app/i18n/I18nContext";

interface TemperatureDisplayProps {
  temperatures: TemperatureInfo[];
}

function getTempColor(tempC: number): string {
  if (tempC < 60) return "var(--accent-green)";
  if (tempC <= 80) return "var(--accent-yellow)";
  return "var(--accent-red)";
}

export default function TemperatureDisplay({ temperatures }: TemperatureDisplayProps) {
  const { t } = useI18n();

  if (!temperatures || temperatures.length === 0) {
    return (
      <div style={{ textAlign: "center", padding: "24px 0", color: "var(--text-muted)" }}>
        <Thermometer size={28} style={{ margin: "0 auto 8px", opacity: 0.4 }} />
        <div style={{ fontSize: 13 }}>{t.temperature.noData}</div>
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      {temperatures.map((sensor, idx) => {
        const color = getTempColor(sensor.temperature_c);

        return (
          <div
            key={`${sensor.label}-${idx}`}
            style={{
              display: "flex",
              alignItems: "center",
              padding: "10px 14px",
              borderRadius: 8,
              background: "var(--bg-card-hover)",
              transition: "all 0.2s",
            }}
          >
            <Thermometer size={16} color={color} style={{ flexShrink: 0 }} />
            <div style={{ marginLeft: 12, flex: 1 }}>
              <span
                style={{
                  fontSize: 13,
                  fontWeight: 600,
                  color: "var(--text-primary)",
                }}
              >
                {sensor.label}
              </span>
            </div>
            <span
              className="font-mono"
              style={{
                fontSize: 14,
                fontWeight: 700,
                color,
              }}
            >
              {sensor.temperature_c.toFixed(1)}°C
            </span>
          </div>
        );
      })}
    </div>
  );
}
