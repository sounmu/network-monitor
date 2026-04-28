"use client";

import { PortStatus } from "@/app/types/metrics";
import { Network } from "lucide-react";
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
    <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
      {ports.map((p) => {
        const label = PORT_LABELS[p.port];
        return (
          <div
            key={p.port}
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: 6,
              padding: "6px 12px",
              borderRadius: 9999,
              background: p.is_open ? "var(--status-online-bg)" : "var(--status-offline-bg)",
              border: `1px solid ${p.is_open ? "var(--badge-online-border)" : "var(--badge-offline-border)"}`,
              fontSize: 12,
              fontWeight: 500,
            }}
          >
            <span
              style={{
                width: 6,
                height: 6,
                borderRadius: "50%",
                background: p.is_open ? "var(--accent-green)" : "var(--accent-red)",
                flexShrink: 0,
              }}
            />
            <span
              className="font-mono"
              style={{
                fontWeight: 600,
                color: "var(--text-primary)",
              }}
            >
              {p.port}
            </span>
            {label && (
              <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
                {label}
              </span>
            )}
          </div>
        );
      })}
    </div>
  );
}
