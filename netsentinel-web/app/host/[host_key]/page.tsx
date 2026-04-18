"use client";

import { use } from "react";
import dynamic from "next/dynamic";
import { notFound } from "next/navigation";
import { useSSE } from "@/app/lib/sse-context";
const TimeSeriesChart = dynamic(
  () => import("@/app/components/TimeSeriesChart"),
  { ssr: false, loading: () => <div className="skeleton" style={{ height: 300 }} /> },
);
import PortList from "@/app/components/PortList";
import GpuCard from "@/app/components/GpuCard";
import {
  getHostStatus,
  STATUS_DOT_CLASS,
} from "@/app/lib/status";
import {
  Activity,
  ArrowLeft,
  Wifi,
  Network,
  Monitor,
  Clock,
  Globe,
  Server,
  Cpu,
  MemoryStick,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useI18n } from "@/app/i18n/I18nContext";

interface Props {
  params: Promise<{ host_key: string }>;
}

/** Format uptime from boot_time (Unix timestamp seconds).
 *  <24h → "Xh Xm", ≥24h → "Xd Xh" */
function formatUptime(bootTime: number): string {
  const now = Math.floor(Date.now() / 1000);
  const secs = Math.max(now - bootTime, 0);
  const minutes = Math.floor(secs / 60) % 60;
  const hours = Math.floor(secs / 3600) % 24;
  const days = Math.floor(secs / 86400);
  if (days > 0) return `${days}d ${hours}h`;
  return `${hours}h ${minutes}m`;
}

function SectionCard({
  title,
  icon,
  children,
  style,
}: {
  title: string;
  icon: React.ReactNode;
  children: React.ReactNode;
  style?: React.CSSProperties;
}) {
  return (
    <div className="glass-card" style={{ padding: "20px 22px", ...style }}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          marginBottom: 16,
          paddingBottom: 12,
          borderBottom: "1px solid var(--border-subtle)",
        }}
      >
        <span style={{ color: "var(--text-muted)", display: "flex" }}>{icon}</span>
        <h2 style={{ fontSize: 14, fontWeight: 600, color: "var(--text-primary)" }}>
          {title}
        </h2>
      </div>
      {children}
    </div>
  );
}

