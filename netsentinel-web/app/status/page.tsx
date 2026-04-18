"use client";

import { useEffect, useState } from "react";
import useSWR from "swr";
import { CheckCircle, XCircle, Shield } from "lucide-react";
import { PublicHostStatus, getPublicStatusUrl, publicFetcher } from "@/app/lib/api";
import { useI18n } from "@/app/i18n/I18nContext";
import { PageHeader } from "@/app/components/PageHeader";

function getUptimeColor(pct: number): string {
  if (pct >= 99.5) return "var(--accent-green)";
  if (pct >= 95) return "var(--accent-yellow)";
  return "var(--accent-red)";
}

export default function StatusPage() {
  const { t, locale } = useI18n();
  const { data: hosts, isLoading } = useSWR<PublicHostStatus[]>(
    getPublicStatusUrl(), publicFetcher,
    { refreshInterval: 30000 }
  );

  const allOnline = hosts?.every((h) => h.is_online) ?? true;

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

      {/* Host list */}
      <div className="glass-card" style={{ overflow: "hidden" }}>
        {hosts?.map((host, idx) => (
          <div
            key={host.host_key}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 12,
              padding: "16px 24px",
              borderBottom: idx < hosts.length - 1 ? "1px solid var(--border-subtle)" : undefined,
            }}
          >
            {/* Status icon */}
            {host.is_online ? (
              <CheckCircle size={18} color="var(--accent-green)" />
            ) : (
              <XCircle size={18} color="var(--accent-red)" />
            )}

            {/* Name */}
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 14, fontWeight: 600, color: "var(--text-primary)" }}>
                {host.display_name}
              </div>
              <div style={{ fontSize: 11, color: "var(--text-muted)", fontFamily: "var(--font-mono), monospace" }}>
                {host.host_key}
              </div>
            </div>

            {/* Status badge */}
            <span className={host.is_online ? "badge-online" : "badge-offline"}>
              {host.is_online ? t.statusPage.operational : t.statusPage.down}
            </span>

            {/* Uptime */}
            <div style={{ textAlign: "right", minWidth: 80 }}>
              <div
                className="font-mono"
                style={{
                  fontSize: 14,
                  fontWeight: 700,
                  color: getUptimeColor(host.uptime_7d),
                }}
              >
                {host.uptime_7d.toFixed(2)}%
              </div>
              <div style={{ fontSize: 10, color: "var(--text-muted)" }}>
                {t.statusPage.uptime7d}
              </div>
            </div>
          </div>
        ))}

        {isLoading && (
          <div style={{ padding: 32, textAlign: "center" }}>
            <div className="skeleton" style={{ height: 48, borderRadius: 8, marginBottom: 8 }} />
            <div className="skeleton" style={{ height: 48, borderRadius: 8 }} />
          </div>
        )}

        {!isLoading && (!hosts || hosts.length === 0) && (
          <div style={{ padding: 32, textAlign: "center", color: "var(--text-muted)", fontSize: 13 }}>
            {t.statusPage.noHosts}
          </div>
        )}
      </div>

      {/* Footer */}
      <div style={{ textAlign: "center", marginTop: 16, fontSize: 11, color: "var(--text-muted)" }}>
        {t.statusPage.lastUpdated}: {now ? now.toLocaleString(locale === "ko" ? "ko-KR" : "en-US") : ""}
      </div>
    </div>
  );
}
