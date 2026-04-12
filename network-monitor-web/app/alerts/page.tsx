"use client";

import { useState, useCallback } from "react";
import useSWR from "swr";
import { Bell, Save, Trash2, ChevronDown, ChevronUp, Plus, Send } from "lucide-react";
import {
  AlertConfigRow, UpsertAlertRequest, NotificationChannel, AlertHistoryRow,
  getAlertConfigsUrl, getHostAlertConfigsUrl, getHostsUrl, getNotificationChannelsUrl, getAlertHistoryUrl,
  fetcher, updateGlobalAlertConfigs, updateHostAlertConfigs, deleteHostAlertConfigs,
  createNotificationChannel, updateNotificationChannel, deleteNotificationChannel, testNotificationChannel,
} from "@/app/lib/api";
import { HostSummary } from "@/app/types/metrics";
import { useI18n } from "@/app/i18n/I18nContext";

interface AlertFormData {
  cpu_enabled: boolean;
  cpu_threshold: number;
  cpu_sustained_secs: number;
  cpu_cooldown_secs: number;
  memory_enabled: boolean;
  memory_threshold: number;
  memory_sustained_secs: number;
  memory_cooldown_secs: number;
  disk_enabled: boolean;
  disk_threshold: number;
  disk_sustained_secs: number;
  disk_cooldown_secs: number;
}

function configsToForm(configs: AlertConfigRow[]): AlertFormData {
  const cpu = configs.find((c) => c.metric_type === "cpu");
  const mem = configs.find((c) => c.metric_type === "memory");
  const disk = configs.find((c) => c.metric_type === "disk");
  return {
    cpu_enabled: cpu?.enabled ?? true,
    cpu_threshold: cpu?.threshold ?? 80,
    cpu_sustained_secs: cpu?.sustained_secs ?? 300,
    cpu_cooldown_secs: cpu?.cooldown_secs ?? 60,
    memory_enabled: mem?.enabled ?? true,
    memory_threshold: mem?.threshold ?? 90,
    memory_sustained_secs: mem?.sustained_secs ?? 300,
    memory_cooldown_secs: mem?.cooldown_secs ?? 60,
    disk_enabled: disk?.enabled ?? true,
    disk_threshold: disk?.threshold ?? 90,
    disk_sustained_secs: disk?.sustained_secs ?? 0,
    disk_cooldown_secs: disk?.cooldown_secs ?? 300,
  };
}

function formToRequests(form: AlertFormData): UpsertAlertRequest[] {
  return [
    { metric_type: "cpu", enabled: form.cpu_enabled, threshold: form.cpu_threshold, sustained_secs: form.cpu_sustained_secs, cooldown_secs: form.cpu_cooldown_secs },
    { metric_type: "memory", enabled: form.memory_enabled, threshold: form.memory_threshold, sustained_secs: form.memory_sustained_secs, cooldown_secs: form.memory_cooldown_secs },
    { metric_type: "disk", enabled: form.disk_enabled, threshold: form.disk_threshold, sustained_secs: form.disk_sustained_secs, cooldown_secs: form.disk_cooldown_secs },
  ];
}

