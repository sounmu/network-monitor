"use client";

import { use } from "react";
import dynamic from "next/dynamic";
import { useSSE } from "@/app/lib/sse-context";
const TimeSeriesChart = dynamic(
  () => import("@/app/components/TimeSeriesChart"),
  { ssr: false, loading: () => <div className="skeleton" style={{ height: 300 }} /> },
);
import LoadGauge from "@/app/components/LoadGauge";
import DockerGrid from "@/app/components/DockerGrid";
import PortList from "@/app/components/PortList";
import DiskUsageBar from "@/app/components/DiskUsageBar";
import ProcessTable from "@/app/components/ProcessTable";
import TemperatureDisplay from "@/app/components/TemperatureDisplay";
import GpuCard from "@/app/components/GpuCard";
import {
  getHostStatus,
  STATUS_BADGE_CLASS,
  STATUS_DOT_CLASS,
  STATUS_LABELS,
} from "@/app/lib/status";
import {
  Activity,
  ArrowLeft,
  Wifi,
  Network,
  Layers,
  HardDrive,
  Cpu,
  Thermometer,
  Monitor,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { LoadAverage } from "@/app/types/metrics";
import { useI18n } from "@/app/i18n/I18nContext";

interface Props {
  params: Promise<{ host_key: string }>;
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
        <span style={{ color: "var(--accent-blue)", display: "flex" }}>{icon}</span>
        <h2 style={{ fontSize: 14, fontWeight: 700, color: "var(--text-primary)" }}>
          {title}
        </h2>
      </div>
      {children}
    </div>
  );
}

