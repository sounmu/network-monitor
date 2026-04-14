"use client";

import { useSSE } from "@/app/lib/sse-context";
import {
  getHostStatus,
  STATUS_COLORS,
  STATUS_LABELS,
  STATUS_BADGE_CLASS,
  STATUS_DOT_CLASS,
  STATUS_SUB_LABELS,
} from "@/app/lib/status";
import { Server, Wifi, WifiOff, AlertTriangle, Activity, Clock, ArrowRight } from "lucide-react";
import DashboardWidgets from "@/app/components/DashboardWidgets";
import { useRouter } from "next/navigation";
import React, { useMemo } from "react";
import { useI18n } from "@/app/i18n/I18nContext";

export default function HomePage() {
  const router = useRouter();
  const { metricsMap, statusMap, isConnected } = useSSE();
  const { t } = useI18n();

  // Build host list from statusMap (memoized to avoid O(n) recalc on unrelated re-renders)
  // statusMap is pre-populated on server start, so offline hosts are always included
  const { hosts, onlineCount, pendingCount, offlineCount } = useMemo(() => {
    const list = Object.values(statusMap).map((status) => {
      const metrics = metricsMap[status.host_key];
      return {
        host_key: status.host_key,
        display_name: metrics?.display_name ?? status.display_name,
        is_online: metrics?.is_online ?? status.is_online ?? false,
        last_seen: metrics?.timestamp ?? status.last_seen ?? null,
      };
    });
    let online = 0;
    let pending = 0;
    let offline = 0;
    for (const h of list) {
      const s = getHostStatus(h.last_seen, h.is_online);
      if (s === "online") online++;
      else if (s === "pending") pending++;
      else offline++;
    }
    return { hosts: list, onlineCount: online, pendingCount: pending, offlineCount: offline };
  }, [statusMap, metricsMap]);

  const isLoading = !isConnected && hosts.length === 0;

  return (
    <div className="page-content fade-in">
      {/* Header */}
      <div style={{ marginBottom: 28 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 6 }}>
          <Activity size={20} color="var(--accent-blue)" />
          <h1
            style={{
              fontSize: 22,
              fontWeight: 800,
              color: "var(--text-primary)",
              letterSpacing: "-0.3px",
            }}
          >
            {t.overview.title}
          </h1>
        </div>
        <p style={{ color: "var(--text-muted)", fontSize: 13 }}>
          {t.overview.description}
          <span
            role="status"
            aria-live="polite"
            style={{
              marginLeft: 8,
              fontSize: 12,
              color: isConnected ? "var(--accent-green)" : "var(--accent-yellow)",
            }}
          >
            {isConnected ? t.overview.live : t.overview.connecting}
          </span>
        </p>
      </div>

      {/* Summary cards */}
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fit, minmax(160px, 1fr))",
          gap: 14,
          marginBottom: 28,
        }}
      >
        <StatCard
          title={t.overview.total}
          value={isLoading ? "—" : String(hosts.length)}
          icon={<Server size={18} />}
          color="var(--accent-blue)"
          bg="var(--preset-hover-bg)"
          borderColor="var(--preset-hover-border)"
        />
        <StatCard
          title={t.overview.online}
          value={isLoading ? "—" : String(onlineCount)}
          icon={<Wifi size={18} />}
          color="var(--accent-green)"
          bg="var(--status-online-bg)"
          borderColor="var(--badge-online-border)"
        />
        <StatCard
          title={t.overview.pending}
          value={isLoading ? "—" : String(pendingCount)}
          icon={<AlertTriangle size={18} />}
          color={pendingCount > 0 ? "var(--accent-yellow)" : "var(--text-muted)"}
          bg={pendingCount > 0 ? "var(--badge-pending-bg)" : "var(--bg-primary)"}
          borderColor={pendingCount > 0 ? "var(--badge-pending-border)" : "var(--border-subtle)"}
        />
        <StatCard
          title={t.overview.offline}
          value={isLoading ? "—" : String(offlineCount)}
          icon={<WifiOff size={18} />}
          color={offlineCount > 0 ? "var(--accent-red)" : "var(--text-muted)"}
          bg={offlineCount > 0 ? "var(--status-offline-bg)" : "var(--bg-primary)"}
          borderColor={offlineCount > 0 ? "var(--badge-offline-border)" : "var(--border-subtle)"}
        />
        <StatCard
          title={t.overview.uptime}
          value={
            isLoading || hosts.length === 0
              ? "—"
              : `${((onlineCount / hosts.length) * 100).toFixed(1)}%`
          }
          icon={<Activity size={18} />}
          color="var(--accent-purple)"
          bg="var(--bg-card-hover)"
          borderColor="var(--border-subtle)"
        />
      </div>

      {/* Dashboard widgets */}
      <DashboardWidgets />

      {/* Server list */}
      <div>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            marginBottom: 14,
          }}
        >
          <h2 style={{ fontSize: 15, fontWeight: 700, color: "var(--text-primary)" }}>
            {t.overview.serverList}
          </h2>
          <span
            style={{
              fontSize: 11,
              color: "var(--text-muted)",
              background: "var(--bg-card-hover)",
              padding: "2px 8px",
              borderRadius: 6,
            }}
          >
            {hosts.length} {t.common.servers}
          </span>
        </div>

        {isLoading && (
          <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
            {[1, 2, 3].map((i) => (
              <div key={i} className="skeleton" style={{ height: 76 }} />
            ))}
          </div>
        )}

        {!isLoading && hosts.length === 0 && (
          <div
            className="glass-card"
            style={{
              padding: "48px 24px",
              textAlign: "center",
              color: "var(--text-muted)",
            }}
          >
            <Activity size={40} style={{ margin: "0 auto 12px", opacity: 0.3 }} />
            <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 6 }}>
              {t.overview.noAgents}
            </div>
            <div style={{ fontSize: 13 }}>
              {t.overview.noAgentsHint}
            </div>
          </div>
        )}

        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          {hosts.map((host) => {
            const status = getHostStatus(host.last_seen, host.is_online);
            const colors = STATUS_COLORS[status];
            return (
              <button
                key={host.host_key}
                onClick={() => router.push(`/host/${encodeURIComponent(host.host_key)}`)}
                className="glass-card"
                style={{
                  padding: "16px 20px",
                  display: "flex",
                  alignItems: "center",
                  gap: 16,
                  cursor: "pointer",
                  border: "1px solid var(--border-subtle)",
                  background: "var(--bg-card)",
                  width: "100%",
                  textAlign: "left",
                }}
              >
                <div
                  style={{
                    width: 42,
                    height: 42,
                    borderRadius: 8,
                    background: colors.bg,
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                    flexShrink: 0,
                  }}
                >
                  <Server size={20} color={colors.accent} />
                </div>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div
                    style={{
                      fontSize: 15,
                      fontWeight: 700,
                      color: "var(--text-primary)",
                      marginBottom: 3,
                    }}
                  >
                    {host.display_name}
                  </div>
                  {/* Show host_key (IP:port) as secondary info when different from display_name */}
                  {host.display_name !== host.host_key && (
                    <div
                      style={{
                        fontSize: 11,
                        color: "var(--text-muted)",
                        fontFamily: "var(--font-mono), monospace",
                        opacity: 0.7,
                      }}
                    >
                      {host.host_key}
                    </div>
                  )}
                  <div
                    style={{
                      fontSize: 12,
                      color: "var(--text-muted)",
                      fontFamily: "var(--font-mono), monospace",
                    }}
                  >
                    {STATUS_SUB_LABELS[status]}
                  </div>
                </div>
                {host.last_seen && (
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: 5,
                      fontSize: 12,
                      color: "var(--text-muted)",
                      flexShrink: 0,
                    }}
                  >
                    <Clock size={12} />
                    {new Date(host.last_seen).toLocaleTimeString()}
                  </div>
                )}
                <div style={{ marginLeft: 8, flexShrink: 0 }}>
                  <span className={STATUS_BADGE_CLASS[status]}>
                    <span className={STATUS_DOT_CLASS[status]} />
                    {STATUS_LABELS[status]}
                  </span>
                </div>
                <ArrowRight size={16} color="var(--text-muted)" style={{ flexShrink: 0 }} />
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}

const StatCard = React.memo(function StatCard({
  title,
  value,
  icon,
  color,
  bg,
  borderColor,
}: {
  title: string;
  value: string;
  icon: React.ReactNode;
  color: string;
  bg: string;
  borderColor: string;
}) {
  return (
    <div
      style={{
        background: bg,
        border: `1px solid ${borderColor}`,
        borderRadius: 8,
        padding: "18px",
        position: "relative",
        overflow: "hidden",
      }}
    >
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "flex-start",
          marginBottom: 10,
        }}
      >
        <div style={{ color, opacity: 0.8 }}>{icon}</div>
      </div>
      <div
        className="font-mono"
        style={{
          fontSize: 32,
          fontWeight: 800,
          color,
          lineHeight: 1,
          marginBottom: 6,
          fontFamily: "var(--font-mono), monospace",
        }}
      >
        {value}
      </div>
      <div style={{ fontSize: 12, color: "var(--text-muted)", fontWeight: 500 }}>
        {title}
      </div>
    </div>
  );
});
