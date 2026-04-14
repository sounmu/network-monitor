"use client";

import { useState, useMemo } from "react";
import useSWR from "swr";
import { Globe, Wifi, Plus, Trash2, CheckCircle, XCircle } from "lucide-react";
import {
  HttpMonitor, HttpMonitorSummary, PingMonitor, PingMonitorSummary,
  getHttpMonitorsUrl, getHttpSummariesUrl, getPingMonitorsUrl, getPingSummariesUrl,
  createHttpMonitor, deleteHttpMonitor, createPingMonitor, deletePingMonitor,
  fetcher,
} from "@/app/lib/api";
import { useI18n } from "@/app/i18n/I18nContext";

function MiniField({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <div style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 4 }}>{label}</div>
      {children}
    </div>
  );
}

type Tab = "http" | "ping";

export default function MonitorsPage() {
  const { t } = useI18n();
  const [activeTab, setActiveTab] = useState<Tab>("http");

  return (
    <div className="page-content fade-in">
      {/* Header */}
      <div style={{ marginBottom: 28 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 6 }}>
          <Globe size={20} color="var(--accent-blue)" />
          <h1 style={{ fontSize: 22, fontWeight: 800, color: "var(--text-primary)", letterSpacing: "-0.3px" }}>
            {t.monitors.title}
          </h1>
        </div>
      </div>

      {/* Tab buttons */}
      <div style={{ display: "flex", gap: 8, marginBottom: 24 }}>
        <button
          onClick={() => setActiveTab("http")}
          style={{
            display: "flex", alignItems: "center", gap: 6, padding: "8px 16px",
            borderRadius: 8, border: `1px solid ${activeTab === "http" ? "var(--accent-blue)" : "var(--border-subtle)"}`,
            background: activeTab === "http" ? "var(--accent-blue)" : "var(--bg-secondary)",
            color: activeTab === "http" ? "white" : "var(--text-secondary)",
            fontSize: 13, fontWeight: 600, cursor: "pointer",
          }}
        >
          <Globe size={14} /> {t.monitors.httpMonitors}
        </button>
        <button
          onClick={() => setActiveTab("ping")}
          style={{
            display: "flex", alignItems: "center", gap: 6, padding: "8px 16px",
            borderRadius: 8, border: `1px solid ${activeTab === "ping" ? "var(--accent-blue)" : "var(--border-subtle)"}`,
            background: activeTab === "ping" ? "var(--accent-blue)" : "var(--bg-secondary)",
            color: activeTab === "ping" ? "white" : "var(--text-secondary)",
            fontSize: 13, fontWeight: 600, cursor: "pointer",
          }}
        >
          <Wifi size={14} /> {t.monitors.pingMonitors}
        </button>
      </div>

      {/* Content */}
      {activeTab === "http" ? <HttpMonitorsTab /> : <PingMonitorsTab />}
    </div>
  );
}

