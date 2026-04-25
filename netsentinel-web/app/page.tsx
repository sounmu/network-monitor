"use client";

import Link from "next/link";
import { useSSE } from "@/app/lib/sse-context";
import {
  getHostStatus,
  STATUS_DOT_CLASS,
  HostStatus,
} from "@/app/lib/status";
import React, { useMemo } from "react";
import { useI18n } from "@/app/i18n/I18nContext";
import { Activity, LayoutDashboard } from "lucide-react";
import { formatNetworkSpeed } from "@/app/lib/formatters";
import { PageHeader } from "@/app/components/PageHeader";

/**
 * Column-header labels + numeric values stay in the primary on-surface
 * color for maximum readability. Only the inline-meter BAR carries the
 * accent-green fill so a glance tells you "running" without the
 * rainbow of per-metric hues the overview previously used.
 */
const HEADER_COLOR = "var(--text-primary)";
const METRIC_VALUE_COLOR = "var(--text-primary)";
const METRIC_BAR_COLOR = "var(--accent-green)";

function InlineMeter({ value, max = 100 }: { value: number; max?: number }) {
  const pct = Math.min(Math.max((value / max) * 100, 0), 100);
  return (
    <div className="inline-meter">
      <span className="inline-meter-value">{pct.toFixed(1)}%</span>
      <span className="inline-meter-bar">
        <span
          className="inline-meter-fill"
          style={{ width: `${pct}%`, background: METRIC_BAR_COLOR }}
        />
      </span>
    </div>
  );
}

interface HostRow {
  host_key: string;
  display_name: string;
  is_online: boolean;
  last_seen: string | null;
  status: HostStatus;
  cpu: number;
  ram: number;
  disk: number; // root disk usage %
  load: number;
  networkRx: number;
  networkTx: number;
}

