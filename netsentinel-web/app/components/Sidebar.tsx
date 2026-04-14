"use client";

import { usePathname, useRouter } from "next/navigation";
import { useMemo } from "react";
import { Activity, Server, WifiOff, AlertTriangle, LayoutDashboard, Radio, Settings, Bell, Sun, Moon, Shield, Globe, LogOut } from "lucide-react";
import { useSSE } from "@/app/lib/sse-context";
import {
  getHostStatus,
  STATUS_COLORS,
  STATUS_DOT_CLASS,
  HostStatus,
} from "@/app/lib/status";
import { useI18n } from "@/app/i18n/I18nContext";
import { useTheme } from "@/app/theme/ThemeContext";
import { useAuth } from "@/app/auth/AuthContext";

export default function Sidebar() {
  const pathname = usePathname();
  const router = useRouter();
  const { metricsMap, statusMap, isConnected } = useSSE();
  const { t, locale, setLocale } = useI18n();
  const { theme, toggleTheme } = useTheme();
  const { user, logout } = useAuth();

  // Build host list from statusMap — offline hosts are always visible (memoized)
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

  return (
    <aside
      style={{
        background: "var(--bg-secondary)",
        borderRight: "1px solid var(--border-subtle)",
        display: "flex",
        flexDirection: "column",
        height: "100%",
        overflowY: "auto",
      }}
    >
      {/* Logo */}
      <div
        style={{
          padding: "22px 20px 18px",
          borderBottom: "1px solid var(--border-subtle)",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
          <div
            style={{
              width: 32,
              height: 32,
              borderRadius: 8,
              background: "var(--accent-blue)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <Activity size={16} color="white" />
          </div>
          <div>
            <div style={{ fontWeight: 700, fontSize: 14, color: "var(--text-primary)" }}>
              {t.sidebar.appName}
            </div>
            <div style={{ fontSize: 11, color: "var(--text-muted)" }}>{t.sidebar.subtitle}</div>
          </div>
        </div>
      </div>

      {/* Summary stats */}
      <div style={{ padding: "14px 16px", borderBottom: "1px solid var(--border-subtle)" }}>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr 1fr", gap: "6px" }}>
          <StatMini label={t.sidebar.total} value={String(hosts.length)} color="var(--accent-blue)" />
          <StatMini label={t.sidebar.online} value={String(onlineCount)} color="var(--accent-green)" />
          <StatMini
            label={t.sidebar.pending}
            value={String(pendingCount)}
            color={pendingCount > 0 ? "var(--accent-yellow)" : "var(--text-muted)"}
          />
          <StatMini
            label={t.sidebar.offline}
            value={String(offlineCount)}
            color={offlineCount > 0 ? "var(--accent-red)" : "var(--text-muted)"}
          />
        </div>
      </div>

      {/* Navigation */}
      <nav aria-label="Main navigation" style={{ padding: "10px 10px 0" }}>
        <SidebarItem
          label={t.sidebar.overview}
          icon={<LayoutDashboard size={15} />}
          onClick={() => router.push("/")}
          active={pathname === "/"}
        />
        <SidebarItem
          label={t.sidebar.agents}
          icon={<Settings size={15} />}
          onClick={() => router.push("/agents")}
          active={pathname === "/agents"}
        />
        <SidebarItem
          label={t.sidebar.alerts}
          icon={<Bell size={15} />}
          onClick={() => router.push("/alerts")}
          active={pathname === "/alerts"}
        />
        <SidebarItem
          label={t.sidebar.monitors}
          icon={<Globe size={15} />}
          onClick={() => router.push("/monitors")}
          active={pathname === "/monitors"}
        />
        <SidebarItem
          label={t.sidebar.status}
          icon={<Shield size={15} />}
          onClick={() => router.push("/status")}
          active={pathname === "/status"}
        />
      </nav>

      {/* Server list */}
      <div style={{ padding: "14px 10px 0", flex: 1 }}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            marginBottom: "8px",
            padding: "0 6px",
          }}
        >
          <span
            style={{
              fontSize: 11,
              fontWeight: 600,
              color: "var(--text-muted)",
              textTransform: "uppercase",
              letterSpacing: "0.8px",
            }}
          >
            {t.sidebar.servers}
          </span>
        </div>

        {!isConnected && hosts.length === 0 && (
          <div style={{ padding: "0 6px" }}>
            {[1, 2, 3].map((i) => (
              <div key={i} className="skeleton" style={{ height: 40, marginBottom: 6 }} />
            ))}
          </div>
        )}

        {isConnected && hosts.length === 0 && (
          <div
            style={{
              padding: "20px 6px",
              textAlign: "center",
              color: "var(--text-muted)",
              fontSize: 13,
            }}
          >
            <Activity size={24} style={{ margin: "0 auto 8px", opacity: 0.3 }} />
            <div>{t.sidebar.waitingForAgents}</div>
          </div>
        )}

        <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
          {hosts.map((host) => {
            const href = `/host/${encodeURIComponent(host.host_key)}`;
            const active = pathname === href;
            const status = getHostStatus(host.last_seen, host.is_online);
            const colors = STATUS_COLORS[status];
            return (
              <button
                key={host.host_key}
                onClick={() => router.push(href)}
                aria-label={`View ${host.display_name}`}
                aria-current={active ? "page" : undefined}
                className={`sidebar-host-item ${active ? "sidebar-host-active" : ""}`}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 10,
                  padding: "8px 10px",
                  borderRadius: 8,
                  border: "1px solid",
                  borderColor: active ? "var(--accent-blue)" : "transparent",
                  background: active ? "var(--preset-hover-bg)" : "transparent",
                  cursor: "pointer",
                  width: "100%",
                  textAlign: "left",
                }}
              >
                <div
                  style={{
                    width: 28,
                    height: 28,
                    borderRadius: 8,
                    background: colors.bg,
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                    flexShrink: 0,
                  }}
                >
                  <StatusIcon status={status} size={13} />
                </div>
                <div style={{ minWidth: 0, flex: 1 }}>
                  <div
                    style={{
                      fontSize: 13,
                      fontWeight: 600,
                      color: active ? "var(--accent-blue)" : "var(--text-primary)",
                      whiteSpace: "nowrap",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                    }}
                  >
                    {host.display_name}
                  </div>
                  <div style={{ fontSize: 11, color: "var(--text-muted)", fontFamily: "var(--font-mono), monospace", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                    {host.display_name !== host.host_key
                      ? host.host_key
                      : status === "online" ? t.sidebar.active : status === "pending" ? t.sidebar.pendingStatus : t.sidebar.offlineStatus}
                  </div>
                </div>
                <span className={STATUS_DOT_CLASS[status]} style={{ flexShrink: 0 }} />
              </button>
            );
          })}
        </div>
      </div>

      {/* Bottom connection status */}
      <div
        style={{
          padding: "14px 20px",
          borderTop: "1px solid var(--border-subtle)",
          marginTop: "auto",
          display: "flex",
          alignItems: "center",
          gap: 6,
        }}
      >
        <Radio
          size={12}
          color={isConnected ? "var(--accent-green)" : "var(--accent-yellow)"}
        />
        <div style={{ fontSize: 11, color: "var(--text-muted)", flex: 1 }}>
          {isConnected ? (
            <>SSE: <span style={{ color: "var(--accent-green)", fontWeight: 600 }}>{t.sidebar.live}</span></>
          ) : (
            <>SSE: <span style={{ color: "var(--accent-yellow)", fontWeight: 600 }}>{t.sidebar.connecting}</span></>
          )}
        </div>
        <button
          onClick={toggleTheme}
          title="Toggle theme"
          aria-label={theme === "light" ? "Switch to dark mode" : "Switch to light mode"}
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            width: 24,
            height: 24,
            borderRadius: 4,
            border: "1px solid var(--border-subtle)",
            background: "var(--bg-secondary)",
            color: "var(--text-muted)",
            cursor: "pointer",
          }}
        >
          {theme === "light" ? <Moon size={12} /> : <Sun size={12} />}
        </button>
        <button
          onClick={() => setLocale(locale === "en" ? "ko" : "en")}
          title="Toggle language"
          aria-label={locale === "en" ? "Switch to Korean" : "Switch to English"}
          style={{
            fontSize: 10,
            fontWeight: 600,
            padding: "2px 6px",
            borderRadius: 4,
            border: "1px solid var(--border-subtle)",
            background: "var(--bg-secondary)",
            color: "var(--text-muted)",
            cursor: "pointer",
            letterSpacing: "0.3px",
          }}
        >
          {locale === "en" ? "KO" : "EN"}
        </button>
        {user && (
          <button
            onClick={logout}
            title={t.auth.logout}
            aria-label={t.auth.logout}
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              width: 24,
              height: 24,
              borderRadius: 4,
              border: "1px solid var(--border-subtle)",
              background: "var(--bg-secondary)",
              color: "var(--text-muted)",
              cursor: "pointer",
            }}
          >
            <LogOut size={12} />
          </button>
        )}
      </div>
    </aside>
  );
}