function HttpMonitorsTab() {
  const { t } = useI18n();
  const { data: monitors, mutate: mutateMonitors } = useSWR<HttpMonitor[]>(
    getHttpMonitorsUrl(), fetcher, { revalidateOnFocus: false }
  );
  const { data: summaries } = useSWR<HttpMonitorSummary[]>(
    getHttpSummariesUrl(), fetcher, { refreshInterval: 10000, revalidateOnFocus: false }
  );

  const [showForm, setShowForm] = useState(false);
  const [formName, setFormName] = useState("");
  const [formUrl, setFormUrl] = useState("");
  const [formMethod, setFormMethod] = useState("GET");
  const [formExpectedStatus, setFormExpectedStatus] = useState(200);
  const [formInterval, setFormInterval] = useState(60);
  const [formTimeout, setFormTimeout] = useState(10000);

  const summaryMap = useMemo(
    () => new Map(summaries?.map((s) => [s.monitor_id, s])),
    [summaries]
  );

  const handleCreate = async () => {
    if (!formName.trim() || !formUrl.trim()) return;
    await createHttpMonitor({
      name: formName,
      url: formUrl,
      method: formMethod,
      expected_status: formExpectedStatus,
      interval_secs: formInterval,
      timeout_ms: formTimeout,
    });
    setShowForm(false);
    setFormName("");
    setFormUrl("");
    setFormMethod("GET");
    setFormExpectedStatus(200);
    setFormInterval(60);
    setFormTimeout(10000);
    await mutateMonitors();
  };

  const handleDelete = async (id: number) => {
    await deleteHttpMonitor(id);
    await mutateMonitors();
  };

  return (
    <div>
      {/* Add Monitor button */}
      <div style={{ display: "flex", justifyContent: "flex-end", marginBottom: 16 }}>
        <button
          onClick={() => setShowForm((v) => !v)}
          style={{
            display: "flex", alignItems: "center", gap: 6, padding: "6px 14px",
            borderRadius: 8, border: "1px solid var(--accent-blue)",
            background: "var(--accent-blue)", color: "white", fontSize: 12,
            fontWeight: 600, cursor: "pointer",
          }}
        >
          <Plus size={14} /> {t.monitors.addMonitor}
        </button>
      </div>

      {/* Add form */}
      {showForm && (
        <div className="glass-card" style={{ padding: 20, marginBottom: 12 }}>
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12, marginBottom: 12 }}>
            <MiniField label={t.monitors.name}>
              <input className="date-input" style={{ width: "100%" }} value={formName}
                onChange={(e) => setFormName(e.target.value)} placeholder="My API" />
            </MiniField>
            <MiniField label={t.monitors.url}>
              <input className="date-input" style={{ width: "100%" }} value={formUrl}
                onChange={(e) => setFormUrl(e.target.value)} placeholder="https://example.com" />
            </MiniField>
          </div>
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(140px, 1fr))", gap: 12, marginBottom: 16 }}>
            <MiniField label={t.monitors.method}>
              <select className="date-input" style={{ width: "100%" }} value={formMethod}
                onChange={(e) => setFormMethod(e.target.value)}>
                <option value="GET">GET</option>
                <option value="POST">POST</option>
                <option value="HEAD">HEAD</option>
              </select>
            </MiniField>
            <MiniField label={t.monitors.expectedStatus}>
              <input className="date-input" style={{ width: "100%" }} type="number"
                value={formExpectedStatus} onChange={(e) => setFormExpectedStatus(parseInt(e.target.value) || 200)} />
            </MiniField>
            <MiniField label={t.monitors.interval}>
              <input className="date-input" style={{ width: "100%" }} type="number"
                value={formInterval} onChange={(e) => setFormInterval(parseInt(e.target.value) || 60)} />
            </MiniField>
            <MiniField label={t.monitors.timeout}>
              <input className="date-input" style={{ width: "100%" }} type="number"
                value={formTimeout} onChange={(e) => setFormTimeout(parseInt(e.target.value) || 10000)} />
            </MiniField>
          </div>
          <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
            <button onClick={() => setShowForm(false)} style={{ padding: "6px 14px", borderRadius: 6, border: "1px solid var(--border-subtle)", background: "var(--bg-secondary)", color: "var(--text-secondary)", fontSize: 12, cursor: "pointer" }}>
              {t.common.cancel}
            </button>
            <button onClick={handleCreate} style={{ padding: "6px 14px", borderRadius: 6, border: "1px solid var(--accent-blue)", background: "var(--accent-blue)", color: "white", fontSize: 12, fontWeight: 600, cursor: "pointer" }}>
              {t.monitors.addMonitor}
            </button>
          </div>
        </div>
      )}

      {/* Monitor list */}
      {monitors?.map((monitor) => {
        const summary = summaryMap.get(monitor.id);
        const isHealthy = summary ? summary.latest_error === null : true;
        const uptimePct = summary?.uptime_pct ?? 0;
        const uptimeColor = uptimePct >= 99 ? "var(--accent-green)" : uptimePct >= 95 ? "var(--accent-yellow)" : "var(--accent-red)";

        return (
          <div key={monitor.id} className="glass-card" style={{ padding: "14px 20px", marginBottom: 8, display: "flex", alignItems: "center", gap: 12 }}>
            {/* Status indicator */}
            {isHealthy
              ? <CheckCircle size={18} color="var(--accent-green)" />
              : <XCircle size={18} color="var(--accent-red)" />
            }

            {/* Name & URL */}
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 14, fontWeight: 700, color: "var(--text-primary)" }}>{monitor.name}</div>
              <div style={{ fontSize: 12, color: "var(--text-muted)", fontFamily: "var(--font-mono), monospace", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {monitor.method} {monitor.url}
              </div>
            </div>

            {/* Response time */}
            {summary?.latest_response_time_ms != null && (
              <div style={{ fontSize: 13, fontFamily: "var(--font-mono), monospace", color: "var(--text-secondary)", whiteSpace: "nowrap" }}>
                {summary.latest_response_time_ms}ms
              </div>
            )}

            {/* Uptime */}
            {summary && (
              <div style={{ fontSize: 13, fontWeight: 600, color: uptimeColor, whiteSpace: "nowrap" }}>
                {uptimePct.toFixed(1)}%
              </div>
            )}

            {/* Delete */}
            <button onClick={() => handleDelete(monitor.id)} style={{
              padding: "4px 8px", borderRadius: 6,
              border: "1px solid var(--badge-offline-border)",
              background: "var(--status-offline-bg)", color: "var(--accent-red)",
              fontSize: 11, cursor: "pointer",
            }}>
              <Trash2 size={12} />
            </button>
          </div>
        );
      })}

      {(!monitors || monitors.length === 0) && !showForm && (
        <div className="glass-card" style={{ padding: 24, textAlign: "center", color: "var(--text-muted)", fontSize: 13 }}>
          {t.monitors.noMonitors}
        </div>
      )}
    </div>
  );
}

