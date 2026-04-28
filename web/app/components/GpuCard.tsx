"use client";

import { Monitor } from "lucide-react";
import { GpuInfo } from "@/app/types/metrics";
import { useI18n } from "@/app/i18n/I18nContext";

interface GpuCardProps {
  gpus: GpuInfo[];
}

function getUsageColor(pct: number): string {
  if (pct < 60) return "var(--accent-green)";
  if (pct < 80) return "var(--accent-yellow)";
  return "var(--accent-red)";
}

function getTempColor(tempC: number): string {
  if (tempC < 60) return "var(--accent-green)";
  if (tempC <= 80) return "var(--accent-yellow)";
  return "var(--accent-red)";
}

function formatMemory(mb: number): string {
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
  return `${mb.toFixed(0)} MB`;
}

export default function GpuCard({ gpus }: GpuCardProps) {
  const { t } = useI18n();

  if (!gpus || gpus.length === 0) {
    return (
      <div style={{ textAlign: "center", padding: "24px 0", color: "var(--text-muted)" }}>
        <Monitor size={28} style={{ margin: "0 auto 8px", opacity: 0.4 }} />
        <div style={{ fontSize: 13 }}>{t.gpu.noData}</div>
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
      {gpus.map((gpu, idx) => {
        const usagePct = Math.min(gpu.gpu_usage_percent, 100);
        const usageColor = getUsageColor(usagePct);
        const tempColor = getTempColor(gpu.temperature_c);

        return (
          <div key={`${gpu.name}-${idx}`}>
            {/* GPU name + usage % */}
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "baseline",
                marginBottom: 6,
              }}
            >
              <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                <Monitor size={14} color="var(--text-muted)" />
                <span
                  style={{
                    fontSize: 13,
                    fontWeight: 600,
                    color: "var(--text-primary)",
                  }}
                >
                  {gpu.name}
                </span>
              </div>
              <span
                className="font-mono"
                style={{ fontSize: 14, fontWeight: 700, color: usageColor }}
              >
                {usagePct.toFixed(1)}%
              </span>
            </div>

            {/* Progress bar */}
            <div
              style={{
                background: "var(--bg-card-hover)",
                borderRadius: 6,
                height: 10,
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
                  width: `${usagePct}%`,
                  background: usageColor,
                  borderRadius: 6,
                  transition: "width 0.5s ease, background 0.3s ease",
                }}
              />
            </div>

            {/* Footer: memory/power + temperature/frequency */}
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                marginTop: 4,
                fontSize: 11,
                color: "var(--text-muted)",
              }}
            >
              <span className="font-mono">
                {gpu.memory_total_mb > 0
                  ? `${t.gpu.memory}: ${formatMemory(gpu.memory_used_mb)} / ${formatMemory(gpu.memory_total_mb)}`
                  : gpu.power_watts != null
                    ? `${t.gpu.power}: ${gpu.power_watts.toFixed(1)} W`
                    : "—"}
              </span>
              <span className="font-mono" style={{ display: "flex", gap: 8 }}>
                {gpu.frequency_mhz != null && (
                  <span style={{ color: "var(--text-muted)" }}>
                    {gpu.frequency_mhz} MHz
                  </span>
                )}
                <span style={{ color: tempColor }}>
                  {gpu.temperature_c.toFixed(1)}°C
                </span>
              </span>
            </div>
            {/* Show power below memory for NVIDIA GPUs that have both */}
            {gpu.memory_total_mb > 0 && gpu.power_watts != null && (
              <div
                style={{
                  marginTop: 2,
                  fontSize: 11,
                  color: "var(--text-muted)",
                }}
              >
                <span className="font-mono">
                  {t.gpu.power}: {gpu.power_watts.toFixed(1)} W
                </span>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
