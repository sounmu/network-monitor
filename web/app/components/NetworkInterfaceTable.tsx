"use client";

import { useState, useMemo } from "react";
import { NetworkInterfaceRate } from "@/app/types/metrics";
import { formatNetworkSpeed } from "@/app/lib/formatters";
import { useI18n } from "@/app/i18n/I18nContext";

interface NetworkInterfaceTableProps {
  interfaces: NetworkInterfaceRate[];
}

type SortMode = "rx" | "tx";
const TOP_N = 5;

export default function NetworkInterfaceTable({ interfaces }: NetworkInterfaceTableProps) {
  const { t } = useI18n();
  const [mode, setMode] = useState<SortMode>("rx");

  const top5 = useMemo(() => {
    if (!interfaces || interfaces.length === 0) return [];
    const sorted = [...interfaces].sort((a, b) =>
      mode === "rx"
        ? b.rx_bytes_per_sec - a.rx_bytes_per_sec
        : b.tx_bytes_per_sec - a.tx_bytes_per_sec
    );
    return sorted.slice(0, TOP_N);
  }, [interfaces, mode]);

  // Find max value for relative bar sizing — must run before any early return
  // to keep Hook call order stable across renders (react-hooks/rules-of-hooks).
  const maxVal = useMemo(() => {
    let m = 0;
    for (const iface of top5) {
      const v = mode === "rx" ? iface.rx_bytes_per_sec : iface.tx_bytes_per_sec;
      if (v > m) m = v;
    }
    return m || 1;
  }, [top5, mode]);

  if (!interfaces || interfaces.length === 0) return null;

  const accentColor = mode === "rx" ? "var(--accent-green)" : "var(--accent-blue)";

  return (
    <div>
      {/* RX / TX toggle */}
      <div
        style={{
          display: "inline-flex",
          borderRadius: 8,
          border: "1px solid var(--border-subtle)",
          overflow: "hidden",
          marginBottom: 12,
          fontSize: 12,
          fontWeight: 600,
        }}
      >
        <button
          onClick={() => setMode("rx")}
          style={{
            padding: "5px 16px",
            border: "none",
            cursor: "pointer",
            background: mode === "rx" ? "var(--accent-green)" : "transparent",
            color: mode === "rx" ? "var(--text-on-accent, #fff)" : "var(--text-muted)",
            transition: "all 0.15s ease",
          }}
        >
          {t.networkInterfaces.rx}
        </button>
        <button
          onClick={() => setMode("tx")}
          style={{
            padding: "5px 16px",
            border: "none",
            borderLeft: "1px solid var(--border-subtle)",
            cursor: "pointer",
            background: mode === "tx" ? "var(--accent-blue)" : "transparent",
            color: mode === "tx" ? "var(--text-on-accent, #fff)" : "var(--text-muted)",
            transition: "all 0.15s ease",
          }}
        >
          {t.networkInterfaces.tx}
        </button>
      </div>

      {/* Bar list */}
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        {top5.map((iface) => {
          const val = mode === "rx" ? iface.rx_bytes_per_sec : iface.tx_bytes_per_sec;
          const pct = maxVal > 0 ? (val / maxVal) * 100 : 0;
          return (
            <div
              key={iface.name}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
              }}
            >
              <span
                style={{
                  fontSize: 11,
                  fontFamily: "var(--font-mono), monospace",
                  fontWeight: 600,
                  color: "var(--text-primary)",
                  minWidth: 60,
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                  flexShrink: 0,
                }}
                title={iface.name}
              >
                {iface.name}
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
                    width: `${pct}%`,
                    background: accentColor,
                    borderRadius: 3,
                    transition: "width 0.5s ease",
                    minWidth: val > 0 ? 3 : 0,
                  }}
                />
              </span>
              <span
                style={{
                  fontSize: 11,
                  fontFamily: "var(--font-mono), monospace",
                  fontWeight: 500,
                  color: accentColor,
                  minWidth: 72,
                  textAlign: "right",
                  flexShrink: 0,
                }}
              >
                {formatNetworkSpeed(val)}
              </span>
            </div>
          );
        })}
      </div>

      {interfaces.length > TOP_N && (
        <div
          style={{
            fontSize: 11,
            color: "var(--text-muted)",
            marginTop: 8,
            textAlign: "center",
          }}
        >
          +{interfaces.length - TOP_N} more
        </div>
      )}
    </div>
  );
}