function PingMonitorsTab() {
  const { t } = useI18n();
  const { data: monitors, mutate: mutateMonitors } = useSWR<PingMonitor[]>(
    getPingMonitorsUrl(), fetcher, { revalidateOnFocus: false }
  );
  const { data: summaries } = useSWR<PingMonitorSummary[]>(
    getPingSummariesUrl(), fetcher, { refreshInterval: 10000, revalidateOnFocus: false }
  );

  const [showForm, setShowForm] = useState(false);
  const [formName, setFormName] = useState("");
  const [formHost, setFormHost] = useState("");

  const summaryMap = useMemo(
    () => new Map(summaries?.map((s) => [s.monitor_id, s])),
    [summaries]
  );

  const handleCreate = async () => {
    if (!formName.trim() || !formHost.trim()) return;
    await createPingMonitor({ name: formName, host: formHost });
    setShowForm(false);
    setFormName("");
    setFormHost("");
    await mutateMonitors();
  };

  const handleDelete = async (id: number) => {
    await deletePingMonitor(id);
    await mutateMonitors();
  };

  return (
    <div>
      {/* Add Monitor button */}
      <div style={{ display: "flex", justifyContent: "flex-end", marginBottom: 16 }}>
        <button
          onClick={() => setShowForm((v) => !v)}
          style={{
            display: "flex", alignItems: "center", gap: 6, padding: "6px 14px",
            borderRadius: 8, border: "1px solid var(--accent-blue)",
            background: "var(--accent-blue)", color: "white", fontSize: 12,
            fontWeight: 600, cursor: "pointer",
          }}
        >
          <Plus size={14} /> {t.monitors.addMonitor}
        </button>
      </div>

      {/* Add form */}
      {showForm && (
        <div className="glass-card" style={{ padding: 20, marginBottom: 12 }}>
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12, marginBottom: 16 }}>
            <MiniField label={t.monitors.name}>
              <input className="date-input" style={{ width: "100%" }} value={formName}
                onChange={(e) => setFormName(e.target.value)} placeholder="Gateway" />
            </MiniField>
            <MiniField label={t.monitors.host}>
              <input className="date-input" style={{ width: "100%" }} value={formHost}
                onChange={(e) => setFormHost(e.target.value)} placeholder="192.168.1.1 or 192.168.1.1:80" />
            </MiniField>
          </div>
          <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
            <button onClick={() => setShowForm(false)} style={{ padding: "6px 14px", borderRadius: 6, border: "1px solid var(--border-subtle)", background: "var(--bg-secondary)", color: "var(--text-secondary)", fontSize: 12, cursor: "pointer" }}>
              {t.common.cancel}
            </button>
            <button onClick={handleCreate} style={{ padding: "6px 14px", borderRadius: 6, border: "1px solid var(--accent-blue)", background: "var(--accent-blue)", color: "white", fontSize: 12, fontWeight: 600, cursor: "pointer" }}>
              {t.monitors.addMonitor}
            </button>
          </div>
        </div>
      )}

      {/* Monitor list */}
      {monitors?.map((monitor) => {
        const summary = summaryMap.get(monitor.id);
        const isHealthy = summary ? summary.latest_success === true : true;
        const uptimePct = summary?.uptime_pct ?? 0;
        const uptimeColor = uptimePct >= 99 ? "var(--accent-green)" : uptimePct >= 95 ? "var(--accent-yellow)" : "var(--accent-red)";

        return (
          <div key={monitor.id} className="glass-card" style={{ padding: "14px 20px", marginBottom: 8, display: "flex", alignItems: "center", gap: 12 }}>
            {/* Status indicator */}
            {isHealthy
              ? <CheckCircle size={18} color="var(--accent-green)" />
              : <XCircle size={18} color="var(--accent-red)" />
            }

            {/* Name & host */}
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 14, fontWeight: 700, color: "var(--text-primary)" }}>{monitor.name}</div>
              <div style={{ fontSize: 12, color: "var(--text-muted)", fontFamily: "var(--font-mono), monospace" }}>
                {monitor.host}
              </div>
            </div>

            {/* RTT */}
            {summary?.latest_rtt_ms != null && (
              <div style={{ fontSize: 13, fontFamily: "var(--font-mono), monospace", color: "var(--text-secondary)", whiteSpace: "nowrap" }}>
                {summary.latest_rtt_ms.toFixed(1)}ms
              </div>
            )}

            {/* Uptime */}
            {summary && (
              <div style={{ fontSize: 13, fontWeight: 600, color: uptimeColor, whiteSpace: "nowrap" }}>
                {uptimePct.toFixed(1)}%
              </div>
            )}

            {/* Delete */}
            <button onClick={() => handleDelete(monitor.id)} style={{
              padding: "4px 8px", borderRadius: 6,
              border: "1px solid var(--badge-offline-border)",
              background: "var(--status-offline-bg)", color: "var(--accent-red)",
              fontSize: 11, cursor: "pointer",
            }}>
              <Trash2 size={12} />
            </button>
          </div>
        );
      })}

      {(!monitors || monitors.length === 0) && !showForm && (
        <div className="glass-card" style={{ padding: 24, textAlign: "center", color: "var(--text-muted)", fontSize: 13 }}>
          {t.monitors.noMonitors}
        </div>
      )}
    </div>
  );
}
