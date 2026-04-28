"use client";

import { useI18n } from "@/app/i18n/I18nContext";

interface CpuCoreGridProps {
  cores: number[];
}

function getCoreColor(pct: number): string {
  if (pct < 40) return "var(--accent-green)";
  if (pct < 70) return "var(--accent-yellow)";
  return "var(--accent-red)";
}

export default function CpuCoreGrid({ cores }: CpuCoreGridProps) {
  const { t } = useI18n();

  if (!cores || cores.length === 0) return null;

  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: cores.length > 8 ? "1fr 1fr" : "1fr",
        gap: 4,
      }}
    >
      {cores.map((pct, idx) => {
        const clamped = Math.min(Math.max(pct, 0), 100);
        const color = getCoreColor(clamped);
        return (
          <div
            key={idx}
            role="meter"
            aria-label={`${t.cpuCores.core} ${idx}`}
            aria-valuenow={clamped}
            aria-valuemin={0}
            aria-valuemax={100}
            title={`${t.cpuCores.core} ${idx}: ${clamped.toFixed(1)}%`}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              padding: "3px 0",
            }}
          >
            <span
              style={{
                fontSize: 10,
                color: "var(--text-muted)",
                fontFamily: "var(--font-mono), monospace",
                minWidth: 18,
                textAlign: "right",
                flexShrink: 0,
              }}
            >
              {idx}
            </span>
            <span
              style={{
                flex: 1,
                height: 10,
                background: "var(--bg-muted)",
                borderRadius: 3,
                overflow: "hidden",
              }}
            >
              <span
                style={{
                  display: "block",
                  height: "100%",
                  width: `${clamped}%`,
                  background: color,
                  borderRadius: 3,
                  transition: "width 0.5s ease, background 0.3s ease",
                }}
              />
            </span>
            <span
              style={{
                fontSize: 11,
                fontWeight: 600,
                fontFamily: "var(--font-mono), monospace",
                color: color,
                minWidth: 38,
                textAlign: "right",
                flexShrink: 0,
              }}
            >
              {clamped.toFixed(0)}%
            </span>
          </div>
        );
      })}
    </div>
  );
}
