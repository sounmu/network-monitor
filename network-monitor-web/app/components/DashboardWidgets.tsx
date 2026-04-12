"use client";

import { useState, useCallback, useMemo } from "react";
import useSWR from "swr";
import { Settings2, Plus, X, Grip } from "lucide-react";
import { useSSE } from "@/app/lib/sse-context";
import { useI18n } from "@/app/i18n/I18nContext";
import {
  DashboardWidget,
  getDashboardUrl,
  saveDashboard,
  fetcher,
  AlertHistoryRow,
  getAlertHistoryUrl,
} from "@/app/lib/api";

export default function DashboardWidgets() {
  const { t, locale } = useI18n();
  const { metricsMap, statusMap } = useSSE();
  const { data: savedWidgets, mutate } = useSWR<DashboardWidget[]>(
    getDashboardUrl(), fetcher, { revalidateOnFocus: false }
  );
  const { data: alerts } = useSWR<AlertHistoryRow[]>(
    getAlertHistoryUrl(undefined, 10), fetcher,
    { refreshInterval: 30000, revalidateOnFocus: false }
  );

  const [widgets, setWidgets] = useState<DashboardWidget[]>([]);
  const [editing, setEditing] = useState(false);
  const [initialized, setInitialized] = useState(false);

  // Sync from server on first load (render-time state adjustment — React recommended pattern
  // for syncing with external data, avoids the set-state-in-effect lint rule)
  if (savedWidgets && !initialized) {
    setWidgets(savedWidgets);
    setInitialized(true);
  }

  const handleSave = useCallback(async () => {
    setEditing(false);
    await saveDashboard(widgets);
    await mutate();
  }, [widgets, mutate]);

  const addWidget = (type: DashboardWidget["type"], host_key?: string) => {
    const id = `${type}-${Date.now()}`;
    setWidgets((prev) => [...prev, { id, type, host_key }]);
  };

  const removeWidget = (id: string) => {
    setWidgets((prev) => prev.filter((w) => w.id !== id));
  };

  const hostKeys = useMemo(() => Object.keys(statusMap), [statusMap]);

  if (widgets.length === 0 && !editing) {
    return (
      <div style={{ marginBottom: 20 }}>
        <button
          onClick={() => setEditing(true)}
          style={{
            display: "flex", alignItems: "center", gap: 6,
            padding: "8px 16px", borderRadius: 8,
            border: "1px dashed var(--border-subtle)",
            background: "transparent", color: "var(--text-muted)",
            fontSize: 12, cursor: "pointer", width: "100%",
            justifyContent: "center",
          }}
        >
          <Plus size={14} /> {t.dashboard.addWidget}
        </button>
      </div>
    );
  }

  return (
    <div style={{ marginBottom: 20 }}>
      {/* Edit toolbar */}
      <div style={{ display: "flex", alignItems: "center", justifyContent: "flex-end", gap: 8, marginBottom: 12 }}>
        {editing ? (
          <>
            <select
              className="date-input"
              style={{ fontSize: 12, padding: "4px 8px" }}
              defaultValue=""
              onChange={(e) => {
                const val = e.target.value;
                if (val === "alert_feed" || val === "uptime_overview") {
                  addWidget(val as DashboardWidget["type"]);
                }
                e.target.value = "";
              }}
            >
              <option value="" disabled>{t.dashboard.addWidget}...</option>
              <option value="alert_feed">{t.dashboard.widgetTypes.alert_feed}</option>
              <option value="uptime_overview">{t.dashboard.widgetTypes.uptime_overview}</option>
            </select>
            {hostKeys.length > 0 && (
              <select
                className="date-input"
                style={{ fontSize: 12, padding: "4px 8px" }}
                defaultValue=""
                onChange={(e) => {
                  if (e.target.value) {
                    addWidget("host_status", e.target.value);
                    e.target.value = "";
                  }
                }}
              >
                <option value="" disabled>{t.dashboard.selectHost}...</option>
                {hostKeys.map((hk) => (
                  <option key={hk} value={hk}>
                    {statusMap[hk]?.display_name ?? hk}
                  </option>
                ))}
              </select>
            )}
            <button
              onClick={handleSave}
              style={{
                padding: "4px 14px", borderRadius: 6,
                border: "1px solid var(--accent-blue)",
                background: "var(--accent-blue)", color: "white",
                fontSize: 12, fontWeight: 600, cursor: "pointer",
              }}
            >
              {t.dashboard.done}
            </button>
          </>
        ) : (
          <button
            onClick={() => setEditing(true)}
            style={{
              display: "flex", alignItems: "center", gap: 4,
              padding: "4px 12px", borderRadius: 6,
              border: "1px solid var(--border-subtle)",
              background: "var(--bg-secondary)", color: "var(--text-muted)",
              fontSize: 11, cursor: "pointer",
            }}
          >
            <Settings2 size={12} /> {t.dashboard.customize}
          </button>
        )}
      </div>

      {/* Widget grid */}
      <div style={{
        display: "grid",
        gridTemplateColumns: "repeat(auto-fill, minmax(280px, 1fr))",
        gap: 12,
      }}>
        {widgets.map((widget) => (
          <div key={widget.id} className="glass-card" style={{ padding: 16, position: "relative" }}>
            {editing && (
              <div style={{ position: "absolute", top: 8, right: 8, display: "flex", gap: 4 }}>
                <button
                  onClick={() => removeWidget(widget.id)}
                  style={{
                    width: 20, height: 20, borderRadius: 4,
                    border: "1px solid var(--badge-offline-border)",
                    background: "var(--status-offline-bg)",
                    color: "var(--accent-red)", cursor: "pointer",
                    display: "flex", alignItems: "center", justifyContent: "center",
                  }}
                >
                  <X size={10} />
                </button>
              </div>
            )}
            {editing && (
              <Grip size={12} color="var(--text-muted)" style={{ marginBottom: 8 }} />
            )}
            <WidgetContent
              widget={widget}
              metricsMap={metricsMap}
              statusMap={statusMap}
              alerts={alerts ?? []}
              locale={locale}
              t={t}
            />
          </div>
        ))}
      </div>
    </div>
  );
}