export default function AlertsPage() {
  const { t } = useI18n();
  const { data: globalConfigs, mutate: mutateGlobal } = useSWR<AlertConfigRow[]>(
    getAlertConfigsUrl(), fetcher, { revalidateOnFocus: false }
  );
  const { data: hosts } = useSWR<HostSummary[]>(getHostsUrl(), fetcher, { revalidateOnFocus: false });

  const [globalForm, setGlobalForm] = useState<AlertFormData | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveMsg, setSaveMsg] = useState<string | null>(null);

  // Initialize global form (render-time state adjustment)
  if (globalConfigs && !globalForm) {
    setGlobalForm(configsToForm(globalConfigs));
  }

  const handleGlobalSave = useCallback(async () => {
    if (!globalForm) return;
    setSaving(true);
    setSaveMsg(null);
    try {
      await updateGlobalAlertConfigs(formToRequests(globalForm));
      await mutateGlobal();
      setSaveMsg(t.alerts.globalSaved);
      setTimeout(() => setSaveMsg(null), 3000);
    } catch (e) {
      setSaveMsg(e instanceof Error ? e.message : t.alerts.saveFailed);
    } finally {
      setSaving(false);
    }
  }, [globalForm, mutateGlobal, t]);

  return (
    <div className="page-content fade-in">
      {/* Header */}
      <div style={{ marginBottom: 28 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 6 }}>
          <Bell size={20} color="var(--accent-blue)" />
          <h1 style={{ fontSize: 22, fontWeight: 800, color: "var(--text-primary)", letterSpacing: "-0.3px" }}>
            {t.alerts.title}
          </h1>
        </div>
        <p style={{ color: "var(--text-muted)", fontSize: 13 }}>
          {t.alerts.description}
        </p>
      </div>

      {/* Global defaults */}
      <div className="glass-card" style={{ padding: 24, marginBottom: 24 }}>
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 20 }}>
          <h2 style={{ fontSize: 15, fontWeight: 700, color: "var(--text-primary)" }}>
            {t.alerts.globalDefaults}
          </h2>
          <button onClick={handleGlobalSave} disabled={saving || !globalForm} style={{ display: "flex", alignItems: "center", gap: 6, padding: "8px 16px", borderRadius: 8, border: "1px solid var(--accent-blue)", background: saving ? "var(--preset-hover-border)" : "var(--accent-blue)", color: "white", fontSize: 13, fontWeight: 600, cursor: saving ? "not-allowed" : "pointer" }}>
            <Save size={14} /> {saving ? t.alerts.saving : t.alerts.save}
          </button>
        </div>

        {saveMsg && (
          <div style={{ marginBottom: 16, padding: "8px 14px", borderRadius: 8, background: saveMsg === t.alerts.saveFailed ? "var(--status-offline-bg)" : "var(--status-online-bg)", color: saveMsg === t.alerts.saveFailed ? "var(--badge-offline-text)" : "var(--badge-online-text)", fontSize: 13, fontWeight: 500 }}>
            {saveMsg}
          </div>
        )}

        {globalForm && (
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))", gap: 20 }}>
            <MetricAlertForm label={t.alerts.cpuAlert} prefix="cpu" form={globalForm} setForm={setGlobalForm} />
            <MetricAlertForm label={t.alerts.memoryAlert} prefix="memory" form={globalForm} setForm={setGlobalForm} />
            <MetricAlertForm label={t.alerts.diskAlert} prefix="disk" form={globalForm} setForm={setGlobalForm} />
          </div>
        )}

        {!globalForm && <div className="skeleton" style={{ height: 200 }} />}
      </div>

      {/* Per-host overrides */}
      <div>
        <h2 style={{ fontSize: 15, fontWeight: 700, color: "var(--text-primary)", marginBottom: 14 }}>
          {t.alerts.perHostOverrides}
        </h2>
        <p style={{ color: "var(--text-muted)", fontSize: 12, marginBottom: 16 }}>
          {t.alerts.perHostDescription}
        </p>

        {hosts?.map((host) => (
          <HostAlertOverride key={host.host_key} host={host} globalConfigs={globalConfigs ?? []} />
        ))}

        {(!hosts || hosts.length === 0) && (
          <div className="glass-card" style={{ padding: 24, textAlign: "center", color: "var(--text-muted)", fontSize: 13 }}>
            {t.alerts.noHosts}
          </div>
        )}
      </div>

      {/* Alert history */}
      <AlertHistorySection />

      {/* Notification channels */}
      <NotificationChannelsSection />
    </div>
  );
}

