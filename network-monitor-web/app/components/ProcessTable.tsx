"use client";

import { useMemo } from "react";
import { Cpu } from "lucide-react";
import { ProcessInfo } from "@/app/types/metrics";
import { useI18n } from "@/app/i18n/I18nContext";

interface ProcessTableProps {
  processes: ProcessInfo[];
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
          fontSize: 13,
        }}
      >
        <thead>
          <tr>
            {[t.process.name, "PID", "CPU %", t.process.memoryMb].map((header) => (
              <th
                key={header}
                style={{
                  textAlign: "left",
                  padding: "8px 12px",
                  fontSize: 11,
                  fontWeight: 600,
                  color: "var(--text-muted)",
                  textTransform: "uppercase",
                  letterSpacing: "0.5px",
                  borderBottom: "1px solid var(--bg-card-hover)",
                }}
              >
                {header}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {sorted.map((proc, idx) => (
            <tr
              key={`${proc.pid}-${idx}`}
              className="process-row"
            >
              <td
                style={{
                  padding: "8px 12px",
                  color: "var(--text-primary)",
                  fontWeight: 600,
                  maxWidth: 200,
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
              >
                {proc.name}
              </td>
              <td
                className="font-mono"
                style={{
                  padding: "8px 12px",
                  color: "var(--text-muted)",
                }}
              >
                {proc.pid}
              </td>
              <td
                className="font-mono"
                style={{
                  padding: "8px 12px",
                  fontWeight: 700,
                  color: "var(--text-primary)",
                }}
              >
                {proc.cpu_usage.toFixed(1)}
              </td>
              <td
                className="font-mono"
                style={{
                  padding: "8px 12px",
                  color: "var(--text-muted)",
                }}
              >
                {proc.memory_mb.toFixed(1)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
