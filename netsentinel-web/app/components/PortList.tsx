"use client";

import { PortStatus } from "@/app/types/metrics";
import { CheckCircle2, XCircle, Network } from "lucide-react";
import { useI18n } from "@/app/i18n/I18nContext";

interface PortListProps {
  ports: PortStatus[];
}

const PORT_LABELS: Record<number, string> = {
  22: "SSH",
  80: "HTTP",
  443: "HTTPS",
  3000: "App",
  3306: "MySQL",
  5432: "PostgreSQL",
  6379: "Redis",
  8080: "HTTP Alt",
  8443: "HTTPS Alt",
  9100: "Node Exporter",
  27017: "MongoDB",
};

export default function PortList({ ports }: PortListProps) {
  const { t } = useI18n();
  if (ports.length === 0) {
    return (
      <div
        style={{
          textAlign: "center",
          padding: "24px 0",
          color: "var(--text-muted)",
          fontSize: 13,
        }}
      >
        <Network size={28} style={{ margin: "0 auto 8px", opacity: 0.4 }} />
        <div>{t.portList.noData}</div>
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      {ports.map((p) => {
        const label = PORT_LABELS[p.port] || "Unknown";
        return (
          <div
            key={p.port}
            style={{
              display: "flex",
              alignItems: "center",
              padding: "10px 14px",
              borderRadius: 8,
              background: p.is_open ? "var(--status-online-bg)" : "var(--status-offline-bg)",
              border: `1px solid ${p.is_open ? "var(--badge-online-border)" : "var(--badge-offline-border)"}`,
              transition: "all 0.2s",
            }}
          >
            {p.is_open ? (
              <CheckCircle2 size={18} color="var(--accent-green)" style={{ flexShrink: 0 }} />
            ) : (
              <XCircle size={18} color="var(--accent-red)" style={{ flexShrink: 0 }} />
            )}
            <div style={{ marginLeft: 12, flex: 1 }}>
              <span
                className="font-mono"
                style={{
                  fontSize: 14,
                  fontWeight: 700,
                  color: "var(--text-primary)",
                }}
              >
                :{p.port}
              </span>
              <span
                style={{
                  marginLeft: 10,
                  fontSize: 12,
                  color: "var(--text-muted)",
                }}
              >
                {label}
              </span>
            </div>
            <span
              style={{
                fontSize: 11,
                fontWeight: 700,
                textTransform: "uppercase",
                letterSpacing: "0.5px",
                color: p.is_open ? "var(--badge-online-text)" : "var(--badge-offline-text)",
              }}
            >
              {p.is_open ? t.portList.open : t.portList.closed}
            </span>
          </div>
        );
      })}
    </div>
  );
}