/** Per-host alert override accordion */
function HostAlertOverride({ host, globalConfigs }: { host: HostSummary; globalConfigs: AlertConfigRow[] }) {
  const { t } = useI18n();
  const [expanded, setExpanded] = useState(false);
  const { data: hostConfigs, mutate } = useSWR<AlertConfigRow[]>(
    expanded ? getHostAlertConfigsUrl(host.host_key) : null,
    fetcher,
    { revalidateOnFocus: false }
  );

  const hasOverride = hostConfigs && hostConfigs.length > 0;
  const [form, setForm] = useState<AlertFormData | null>(null);
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  // Initialize form: use override values if they exist, otherwise use global defaults
  if (expanded && hostConfigs !== undefined && !form) {
    setForm(configsToForm(hasOverride ? hostConfigs : globalConfigs));
  }

  const handleSave = async () => {
    if (!form) return;
    setSaving(true);
    setMsg(null);
    try {
      await updateHostAlertConfigs(host.host_key, formToRequests(form));
      await mutate();
      setMsg(t.alerts.saved);
      setTimeout(() => setMsg(null), 3000);
    } catch (e) {
      setMsg(e instanceof Error ? e.message : t.alerts.saveFailed);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    try {
      await deleteHostAlertConfigs(host.host_key);
      setForm(null);
      await mutate();
      setMsg(t.alerts.revertedToGlobal);
      setTimeout(() => setMsg(null), 3000);
    } catch {
      // silently ignore if no override existed
    }
  };

  const toggle = () => {
    setExpanded((v) => !v);
    if (expanded) { setForm(null); setMsg(null); }
  };

  return (
    <div className="glass-card" style={{ marginBottom: 8, overflow: "hidden" }}>
      <button onClick={toggle} style={{ width: "100%", display: "flex", alignItems: "center", gap: 12, padding: "14px 20px", background: "transparent", border: "none", cursor: "pointer", textAlign: "left" }}>
        <div style={{ width: 8, height: 8, borderRadius: "50%", background: host.is_online ? "var(--accent-green)" : "var(--accent-red)", flexShrink: 0 }} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 14, fontWeight: 600, color: "var(--text-primary)" }}>{host.display_name}</div>
          <div style={{ fontSize: 11, color: "var(--text-muted)", fontFamily: "var(--font-mono), monospace" }}>{host.host_key}</div>
        </div>
        {hasOverride && (
          <span style={{ padding: "2px 8px", borderRadius: 6, background: "var(--preset-hover-bg)", color: "var(--accent-blue)", fontSize: 10, fontWeight: 600 }}>{t.alerts.override}</span>
        )}
        {expanded ? <ChevronUp size={16} color="var(--text-muted)" /> : <ChevronDown size={16} color="var(--text-muted)" />}
      </button>

      {expanded && form && (
        <div style={{ padding: "0 20px 20px", borderTop: "1px solid var(--border-subtle)" }}>
          <div style={{ paddingTop: 16, display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))", gap: 20 }}>
            <MetricAlertForm label={t.alerts.cpu} prefix="cpu" form={form} setForm={setForm} />
            <MetricAlertForm label={t.alerts.memory} prefix="memory" form={form} setForm={setForm} />
            <MetricAlertForm label={t.alerts.disk} prefix="disk" form={form} setForm={setForm} />
          </div>

          {msg && (
            <div style={{ marginTop: 12, fontSize: 12, color: msg === t.alerts.saveFailed ? "var(--accent-red)" : "var(--accent-green)", fontWeight: 500 }}>
              {msg}
            </div>
          )}

          <div style={{ display: "flex", gap: 8, marginTop: 16, justifyContent: "flex-end" }}>
            {hasOverride && (
              <button onClick={handleDelete} style={{ display: "flex", alignItems: "center", gap: 4, padding: "6px 14px", borderRadius: 6, border: "1px solid var(--badge-offline-border)", background: "var(--status-offline-bg)", color: "var(--accent-red)", fontSize: 12, fontWeight: 500, cursor: "pointer" }}>
                <Trash2 size={12} /> {t.alerts.deleteOverride}
              </button>
            )}
            <button onClick={handleSave} disabled={saving} style={{ display: "flex", alignItems: "center", gap: 4, padding: "6px 14px", borderRadius: 6, border: "1px solid var(--accent-blue)", background: "var(--accent-blue)", color: "white", fontSize: 12, fontWeight: 600, cursor: "pointer" }}>
              <Save size={12} /> {saving ? t.alerts.saving : t.alerts.save}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

/** Reusable CPU/memory alert configuration form */
function MetricAlertForm({ label, prefix, form, setForm }: {
  label: string;
  prefix: "cpu" | "memory" | "disk";
  form: AlertFormData;
  setForm: React.Dispatch<React.SetStateAction<AlertFormData | null>>;
}) {
  const { t } = useI18n();
  const enabled = form[`${prefix}_enabled`];
  const threshold = form[`${prefix}_threshold`];
  const sustained = form[`${prefix}_sustained_secs`];
  const cooldown = form[`${prefix}_cooldown_secs`];

  const update = (field: string, value: number | boolean) => {
    setForm((prev) => prev ? { ...prev, [`${prefix}_${field}`]: value } : prev);
  };

  return (
    <div style={{ opacity: enabled ? 1 : 0.5, transition: "opacity 0.2s" }}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 12 }}>
        <span style={{ fontSize: 13, fontWeight: 700, color: "var(--text-primary)" }}>{label}</span>
        <label style={{ display: "flex", alignItems: "center", gap: 6, cursor: "pointer", fontSize: 12, color: "var(--text-muted)" }}>
          <input type="checkbox" checked={enabled} onChange={(e) => update("enabled", e.target.checked)}
            style={{ width: 16, height: 16, accentColor: "var(--accent-blue)" }} />
          {t.alerts.enabled}
        </label>
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <MiniField label={t.alerts.threshold}>
          <input className="date-input" style={{ width: "100%" }} type="number" step="0.1"
            value={threshold} onChange={(e) => update("threshold", parseFloat(e.target.value) || 0)} />
        </MiniField>
        <MiniField label={t.alerts.sustained}>
          <input className="date-input" style={{ width: "100%" }} type="number"
            value={sustained} onChange={(e) => update("sustained_secs", parseInt(e.target.value) || 0)} />
        </MiniField>
        <MiniField label={t.alerts.cooldown}>
          <input className="date-input" style={{ width: "100%" }} type="number"
            value={cooldown} onChange={(e) => update("cooldown_secs", parseInt(e.target.value) || 0)} />
        </MiniField>
      </div>
    </div>
  );
}

function MiniField({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <div style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 4 }}>{label}</div>
      {children}
    </div>
  );
}

