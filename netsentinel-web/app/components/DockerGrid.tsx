"use client";

import { DockerContainer } from "@/app/types/metrics";
import { Box, Cpu } from "lucide-react";
import { useI18n } from "@/app/i18n/I18nContext";

interface DockerGridProps {
  containers: DockerContainer[];
}

export default function DockerGrid({ containers }: DockerGridProps) {
  const { t } = useI18n();
  if (containers.length === 0) {
    return (
      <div
        style={{
          textAlign: "center",
          padding: "24px 0",
          color: "var(--text-muted)",
          fontSize: 13,
        }}
      >
        <Box size={28} style={{ margin: "0 auto 8px", opacity: 0.4 }} />
        <div>{t.dockerGrid.noContainers}</div>
      </div>
    );
  }

  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "repeat(auto-fill, minmax(220px, 1fr))",
        gap: 10,
      }}
    >
      {containers.map((c, idx) => {
        const isRunning = c.state === "running";
        return (
          <div
            key={idx}
            style={{
              background: isRunning ? "var(--status-online-bg)" : "var(--status-offline-bg)",
              border: `1px solid ${isRunning ? "var(--badge-online-border)" : "var(--badge-offline-border)"}`,
              borderRadius: 8,
              padding: "12px 14px",
              transition: "all 0.2s",
            }}
          >
            <div
              style={{
                display: "flex",
                alignItems: "flex-start",
                gap: 10,
                marginBottom: 8,
              }}
            >
              <div
                style={{
                  width: 32,
                  height: 32,
                  borderRadius: 8,
                  background: isRunning ? "var(--status-online-bg-light)" : "var(--status-offline-bg-light)",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  flexShrink: 0,
                }}
              >
                <Cpu
                  size={16}
                  color={isRunning ? "var(--accent-green)" : "var(--accent-red)"}
                />
              </div>
              <div style={{ minWidth: 0, flex: 1 }}>
                <div
                  style={{
                    fontSize: 13,
                    fontWeight: 700,
                    color: "var(--text-primary)",
                    whiteSpace: "nowrap",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                  }}
                  title={c.container_name}
                >
                  {c.container_name}
                </div>
                <div
                  style={{
                    fontSize: 11,
                    color: "var(--text-muted)",
                    whiteSpace: "nowrap",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    fontFamily: "monospace",
                  }}
                  title={c.image}
                >
                  {c.image}
                </div>
              </div>
            </div>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: 4,
                  padding: "2px 8px",
                  borderRadius: 6,
                  fontSize: 10,
                  fontWeight: 700,
                  letterSpacing: "0.5px",
                  textTransform: "uppercase",
                  background: isRunning ? "var(--status-online-bg-light)" : "var(--status-offline-bg-light)",
                  color: isRunning ? "var(--badge-online-text)" : "var(--badge-offline-text)",
                }}
              >
                <span
                  className={`pulse-dot ${isRunning ? "green" : "red"}`}
                  style={{ width: 5, height: 5 }}
                />
                {c.state}
              </span>
              <span
                style={{
                  fontSize: 11,
                  color: "var(--text-muted)",
                  flex: 1,
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
                title={c.status}
              >
                {c.status}
              </span>
            </div>
          </div>
        );
      })}
    </div>
  );
}
