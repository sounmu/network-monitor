"use client";

import { useEffect, useState } from "react";
import useSWR from "swr";
import { CheckCircle, XCircle, Shield } from "lucide-react";
import {
  ApiError,
  PublicMonitorStatus,
  PublicHostStatus,
  PublicStatusResponse,
  getPublicStatusUrl,
  publicFetcher,
} from "@/app/lib/api";
import { useI18n } from "@/app/i18n/I18nContext";
import { PageHeader } from "@/app/components/PageHeader";

function getUptimeColor(pct: number): string {
  if (pct >= 99.5) return "var(--accent-green)";
  if (pct >= 95) return "var(--accent-yellow)";
  return "var(--accent-red)";
}

type StatusRow = {
  key: string;
  primary: string;
  secondary: string;
  is_online: boolean;
  uptime: number;
  uptimeLabel: string;
};

export default function StatusPage() {
  const { t, locale } = useI18n();
  const { data, error, isLoading } = useSWR<PublicStatusResponse>(
    getPublicStatusUrl(), publicFetcher,
    { refreshInterval: 30000, shouldRetryOnError: false }
  );

  // `/api/public/status` returns 404 when `PUBLIC_STATUS_ENABLED` is unset.
  // That is a configuration signal, not a failure, so surface it explicitly
  // rather than showing an empty list (which is indistinguishable from "no
  // hosts registered" and wastes the operator's triage time).
  const isDisabled = error instanceof ApiError && error.status === 404;

  const hosts: PublicHostStatus[] = data?.hosts ?? [];
  const monitors: PublicMonitorStatus[] = data?.monitors ?? [];

  const allOnline =
    hosts.every((h) => h.is_online) && monitors.every((m) => m.is_online);

  const hostRows: StatusRow[] = hosts.map((h) => ({
    key: `host:${h.host_key}`,
    primary: h.display_name,
    secondary: h.host_key,
    is_online: h.is_online,
    uptime: h.uptime_7d,
    uptimeLabel: t.statusPage.uptime7d,
  }));

  const monitorRows: StatusRow[] = monitors.map((m) => ({
    key: `monitor:${m.kind}:${m.monitor_id}`,
    primary: `[${m.kind.toUpperCase()}] ${m.name}`,
    secondary: m.target,
    is_online: m.is_online,
    uptime: m.uptime_24h,
    uptimeLabel: t.statusPage.uptime24h,
  }));

  // Rendering `new Date()` during the render body produces a hydration
  // mismatch on /status (a public SSR-able page) — server wall-clock !==
  // client wall-clock even by a few ms. Keep the timestamp out of the
  // SSR'd HTML until after hydration; `null` on the server, then a real
  // Date on the client. set-state-in-effect is exactly the shape the rule
  // flags, but post-hydration state-seeding is the documented valid escape.
  const [now, setNow] = useState<Date | null>(null);
  /* eslint-disable react-hooks/set-state-in-effect */
  useEffect(() => {
    setNow(new Date());
    const id = setInterval(() => setNow(new Date()), 30_000);
    return () => clearInterval(id);
  }, []);
  /* eslint-enable react-hooks/set-state-in-effect */

  return (
    <div className="page-content fade-in" style={{ maxWidth: 720, margin: "0 auto" }}>
      <PageHeader
        icon={<Shield size={18} aria-hidden="true" />}
        title={t.statusPage.title}
        description={t.statusPage.subtitle}
        align="center"
      />

      {isDisabled ? (
        <div
          className="glass-card"
          style={{ padding: "24px", textAlign: "center" }}
        >
          <div style={{ fontSize: 15, fontWeight: 700, marginBottom: 8 }}>
            {t.statusPage.disabledTitle}
          </div>
          <div style={{ fontSize: 12, color: "var(--text-muted)" }}>
            {t.statusPage.disabledBody}
          </div>
        </div>
      ) : (
        <>
          {/* Overall status banner */}
          <div
            className="glass-card"
            style={{
              padding: "16px 24px",
              marginBottom: 24,
              display: "flex",
              alignItems: "center",
              gap: 12,
              background: allOnline ? "var(--status-online-bg)" : "var(--status-offline-bg)",
              borderColor: allOnline ? "var(--badge-online-border)" : "var(--badge-offline-border)",
            }}
          >
            {allOnline ? (
              <CheckCircle size={20} color="var(--badge-online-text)" />
            ) : (
              <XCircle size={20} color="var(--badge-offline-text)" />
            )}
            <span style={{
              fontSize: 15,
              fontWeight: 700,
              color: allOnline ? "var(--badge-online-text)" : "var(--badge-offline-text)",
            }}>
              {allOnline ? t.statusPage.allOperational : t.statusPage.someIssues}
            </span>
          </div>

          {renderSection(t.statusPage.hostsSection, hostRows, t)}
          {renderSection(t.statusPage.monitorsSection, monitorRows, t)}

          {isLoading && (
            <div className="glass-card" style={{ padding: 32, textAlign: "center" }}>
              <div className="skeleton" style={{ height: 48, borderRadius: 8, marginBottom: 8 }} />
              <div className="skeleton" style={{ height: 48, borderRadius: 8 }} />
            </div>
          )}

          {!isLoading && hostRows.length === 0 && monitorRows.length === 0 && (
            <div className="glass-card" style={{ padding: 32, textAlign: "center", color: "var(--text-muted)", fontSize: 13 }}>
              {t.statusPage.noHosts}
            </div>
          )}
        </>
      )}

      {/* Footer */}
      <div style={{ textAlign: "center", marginTop: 16, fontSize: 11, color: "var(--text-muted)" }}>
        {t.statusPage.lastUpdated}: {now ? now.toLocaleString(locale === "ko" ? "ko-KR" : "en-US") : ""}
      </div>
    </div>
  );
}

function renderSection(
  title: string,
  rows: StatusRow[],
  t: ReturnType<typeof useI18n>["t"],
) {
  if (rows.length === 0) return null;
  return (
    <div style={{ marginBottom: 24 }}>
      <div style={{ fontSize: 11, fontWeight: 700, textTransform: "uppercase", letterSpacing: 0.6, color: "var(--text-muted)", padding: "0 4px 8px" }}>
        {title}
      </div>
      <div className="glass-card" style={{ overflow: "hidden" }}>
        {rows.map((row, idx) => (
          <div
            key={row.key}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 12,
              padding: "16px 24px",
              borderBottom: idx < rows.length - 1 ? "1px solid var(--border-subtle)" : undefined,
            }}
          >
            {row.is_online ? (
              <CheckCircle size={18} color="var(--accent-green)" />
            ) : (
              <XCircle size={18} color="var(--accent-red)" />
            )}

            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 14, fontWeight: 600, color: "var(--text-primary)" }}>
                {row.primary}
              </div>
              <div style={{ fontSize: 11, color: "var(--text-muted)", fontFamily: "var(--font-mono), monospace", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {row.secondary}
              </div>
            </div>

            <span className={row.is_online ? "badge-online" : "badge-offline"}>
              {row.is_online ? t.statusPage.operational : t.statusPage.down}
            </span>

            <div style={{ textAlign: "right", minWidth: 80 }}>
              <div
                className="font-mono"
                style={{
                  fontSize: 14,
                  fontWeight: 700,
                  color: getUptimeColor(row.uptime),
                }}
              >
                {row.uptime.toFixed(2)}%
              </div>
              <div style={{ fontSize: 10, color: "var(--text-muted)" }}>
                {row.uptimeLabel}
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