function WidgetContent({
  widget,
  metricsMap,
  statusMap,
  alerts,
  locale,
  t,
}: {
  widget: DashboardWidget;
  metricsMap: Record<string, { cpu_usage_percent: number; memory_usage_percent: number; display_name: string; is_online: boolean }>;
  statusMap: Record<string, { host_key: string; display_name: string; is_online: boolean }>;
  alerts: AlertHistoryRow[];
  locale: string;
  t: ReturnType<typeof useI18n>["t"];
}) {
  if (widget.type === "host_status" && widget.host_key) {
    const metrics = metricsMap[widget.host_key];
    const status = statusMap[widget.host_key];
    const name = metrics?.display_name ?? status?.display_name ?? widget.host_key;
    const isOnline = metrics?.is_online ?? status?.is_online ?? false;
    const cpu = metrics?.cpu_usage_percent ?? 0;
    const mem = metrics?.memory_usage_percent ?? 0;

    return (
      <div>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 10 }}>
          <div style={{
            width: 8, height: 8, borderRadius: "50%",
            background: isOnline ? "var(--accent-green)" : "var(--accent-red)",
          }} />
          <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text-primary)" }}>{name}</span>
        </div>
        <div style={{ display: "flex", gap: 16 }}>
          <MiniGauge label={t.dashboard.cpu} value={cpu} />
          <MiniGauge label={t.dashboard.ram} value={mem} />
        </div>
      </div>
    );
  }

  if (widget.type === "alert_feed") {
    const recentAlerts = alerts.slice(0, 5);
    const alertEmoji: Record<string, string> = {
      cpu_overload: "🔥", cpu_recovery: "✅", memory_overload: "🔥", memory_recovery: "✅",
      disk_overload: "💾", disk_recovery: "✅", host_down: "🔴", host_recovery: "✅",
      port_down: "🚫", port_recovery: "✅", load_overload: "⚡", load_recovery: "✅",
    };

    return (
      <div>
        <div style={{ fontSize: 12, fontWeight: 700, color: "var(--text-primary)", marginBottom: 8 }}>
          {t.dashboard.widgetTypes.alert_feed}
        </div>
        {recentAlerts.length === 0 && (
          <div style={{ fontSize: 11, color: "var(--text-muted)" }}>{t.dashboard.noRecentAlerts}</div>
        )}
        {recentAlerts.map((a) => (
          <div key={a.id} style={{ fontSize: 11, color: "var(--text-secondary)", marginBottom: 4, lineHeight: 1.4 }}>
            <span>{alertEmoji[a.alert_type] ?? "🔔"} </span>
            <span style={{ fontFamily: "var(--font-mono), monospace", color: "var(--text-muted)", fontSize: 10 }}>
              {new Date(a.created_at).toLocaleTimeString(locale === "ko" ? "ko-KR" : "en-US")}
            </span>
            {" "}
            {a.message.replace(/\*\*/g, "").replace(/`/g, "").slice(0, 60)}
          </div>
        ))}
      </div>
    );
  }

  if (widget.type === "uptime_overview") {
    const hosts = Object.values(statusMap);
    const online = hosts.filter((h) => h.is_online).length;
    return (
      <div>
        <div style={{ fontSize: 12, fontWeight: 700, color: "var(--text-primary)", marginBottom: 8 }}>
          {t.dashboard.widgetTypes.uptime_overview}
        </div>
        <div style={{ fontSize: 24, fontWeight: 800, color: "var(--accent-green)", fontFamily: "var(--font-mono), monospace" }}>
          {hosts.length > 0 ? ((online / hosts.length) * 100).toFixed(0) : 0}%
        </div>
        <div style={{ fontSize: 11, color: "var(--text-muted)" }}>
          {online}/{hosts.length} online
        </div>
      </div>
    );
  }

  return <div style={{ fontSize: 11, color: "var(--text-muted)" }}>Unknown widget type</div>;
}

function MiniGauge({ label, value }: { label: string; value: number }) {
  const color = value < 60 ? "var(--accent-green)" : value < 80 ? "var(--accent-yellow)" : "var(--accent-red)";
  return (
    <div style={{ flex: 1 }}>
      <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 4 }}>
        <span style={{ fontSize: 10, color: "var(--text-muted)" }}>{label}</span>
        <span className="font-mono" style={{ fontSize: 12, fontWeight: 700, color }}>{value.toFixed(1)}%</span>
      </div>
      <div style={{ background: "var(--bg-card-hover)", borderRadius: 4, height: 6, overflow: "hidden" }}>
        <div style={{
          width: `${Math.min(value, 100)}%`, height: "100%",
          background: color, borderRadius: 4,
          transition: "width 0.5s ease",
        }} />
      </div>
    </div>
  );
}