export default function HostPage({ params }: Props) {
  const { host_key } = use(params);
  const decodedHostKey = decodeURIComponent(host_key);
  const router = useRouter();

  const { metricsMap, statusMap, isConnected } = useSSE();
  const { t } = useI18n();

  const liveMetrics = metricsMap[decodedHostKey] ?? null;
  const statusData = statusMap[decodedHostKey] ?? null;

  const displayName = liveMetrics?.display_name ?? statusData?.display_name ?? decodedHostKey;
  const hasData = liveMetrics !== null || statusData !== null;

  const ports = statusData?.ports ?? [];
  const gpus = statusData?.gpus ?? [];
  const latestTimestamp = liveMetrics?.timestamp ?? statusData?.last_seen ?? null;

  const isOnline = liveMetrics?.is_online ?? statusData?.is_online;
  const hostStatus = latestTimestamp
    ? getHostStatus(latestTimestamp, isOnline, statusData?.scrape_interval_secs)
    : "pending";

  if (isConnected && !hasData && !(decodedHostKey in statusMap)) {
    notFound();
  }

  return (
    <div className="fade-in">
      {/* Info bar */}
      <div className="glass-card" style={{ marginBottom: 16 }}>
        <div
          style={{
            padding: "16px 20px 0",
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            gap: 12,
            flexWrap: "wrap",
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
            <button
              onClick={() => router.push("/")}
              style={{
                background: "none",
                border: "none",
                cursor: "pointer",
                color: "var(--text-muted)",
                display: "flex",
                alignItems: "center",
                padding: 0,
              }}
              aria-label={t.host.backToOverview}
            >
              <ArrowLeft size={18} />
            </button>
            <h1
              style={{
                fontSize: 20,
                fontWeight: 700,
                color: "var(--text-primary)",
                letterSpacing: "-0.3px",
              }}
            >
              {displayName}
            </h1>
            <span
              className={STATUS_DOT_CLASS[hostStatus]}
              style={{ width: 10, height: 10 }}
            />
          </div>
        </div>

        <div className="info-bar">
          {/* System info (from /system-info endpoint) */}
          {statusData?.ip_address && (
            <>
              <div className="info-bar-item">
                <Globe size={14} color="var(--text-muted)" />
                <span style={{ fontFamily: "var(--font-mono), monospace", fontSize: 12 }}>
                  {statusData.ip_address}
                </span>
              </div>
              <div className="info-bar-separator" />
            </>
          )}
          {statusData?.boot_time && (
            <>
              <div className="info-bar-item">
                <Clock size={14} color="var(--text-muted)" />
                <span style={{ fontSize: 12 }}>
                  {t.host.uptime}: {formatUptime(statusData.boot_time)}
                </span>
              </div>
              <div className="info-bar-separator" />
            </>
          )}
          {statusData?.os_info && (
            <>
              <div className="info-bar-item">
                <Monitor size={14} color="var(--text-muted)" />
                <span style={{ fontSize: 12 }}>{statusData.os_info}</span>
              </div>
              <div className="info-bar-separator" />
            </>
          )}
          {statusData?.cpu_model && (
            <>
              <div className="info-bar-item">
                <Cpu size={14} color="var(--text-muted)" />
                <span style={{ fontSize: 12 }}>{statusData.cpu_model}</span>
              </div>
              <div className="info-bar-separator" />
            </>
          )}
          {statusData?.memory_total_mb != null && (
            <div className="info-bar-item">
              <MemoryStick size={14} color="var(--text-muted)" />
              <span style={{ fontSize: 12, fontFamily: "var(--font-mono), monospace" }}>
                {statusData.memory_total_mb >= 1024
                  ? `${(statusData.memory_total_mb / 1024).toFixed(1)} GB`
                  : `${statusData.memory_total_mb} MB`}
              </span>
            </div>
          )}

          {/* Fallback: show basic info when system info is not yet available */}
          {!statusData?.ip_address && !statusData?.os_info && (
            <>
              {displayName !== decodedHostKey && (
                <>
                  <div className="info-bar-item">
                    <Globe size={14} color="var(--text-muted)" />
                    <span style={{ fontFamily: "var(--font-mono), monospace", fontSize: 12 }}>
                      {decodedHostKey}
                    </span>
                  </div>
                  <div className="info-bar-separator" />
                </>
              )}
              <div className="info-bar-item">
                <Server size={14} color="var(--text-muted)" />
                <span>{displayName}</span>
              </div>
              {latestTimestamp && (
                <>
                  <div className="info-bar-separator" />
                  <div className="info-bar-item">
                    <Clock size={14} color="var(--text-muted)" />
                    <span style={{ fontSize: 12, fontFamily: "var(--font-mono), monospace" }}>
                      {new Date(latestTimestamp).toLocaleString()}
                    </span>
                  </div>
                </>
              )}
              {liveMetrics && (
                <>
                  <div className="info-bar-separator" />
                  <div className="info-bar-item">
                    <Cpu size={14} color="var(--text-muted)" />
                    <span style={{ fontFamily: "var(--font-mono), monospace", fontSize: 12, fontWeight: 600 }}>
                      CPU {liveMetrics.cpu_usage_percent.toFixed(1)}%
                    </span>
                  </div>
                  <div className="info-bar-separator" />
                  <div className="info-bar-item">
                    <Activity size={14} color="var(--text-muted)" />
                    <span style={{ fontFamily: "var(--font-mono), monospace", fontSize: 12, fontWeight: 600 }}>
                      RAM {liveMetrics.memory_usage_percent.toFixed(1)}%
                    </span>
                  </div>
                </>
              )}
            </>
          )}
        </div>
      </div>

      {/* Loading */}
      {!isConnected && !hasData && (
        <div style={{ display: "grid", gap: 16 }}>
          {[220, 400, 200].map((h, i) => (
            <div key={i} className="skeleton" style={{ height: h }} />
          ))}
        </div>
      )}

      {/* No data */}
      {isConnected && !hasData && (
        <div
          className="glass-card"
          style={{ padding: "48px 24px", textAlign: "center", color: "var(--text-muted)" }}
        >
          <Wifi size={36} style={{ margin: "0 auto 12px", opacity: 0.3 }} />
          <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 6 }}>{t.host.noMetrics}</div>
          <div style={{ fontSize: 13 }}>{t.host.noMetricsHint}</div>
        </div>
      )}

      {/* All charts + remaining sections */}
      {hasData && (
        <>
          {/* Main time-series charts (CPU, RAM, Network, Temp, Cores, Disk, Processes) */}
          <div style={{ marginBottom: 16 }}>
            <TimeSeriesChart hostKey={decodedHostKey} />
          </div>

          {/* Ports + GPU — small info cards */}
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fit, minmax(340px, 1fr))",
              gap: 16,
              marginBottom: 16,
            }}
          >
            {ports.length > 0 && (
              <SectionCard
                title={`${t.host.portStatus} (${ports.length})`}
                icon={<Network size={15} />}
              >
                <PortList ports={ports} />
              </SectionCard>
            )}
            {gpus.length > 0 && (
              <SectionCard
                title={`${t.host.gpu} (${gpus.length})`}
                icon={<Monitor size={15} />}
              >
                <GpuCard gpus={gpus} />
              </SectionCard>
            )}
          </div>

        </>
      )}
    </div>
  );
}