export default function HostPage({ params }: Props) {
  const { host_key } = use(params);
  // host_key is the target URL (e.g. "192.168.1.10:9101") — needs URL decoding
  const decodedHostKey = decodeURIComponent(host_key);
  const router = useRouter();

  const { metricsMap, statusMap, isConnected } = useSSE();
  const { t } = useI18n();

  // Look up latest metrics and status data for this host by host_key from SSE
  const liveMetrics = metricsMap[decodedHostKey] ?? null;
  const statusData = statusMap[decodedHostKey] ?? null;

  // Display name: prefer display_name, fall back to raw host_key
  const displayName = liveMetrics?.display_name ?? statusData?.display_name ?? decodedHostKey;

  // Connected but no data received for this host yet
  const hasData = liveMetrics !== null || statusData !== null;

  const loadAvg: LoadAverage | null = liveMetrics
    ? {
        one_min: liveMetrics.load_1min,
        five_min: liveMetrics.load_5min,
        fifteen_min: liveMetrics.load_15min,
      }
    : null;

  const docker = statusData?.docker_containers ?? [];
  const ports = statusData?.ports ?? [];
  const disks = statusData?.disks ?? [];
  const processes = statusData?.processes ?? [];
  const temperatures = statusData?.temperatures ?? [];
  const gpus = statusData?.gpus ?? [];
  const latestTimestamp = liveMetrics?.timestamp ?? statusData?.last_seen ?? null;

  return (
    <div className="page-content fade-in">
      {/* Header */}
      <div
        style={{
          display: "flex",
          alignItems: "flex-start",
          justifyContent: "space-between",
          marginBottom: 24,
          flexWrap: "wrap",
          gap: 12,
        }}
      >
        <div>
          <button
            onClick={() => router.push("/")}
            style={{
              background: "none",
              border: "none",
              cursor: "pointer",
              color: "var(--text-muted)",
              fontSize: 12,
              display: "flex",
              alignItems: "center",
              gap: 4,
              marginBottom: 8,
              padding: 0,
            }}
          >
            <ArrowLeft size={12} /> {t.host.backToOverview}
          </button>
          <div style={{ display: "flex", alignItems: "center", gap: 12, flexWrap: "wrap" }}>
            <h1
              style={{
                fontSize: 24,
                fontWeight: 800,
                color: "var(--text-primary)",
                letterSpacing: "-0.5px",
              }}
            >
              {displayName}
            </h1>
            {/* Show host_key (IP:port) as secondary info when different from display_name */}
            {displayName !== decodedHostKey && (
              <div
                style={{
                  fontSize: 12,
                  color: "var(--text-muted)",
                  fontFamily: "var(--font-mono), monospace",
                }}
              >
                {decodedHostKey}
              </div>
            )}
            {latestTimestamp && (() => {
              const isOnline = liveMetrics?.is_online ?? statusData?.is_online;
              const status = getHostStatus(latestTimestamp, isOnline);
              return (
                <span className={STATUS_BADGE_CLASS[status]}>
                  <span className={STATUS_DOT_CLASS[status]} />
                  {STATUS_LABELS[status]}
                </span>
              );
            })()}
          </div>
          {latestTimestamp && (
            <div style={{ fontSize: 12, color: "var(--text-muted)", marginTop: 4 }}>
              {t.host.lastSeen} {new Date(latestTimestamp).toLocaleString()}
            </div>
          )}
        </div>
      </div>

      {/* Loading skeleton while waiting for connection */}
      {!isConnected && !hasData && (
        <div style={{ display: "grid", gap: 16 }}>
          {[220, 400, 200].map((h, i) => (
            <div key={i} className="skeleton" style={{ height: h }} />
          ))}
        </div>
      )}

      {/* Connected but no data available for this host yet */}
      {isConnected && !hasData && (
        <div
          className="glass-card"
          style={{
            padding: "48px 24px",
            textAlign: "center",
            color: "var(--text-muted)",
          }}
        >
          <Wifi size={40} style={{ margin: "0 auto 12px", opacity: 0.3 }} />
          <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 6 }}>
            {t.host.noMetrics}
          </div>
          <div style={{ fontSize: 13 }}>
            {t.host.noMetricsHint}
          </div>
        </div>
      )}

      {/* Actual data */}
      {hasData && (
        <>
          {/* Time-series chart — internally uses REST API + SSE trigger */}
          <div style={{ marginBottom: 20 }}>
            <TimeSeriesChart hostKey={decodedHostKey} />
          </div>

          {/* Load Average + Ports */}
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fit, minmax(280px, 1fr))",
              gap: 16,
              marginBottom: 16,
            }}
          >
            <SectionCard title={t.host.loadAverage} icon={<Activity size={15} />}>
              {loadAvg && <LoadGauge load={loadAvg} />}
            </SectionCard>

            <SectionCard
              title={`${t.host.portStatus} (${ports.length})`}
              icon={<Network size={15} />}
            >
              <PortList ports={ports} />
            </SectionCard>
          </div>

          {/* Disk + Processes grid */}
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fit, minmax(280px, 1fr))",
              gap: 16,
              marginBottom: 16,
            }}
          >
            <SectionCard
              title={`${t.host.diskUsage} (${disks.length})`}
              icon={<HardDrive size={15} />}
            >
              <DiskUsageBar disks={disks} />
            </SectionCard>

            <SectionCard
              title={`${t.host.topProcesses} (${processes.length})`}
              icon={<Cpu size={15} />}
            >
              <ProcessTable processes={processes} />
            </SectionCard>
          </div>

          {/* Temperature + GPU grid */}
          {(temperatures.length > 0 || gpus.length > 0) && (
            <div
              style={{
                display: "grid",
                gridTemplateColumns: "repeat(auto-fit, minmax(280px, 1fr))",
                gap: 16,
                marginBottom: 16,
              }}
            >
              {temperatures.length > 0 && (
                <SectionCard
                  title={`${t.host.temperatures} (${temperatures.length})`}
                  icon={<Thermometer size={15} />}
                >
                  <TemperatureDisplay temperatures={temperatures} />
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
          )}

          {/* Docker containers */}
          <div>
            <SectionCard
              title={`${t.host.dockerContainers} (${docker.length})`}
              icon={<Layers size={15} />}
            >
              <DockerGrid containers={docker} />
            </SectionCard>
          </div>
        </>
      )}
    </div>
  );
}