function StatusIcon({ status, size }: { status: HostStatus; size: number }) {
  const colors = STATUS_COLORS[status];
  if (status === "online") return <Server size={size} color={colors.accent} />;
  if (status === "pending") return <AlertTriangle size={size} color={colors.accent} />;
  return <WifiOff size={size} color={colors.accent} />;
}

function StatMini({ label, value, color }: { label: string; value: string; color: string }) {
  return (
    <div
      style={{
        background: "var(--bg-primary)",
        borderRadius: 8,
        padding: "8px 6px",
        textAlign: "center",
        border: "1px solid var(--border-subtle)",
      }}
    >
      <div style={{ fontSize: 17, fontWeight: 700, color, fontFamily: "var(--font-mono), monospace" }}>
        {value}
      </div>
      <div style={{ fontSize: 10, color: "var(--text-muted)", marginTop: 2 }}>{label}</div>
    </div>
  );
}

function SidebarItem({
  label,
  icon,
  onClick,
  active,
}: {
  label: string;
  icon: React.ReactNode;
  onClick: () => void;
  active: boolean;
}) {
  return (
    <button
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 10,
        padding: "9px 10px",
        borderRadius: 8,
        border: "1px solid",
        borderColor: active ? "var(--accent-blue)" : "transparent",
        background: active ? "var(--preset-hover-bg)" : "transparent",
        cursor: "pointer",
        width: "100%",
        color: active ? "var(--accent-blue)" : "var(--text-secondary)",
        fontSize: 13,
        fontWeight: active ? 600 : 400,
        transition: "all 0.15s ease",
        marginBottom: 2,
      }}
    >
      {icon}
      {label}
    </button>
  );
}
