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
    <div>
      <div
        style={{
          display: "grid",
          gridTemplateColumns: `repeat(auto-fill, minmax(${cores.length > 16 ? 42 : 56}px, 1fr))`,
          gap: 4,
        }}
      >
        {cores.map((pct, idx) => {
          const clamped = Math.min(Math.max(pct, 0), 100);
          const color = getCoreColor(clamped);
          return (
            <div
              key={idx}
              title={`${t.cpuCores.core} ${idx}: ${clamped.toFixed(1)}%`}
              style={{
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                gap: 2,
              }}
            >
              <div
                style={{
                  width: "100%",
                  height: 20,
                  background: "var(--bg-card-hover)",
                  borderRadius: 4,
                  overflow: "hidden",
                  position: "relative",
                }}
              >
                <div
                  style={{
                    position: "absolute",
                    bottom: 0,
                    left: 0,
                    width: "100%",
                    height: `${clamped}%`,
                    background: color,
                    borderRadius: 4,
                    transition: "height 0.5s ease, background 0.3s ease",
                  }}
                />
              </div>
              <span
                style={{
                  fontSize: 9,
                  color: "var(--text-muted)",
                  fontFamily: "var(--font-mono), monospace",
                }}
              >
                {idx}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
