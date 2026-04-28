"use client";

import { useMemo } from "react";
import { Cpu } from "lucide-react";
import { ProcessInfo } from "@/app/types/metrics";
import { useI18n } from "@/app/i18n/I18nContext";

interface ProcessTableProps {
  processes: ProcessInfo[];
}

function cpuBarColor(pct: number): string {
  if (pct < 40) return "var(--accent-green)";
  if (pct < 70) return "var(--accent-yellow)";
  return "var(--accent-red)";
}

export default function ProcessTable({ processes }: ProcessTableProps) {
  const { t } = useI18n();

  const sorted = useMemo(
    () => processes ? [...processes].sort((a, b) => b.cpu_usage - a.cpu_usage) : [],
    [processes]
  );

  if (!processes || processes.length === 0) {
    return (
      <div style={{ textAlign: "center", padding: "24px 0", color: "var(--text-muted)" }}>
        <Cpu size={28} style={{ margin: "0 auto 8px", opacity: 0.4 }} />
        <div style={{ fontSize: 13 }}>{t.process.noData}</div>
      </div>
    );
  }

  return (
    <div style={{ overflowX: "auto" }}>
      <table
        style={{
          width: "100%",
          borderCollapse: "collapse",
          fontSize: 12,
        }}
      >
        <thead>
          <tr>
            {[t.process.name, "CPU %", t.process.memoryMb].map((header) => (
              <th
                key={header}
                style={{
                  textAlign: "left",
                  padding: "6px 10px",
                  fontSize: 11,
                  fontWeight: 600,
                  color: "var(--text-muted)",
                  textTransform: "uppercase",
                  letterSpacing: "0.3px",
                  borderBottom: "1px solid var(--border-subtle)",
                }}
              >
                {header}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {sorted.map((proc, idx) => {
            const cpuPct = Math.min(proc.cpu_usage, 100);
            return (
              <tr
                key={`${proc.pid}-${idx}`}
                className="process-row"
              >
                <td
                  style={{
                    padding: "6px 10px",
                    color: "var(--text-primary)",
                    fontWeight: 500,
                    maxWidth: 180,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                  title={`${proc.name} (PID: ${proc.pid})`}
                >
                  {proc.name}
                </td>
                <td style={{ padding: "6px 10px", width: "40%" }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                    <span
                      className="font-mono"
                      style={{
                        fontSize: 11,
                        fontWeight: 600,
                        color: "var(--text-primary)",
                        minWidth: 36,
                        textAlign: "right",
                      }}
                    >
                      {proc.cpu_usage.toFixed(1)}
                    </span>
                    <span
                      style={{
                        flex: 1,
                        height: "0.8em",
                        background: "var(--bg-muted)",
                        borderRadius: 3,
                        overflow: "hidden",
                      }}
                    >
                      <span
                        style={{
                          display: "block",
                          height: "100%",
                          width: `${cpuPct}%`,
                          background: cpuBarColor(cpuPct),
                          borderRadius: 3,
                          transition: "width 0.3s ease",
                        }}
                      />
                    </span>
                  </div>
                </td>
                <td
                  className="font-mono"
                  style={{
                    padding: "6px 10px",
                    color: "var(--text-muted)",
                    fontSize: 11,
                  }}
                >
                  {proc.memory_mb.toFixed(0)}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