export default function HomePage() {
  const { metricsMap, statusMap, isConnected } = useSSE();
  const { t } = useI18n();

  const { hosts, onlineCount, offlineCount } = useMemo(() => {
    const list: HostRow[] = Object.values(statusMap).map((status) => {
      const metrics = metricsMap[status.host_key];
      const lastSeen = metrics?.timestamp ?? status.last_seen ?? null;
      const isOnline = metrics?.is_online ?? status.is_online ?? false;
      const hostStatus = getHostStatus(lastSeen, isOnline, status.scrape_interval_secs);

      // Root disk usage: pick "/" mount or highest usage partition
      const disks = status.disks ?? [];
      let diskPct = 0;
      if (disks.length > 0) {
        const root = disks.find((d) => d.mount_point === "/");
        diskPct = root ? root.usage_percent : Math.max(...disks.map((d) => d.usage_percent));
      }

      return {
        host_key: status.host_key,
        display_name: metrics?.display_name ?? status.display_name,
        is_online: isOnline,
        last_seen: lastSeen,
        status: hostStatus,
        cpu: metrics?.cpu_usage_percent ?? 0,
        ram: metrics?.memory_usage_percent ?? 0,
        disk: diskPct,
        load: metrics?.load_1min ?? 0,
        networkRx: metrics?.network_rate?.rx_bytes_per_sec ?? 0,
        networkTx: metrics?.network_rate?.tx_bytes_per_sec ?? 0,
      };
    });

    list.sort((a, b) => {
      const order: Record<HostStatus, number> = { online: 0, pending: 1, offline: 2 };
      const diff = order[a.status] - order[b.status];
      if (diff !== 0) return diff;
      return a.display_name.localeCompare(b.display_name);
    });

    let online = 0;
    let offline = 0;
    for (const h of list) {
      if (h.status === "online") online++;
      else if (h.status === "offline") offline++;
    }
    return { hosts: list, onlineCount: online, offlineCount: offline };
  }, [statusMap, metricsMap]);

  const isLoading = !isConnected && hosts.length === 0;

  return (
    <div className="page-content fade-in">
      <PageHeader
        icon={<LayoutDashboard size={18} aria-hidden="true" />}
        title={t.overview.title}
        badge={hosts.length}
        right={
          (onlineCount > 0 || offlineCount > 0) ? (
            <div className="page-header__stats">
              {onlineCount > 0 && (
                <span className="page-header__stats-item">
                  <span className="pulse-dot green" style={{ width: 6, height: 6 }} />
                  {onlineCount} {t.overview.online}
                </span>
              )}
              {offlineCount > 0 && (
                <span className="page-header__stats-item">
                  <span className="pulse-dot red" style={{ width: 6, height: 6 }} />
                  {offlineCount} {t.overview.offline}
                </span>
              )}
            </div>
          ) : undefined
        }
      />

      <div className="glass-card" style={{ overflow: "hidden" }}>
        {isLoading && (
          <div style={{ padding: 20 }}>
            {[1, 2, 3].map((i) => (
              <div key={i} className="skeleton" style={{ height: 48, marginBottom: 8 }} />
            ))}
          </div>
        )}

        {!isLoading && hosts.length === 0 && (
          <div
            style={{
              padding: "48px 24px",
              textAlign: "center",
              color: "var(--text-muted)",
            }}
          >
            <Activity size={36} style={{ margin: "0 auto 12px", opacity: 0.3 }} />
            <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 6 }}>
              {t.overview.noAgents}
            </div>
            <div style={{ fontSize: 13 }}>
              {t.overview.noAgentsHint}
            </div>
          </div>
        )}

        {!isLoading && hosts.length > 0 && (
          <div className="systems-table-wrap">
            <table className="systems-table">
              <thead>
                <tr>
                  <th>{t.overview.tableHeaders.system}</th>
                  <th style={{ width: "14%" }}>
                    <span style={{ color: HEADER_COLOR }}>{t.overview.tableHeaders.cpu}</span>
                  </th>
                  <th style={{ width: "14%" }}>
                    <span style={{ color: HEADER_COLOR }}>{t.overview.tableHeaders.memory}</span>
                  </th>
                  <th style={{ width: "14%" }}>
                    <span style={{ color: HEADER_COLOR }}>{t.overview.tableHeaders.disk}</span>
                  </th>
                  <th style={{ width: "9%" }}>
                    <span style={{ color: HEADER_COLOR }}>{t.overview.tableHeaders.load}</span>
                  </th>
                  <th style={{ width: "11%" }}>
                    <span style={{ color: HEADER_COLOR }}>{t.overview.tableHeaders.netRx}</span>
                  </th>
                  <th style={{ width: "11%" }}>
                    <span style={{ color: HEADER_COLOR }}>{t.overview.tableHeaders.netTx}</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {hosts.map((host) => {
                  const offline = host.status !== "online";
                  const dash = <span style={{ color: "var(--text-muted)", fontSize: 12 }}>—</span>;
                  return (
                    <tr
                      key={host.host_key}
                    >
                      <td>
                        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                          <span
                            className={STATUS_DOT_CLASS[host.status]}
                            style={{ width: 8, height: 8, flexShrink: 0 }}
                          />
                          <div style={{ minWidth: 0 }}>
                            <div
                              style={{
                                fontSize: 14,
                                fontWeight: 600,
                                color: "var(--text-primary)",
                                whiteSpace: "nowrap",
                                overflow: "hidden",
                                textOverflow: "ellipsis",
                              }}
                            >
                              <Link
                                href={`/host/?key=${encodeURIComponent(host.host_key)}`}
                                // `prefetch={false}` because Next.js fetches the route
                                // chunk by walking `/host/?key=…` in dev/preview mode,
                                // which our `output: 'export'` + `ServeDir` setup
                                // resolves to a 404 (the static asset lives at
                                // `/host/index.html`, query string is irrelevant to
                                // ServeDir). Disabling prefetch keeps the navigation
                                // path identical (Next still hydrates the cached
                                // chunk on click) without the noisy 404s in server
                                // logs and DevTools Network panel.
                                prefetch={false}
                                style={{ color: "inherit", textDecoration: "none" }}
                              >
                                {host.display_name}
                              </Link>
                            </div>
                            {host.display_name !== host.host_key && (
                              <div
                                style={{
                                  fontSize: 11,
                                  color: "var(--text-muted)",
                                  fontFamily: "var(--font-mono), monospace",
                                  whiteSpace: "nowrap",
                                  overflow: "hidden",
                                  textOverflow: "ellipsis",
                                }}
                              >
                                {host.host_key}
                              </div>
                            )}
                          </div>
                        </div>
                      </td>
                      <td>{offline ? dash : <InlineMeter value={host.cpu} />}</td>
                      <td>{offline ? dash : <InlineMeter value={host.ram} />}</td>
                      <td>{offline ? dash : <InlineMeter value={host.disk} />}</td>
                      <td>
                        {offline ? dash : (
                          <span
                            style={{
                              fontSize: 13,
                              fontWeight: 600,
                              color: METRIC_VALUE_COLOR,
                              fontVariantNumeric: "tabular-nums",
                            }}
                          >
                            {host.load.toFixed(2)}
                          </span>
                        )}
                      </td>
                      <td>
                        {offline ? dash : (
                          <span
                            style={{
                              fontSize: 13,
                              color: METRIC_VALUE_COLOR,
                              fontWeight: 600,
                              fontVariantNumeric: "tabular-nums",
                            }}
                          >
                            {formatNetworkSpeed(host.networkRx)}
                          </span>
                        )}
                      </td>
                      <td>
                        {offline ? dash : (
                          <span
                            style={{
                              fontSize: 13,
                              color: METRIC_VALUE_COLOR,
                              fontWeight: 600,
                              fontVariantNumeric: "tabular-nums",
                            }}
                          >
                            {formatNetworkSpeed(host.networkTx)}
                          </span>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