/** Alert history feed */
function AlertHistorySection() {
  const { t, locale } = useI18n();
  const { data: alerts } = useSWR<AlertHistoryRow[]>(
    getAlertHistoryUrl(undefined, 30), fetcher,
    { refreshInterval: 30000, revalidateOnFocus: false }
  );

  const alertTypeEmoji: Record<string, string> = {
    cpu_overload: "🔥", cpu_recovery: "✅",
    memory_overload: "🔥", memory_recovery: "✅",
    disk_overload: "💾", disk_recovery: "✅",
    load_overload: "⚡", load_recovery: "✅",
    port_down: "🚫", port_recovery: "✅",
    host_down: "🔴", host_recovery: "✅",
  };

  return (
    <div style={{ marginTop: 32 }}>
      <h2 style={{ fontSize: 15, fontWeight: 700, color: "var(--text-primary)", marginBottom: 14 }}>
        {t.alertHistory.title}
      </h2>

      <div className="glass-card" style={{ overflow: "hidden" }}>
        {(!alerts || alerts.length === 0) && (
          <div style={{ padding: 24, textAlign: "center", color: "var(--text-muted)", fontSize: 13 }}>
            {t.alertHistory.noAlerts}
          </div>
        )}

        {alerts?.map((alert) => (
          <div key={alert.id} style={{
            display: "flex", alignItems: "flex-start", gap: 10,
            padding: "10px 20px", borderBottom: "1px solid var(--border-subtle)",
            fontSize: 13,
          }}>
            <span style={{ fontSize: 14, flexShrink: 0 }}>
              {alertTypeEmoji[alert.alert_type] ?? "🔔"}
            </span>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ color: "var(--text-primary)", lineHeight: 1.4 }}>
                {alert.message.replace(/\*\*/g, "").replace(/`/g, "")}
              </div>
              <div style={{ fontSize: 11, color: "var(--text-muted)", marginTop: 2 }}>
                <span style={{ fontFamily: "var(--font-mono), monospace" }}>{alert.host_key}</span>
                {" · "}
                {new Date(alert.created_at).toLocaleString(locale === "ko" ? "ko-KR" : "en-US")}
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

/** Notification channels management section */
function NotificationChannelsSection() {
  const { t } = useI18n();
  const { data: channels, mutate } = useSWR<NotificationChannel[]>(
    getNotificationChannelsUrl(), fetcher, { revalidateOnFocus: false }
  );
  const [showForm, setShowForm] = useState(false);
  const [formType, setFormType] = useState<"discord" | "slack" | "email">("discord");
  const [formName, setFormName] = useState("");
  const [formConfig, setFormConfig] = useState<Record<string, string>>({});
  const [testMsg, setTestMsg] = useState<Record<number, string>>({});

  const handleCreate = async () => {
    if (!formName.trim()) return;
    await createNotificationChannel({
      name: formName,
      channel_type: formType,
      config: formConfig,
    });
    setShowForm(false);
    setFormName("");
    setFormConfig({});
    await mutate();
  };

  const handleDelete = async (id: number) => {
    await deleteNotificationChannel(id);
    await mutate();
  };

  const handleToggle = async (ch: NotificationChannel) => {
    await updateNotificationChannel(ch.id, { enabled: !ch.enabled });
    await mutate();
  };

  const handleTest = async (id: number) => {
    try {
      await testNotificationChannel(id);
      setTestMsg((prev) => ({ ...prev, [id]: t.notifications.testSuccess }));
    } catch {
      setTestMsg((prev) => ({ ...prev, [id]: t.notifications.testFailed }));
    }
    setTimeout(() => setTestMsg((prev) => { const n = { ...prev }; delete n[id]; return n; }), 3000);
  };

  const configFields = formType === "email"
    ? ["smtp_host", "smtp_port", "smtp_user", "smtp_pass", "from", "to"]
    : ["webhook_url"];

  const configLabels: Record<string, string> = {
    webhook_url: t.notifications.webhookUrl,
    smtp_host: t.notifications.smtpHost,
    smtp_port: t.notifications.smtpPort,
    smtp_user: t.notifications.smtpUser,
    smtp_pass: t.notifications.smtpPass,
    from: t.notifications.emailFrom,
    to: t.notifications.emailTo,
  };

  return (
    <div style={{ marginTop: 32 }}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 14 }}>
        <h2 style={{ fontSize: 15, fontWeight: 700, color: "var(--text-primary)" }}>
          {t.notifications.title}
        </h2>
        <button
          onClick={() => setShowForm((v) => !v)}
          style={{
            display: "flex", alignItems: "center", gap: 6, padding: "6px 14px",
            borderRadius: 8, border: "1px solid var(--accent-blue)",
            background: "var(--accent-blue)", color: "white", fontSize: 12,
            fontWeight: 600, cursor: "pointer",
          }}
        >
          <Plus size={14} /> {t.notifications.addChannel}
        </button>
      </div>

      <p style={{ color: "var(--text-muted)", fontSize: 12, marginBottom: 16 }}>
        {t.notifications.description}
      </p>

      {/* Add channel form */}
      {showForm && (
        <div className="glass-card" style={{ padding: 20, marginBottom: 12 }}>
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12, marginBottom: 12 }}>
            <MiniField label={t.notifications.channelName}>
              <input className="date-input" style={{ width: "100%" }} value={formName}
                onChange={(e) => setFormName(e.target.value)} placeholder="My Slack" />
            </MiniField>
            <MiniField label={t.notifications.channelType}>
              <select className="date-input" style={{ width: "100%" }} value={formType}
                onChange={(e) => { setFormType(e.target.value as "discord" | "slack" | "email"); setFormConfig({}); }}>
                <option value="discord">Discord</option>
                <option value="slack">Slack</option>
                <option value="email">Email</option>
              </select>
            </MiniField>
          </div>
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))", gap: 12, marginBottom: 16 }}>
            {configFields.map((field) => (
              <MiniField key={field} label={configLabels[field] ?? field}>
                <input className="date-input" style={{ width: "100%" }}
                  type={field === "smtp_pass" ? "password" : "text"}
                  value={formConfig[field] ?? ""}
                  onChange={(e) => setFormConfig((prev) => ({ ...prev, [field]: e.target.value }))}
                />
              </MiniField>
            ))}
          </div>
          <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
            <button onClick={() => setShowForm(false)} style={{ padding: "6px 14px", borderRadius: 6, border: "1px solid var(--border-subtle)", background: "var(--bg-secondary)", color: "var(--text-secondary)", fontSize: 12, cursor: "pointer" }}>
              {t.common.cancel}
            </button>
            <button onClick={handleCreate} style={{ padding: "6px 14px", borderRadius: 6, border: "1px solid var(--accent-blue)", background: "var(--accent-blue)", color: "white", fontSize: 12, fontWeight: 600, cursor: "pointer" }}>
              <Save size={12} /> {t.alerts.save}
            </button>
          </div>
        </div>
      )}

      {/* Channel list */}
      {channels?.map((ch) => (
        <div key={ch.id} className="glass-card" style={{ padding: "14px 20px", marginBottom: 8, display: "flex", alignItems: "center", gap: 12 }}>
          <div
            onClick={() => handleToggle(ch)}
            style={{ width: 32, height: 18, borderRadius: 9, background: ch.enabled ? "var(--accent-green)" : "var(--bg-card-hover)", cursor: "pointer", position: "relative", transition: "background 0.2s" }}
          >
            <div style={{ width: 14, height: 14, borderRadius: "50%", background: "white", position: "absolute", top: 2, left: ch.enabled ? 16 : 2, transition: "left 0.2s", boxShadow: "0 1px 3px rgba(0,0,0,0.2)" }} />
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: "var(--text-primary)" }}>{ch.name}</div>
            <div style={{ fontSize: 11, color: "var(--text-muted)" }}>{ch.channel_type}</div>
          </div>
          {testMsg[ch.id] && (
            <span style={{ fontSize: 11, color: testMsg[ch.id] === t.notifications.testSuccess ? "var(--accent-green)" : "var(--accent-red)", fontWeight: 500 }}>
              {testMsg[ch.id]}
            </span>
          )}
          <button onClick={() => handleTest(ch.id)} style={{ padding: "4px 10px", borderRadius: 6, border: "1px solid var(--border-subtle)", background: "var(--bg-secondary)", color: "var(--text-secondary)", fontSize: 11, cursor: "pointer", display: "flex", alignItems: "center", gap: 4 }}>
            <Send size={10} /> {t.notifications.testSend}
          </button>
          <button onClick={() => handleDelete(ch.id)} style={{ padding: "4px 8px", borderRadius: 6, border: "1px solid var(--badge-offline-border)", background: "var(--status-offline-bg)", color: "var(--accent-red)", fontSize: 11, cursor: "pointer" }}>
            <Trash2 size={10} />
          </button>
        </div>
      ))}

      {(!channels || channels.length === 0) && !showForm && (
        <div className="glass-card" style={{ padding: 24, textAlign: "center", color: "var(--text-muted)", fontSize: 13 }}>
          {t.notifications.noChannels}
        </div>
      )}
    </div>
  );
}
