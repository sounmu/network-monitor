"use client";

import { HardDrive } from "lucide-react";
import { DiskInfo } from "@/app/types/metrics";
import { formatNetworkSpeed } from "@/app/lib/formatters";
import { useI18n } from "@/app/i18n/I18nContext";

interface DiskUsageBarProps {
  disks: DiskInfo[];
}

function formatSize(gb: number): string {
  if (gb >= 1000) return `${(gb / 1000).toFixed(1)} TB`;
  if (gb >= 1) return `${gb.toFixed(1)} GB`;
  return `${(gb * 1024).toFixed(0)} MB`;
}

function getColor(pct: number): string {
  if (pct < 60) return "var(--accent-green)";
  if (pct < 80) return "var(--accent-yellow)";
  return "var(--accent-red)";
}

export default function DiskUsageBar({ disks }: DiskUsageBarProps) {
  const { t } = useI18n();

  if (!disks || disks.length === 0) {
    return (
      <div style={{ textAlign: "center", padding: "24px 0", color: "var(--text-muted)" }}>
        <HardDrive size={28} style={{ margin: "0 auto 8px", opacity: 0.4 }} />
        <div style={{ fontSize: 13 }}>{t.disk.noDisks}</div>
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
      {disks.map((disk) => {
        const usedGb = disk.total_gb - disk.available_gb;
        const pct = Math.min(disk.usage_percent, 100);
        const color = getColor(pct);

        return (
          <div key={disk.mount_point}>
            {/* Header: mount point + usage % */}
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "baseline",
                marginBottom: 6,
              }}
            >
              <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                <HardDrive size={14} color="var(--text-muted)" />
                <span
                  style={{
                    fontSize: 13,
                    fontWeight: 600,
                    color: "var(--text-primary)",
                    fontFamily: "var(--font-mono), monospace",
                  }}
                >
                  {disk.mount_point}
                </span>
                {disk.name && (
                  <span style={{ fontSize: 11, color: "var(--text-muted)" }}>
                    ({disk.name})
                  </span>
                )}
              </div>
              <span
                className="font-mono"
                style={{ fontSize: 14, fontWeight: 700, color }}
              >
                {pct.toFixed(1)}%
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
                  width: `${pct}%`,
                  background: color,
                  borderRadius: 6,
                  transition: "width 0.5s ease, background 0.3s ease",
                }}
              />
            </div>

            {/* Footer: used / total + I/O speeds */}
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                marginTop: 4,
                fontSize: 11,
                color: "var(--text-muted)",
              }}
            >
              <span>
                {t.disk.used}: {formatSize(usedGb)}
              </span>
              <span>
                {t.disk.free}: {formatSize(disk.available_gb)} / {t.disk.total}: {formatSize(disk.total_gb)}
              </span>
            </div>
            {(disk.read_bytes_per_sec > 0 || disk.write_bytes_per_sec > 0) && (
              <div
                style={{
                  display: "flex",
                  gap: 12,
                  marginTop: 4,
                  fontSize: 11,
                  color: "var(--text-muted)",
                }}
              >
                <span>
                  R: <span style={{ color: "var(--accent-green)", fontFamily: "var(--font-mono), monospace" }}>
                    {formatNetworkSpeed(disk.read_bytes_per_sec)}
                  </span>
                </span>
                <span>
                  W: <span style={{ color: "var(--accent-blue)", fontFamily: "var(--font-mono), monospace" }}>
                    {formatNetworkSpeed(disk.write_bytes_per_sec)}
                  </span>
                </span>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
