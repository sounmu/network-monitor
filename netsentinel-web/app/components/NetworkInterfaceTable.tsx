"use client";

import { Network } from "lucide-react";
import { NetworkInterfaceRate } from "@/app/types/metrics";
import { formatNetworkSpeed } from "@/app/lib/formatters";
import { useI18n } from "@/app/i18n/I18nContext";

interface NetworkInterfaceTableProps {
  interfaces: NetworkInterfaceRate[];
}

export default function NetworkInterfaceTable({ interfaces }: NetworkInterfaceTableProps) {
  const { t } = useI18n();

  if (!interfaces || interfaces.length === 0) return null;

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
          <tr
            style={{
              borderBottom: "1px solid var(--border-color)",
              color: "var(--text-muted)",
              fontSize: 11,
              textTransform: "uppercase",
              letterSpacing: "0.5px",
            }}
          >
            <th style={{ textAlign: "left", padding: "6px 8px" }}>
              <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
                <Network size={12} />
                {t.networkInterfaces.interface}
              </div>
            </th>
            <th style={{ textAlign: "right", padding: "6px 8px" }}>
              {t.networkInterfaces.rx}
            </th>
            <th style={{ textAlign: "right", padding: "6px 8px" }}>
              {t.networkInterfaces.tx}
            </th>
          </tr>
        </thead>
        <tbody>
          {interfaces.map((iface) => (
            <tr
              key={iface.name}
              style={{
                borderBottom: "1px solid var(--border-color)",
              }}
            >
              <td
                style={{
                  padding: "8px",
                  fontFamily: "var(--font-mono), monospace",
                  fontWeight: 600,
                  color: "var(--text-primary)",
                }}
              >
                {iface.name}
              </td>
              <td
                style={{
                  padding: "8px",
                  textAlign: "right",
                  fontFamily: "var(--font-mono), monospace",
                  color: "var(--accent-green)",
                }}
              >
                {formatNetworkSpeed(iface.rx_bytes_per_sec)}
              </td>
              <td
                style={{
                  padding: "8px",
                  textAlign: "right",
                  fontFamily: "var(--font-mono), monospace",
                  color: "var(--accent-blue)",
                }}
              >
                {formatNetworkSpeed(iface.tx_bytes_per_sec)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
