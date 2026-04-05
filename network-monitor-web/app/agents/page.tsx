"use client";

import { useState, useCallback } from "react";
import useSWR from "swr";
import {
  Settings, Plus, Pencil, Trash2, Server, Save, X, AlertTriangle,
} from "lucide-react";
import {
  HostConfig, getHostsUrl, fetcher, getHostConfig, createHost, updateHost, deleteHost,
} from "@/app/lib/api";
import { HostSummary } from "@/app/types/metrics";
import { useI18n } from "@/app/i18n/I18nContext";

/** Host form data */
interface HostFormData {
  host_key: string;
  display_name: string;
  scrape_interval_secs: number;
  load_threshold: number;
  ports: string; // comma-separated string
  containers: string;
}

const emptyForm: HostFormData = {
  host_key: "",
  display_name: "",
  scrape_interval_secs: 10,
  load_threshold: 4.0,
  ports: "80, 443",
  containers: "",
};

function hostToForm(h: HostConfig): HostFormData {
  return {
    host_key: h.host_key,
    display_name: h.display_name,
    scrape_interval_secs: h.scrape_interval_secs,
    load_threshold: h.load_threshold,
    ports: h.ports.join(", "),
    containers: h.containers.join(", "),
  };
}

function parsePorts(s: string): number[] {
  return s.split(",").map((p) => parseInt(p.trim(), 10)).filter((n) => !isNaN(n));
}

function parseContainers(s: string): string[] {
  return s.split(",").map((c) => c.trim()).filter(Boolean);
}

export default function AgentsPage() {
  const { t } = useI18n();
  const { data: hosts, isLoading, error, mutate } = useSWR<HostSummary[]>(
    getHostsUrl(), fetcher, { revalidateOnFocus: false }
  );

  const [editingKey, setEditingKey] = useState<string | null>(null); // null=list, "new"=add, host_key=edit
  const [form, setForm] = useState<HostFormData>(emptyForm);
  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [deleteConfirm, setDeleteConfirm] = useState<string | null>(null);

  const openAdd = () => { setForm(emptyForm); setEditingKey("new"); setFormError(null); };
  const openEdit = async (hostKey: string) => {
    // Fetch full host config (HostSummary doesn't include config fields)
    try {
      const found = await getHostConfig(hostKey);
      setForm(hostToForm(found));
      setEditingKey(hostKey);
      setFormError(null);
    } catch {
      setFormError(t.agents.errorLoadHost);
    }
  };
  const closeForm = () => { setEditingKey(null); setFormError(null); };

  const handleSave = useCallback(async () => {
    if (!form.host_key.trim()) {
      setFormError(t.agents.errorHostKeyRequired);
      return;
    }
    if (!form.display_name.trim()) {
      setFormError(t.agents.errorDisplayNameRequired);
      return;
    }

    setSaving(true);
    setFormError(null);
    try {
      if (editingKey === "new") {
        await createHost({
          host_key: form.host_key.trim(),
          display_name: form.display_name.trim(),
          scrape_interval_secs: form.scrape_interval_secs,
          load_threshold: form.load_threshold,
          ports: parsePorts(form.ports),
          containers: parseContainers(form.containers),
        });
      } else if (editingKey) {
        await updateHost(editingKey, {
          display_name: form.display_name.trim(),
          scrape_interval_secs: form.scrape_interval_secs,
          load_threshold: form.load_threshold,
          ports: parsePorts(form.ports),
          containers: parseContainers(form.containers),
        });
      }
      await mutate();
      closeForm();
    } catch (e) {
      setFormError(e instanceof Error ? e.message : t.agents.errorSaveFailed);
    } finally {
      setSaving(false);
    }
  }, [form, editingKey, mutate, t]);

  const handleDelete = useCallback(async (hostKey: string) => {
    try {
      await deleteHost(hostKey);
      await mutate();
      setDeleteConfirm(null);
    } catch (e) {
      alert(e instanceof Error ? e.message : t.agents.errorDeleteFailed);
    }
  }, [mutate, t]);

  const updateField = <K extends keyof HostFormData>(key: K, value: HostFormData[K]) => {
    setForm((prev) => ({ ...prev, [key]: value }));
  };

  return (
    <div className="page-content fade-in">
      {/* Header */}
      <div style={{ marginBottom: 28 }}>
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 6 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <Settings size={20} color="var(--accent-blue)" />
            <h1 style={{ fontSize: 22, fontWeight: 800, color: "var(--text-primary)", letterSpacing: "-0.3px" }}>
              {t.agents.title}
            </h1>
          </div>
          {editingKey === null && (
            <button onClick={openAdd} style={{ display: "flex", alignItems: "center", gap: 6, padding: "8px 16px", borderRadius: 8, border: "1px solid var(--accent-blue)", background: "var(--accent-blue)", color: "white", fontSize: 13, fontWeight: 600, cursor: "pointer" }}>
              <Plus size={14} /> {t.agents.addAgent}
            </button>
          )}
        </div>
        <p style={{ color: "var(--text-muted)", fontSize: 13 }}>
          {t.agents.description}
        </p>
      </div>

      {/* Add/Edit form */}
      {editingKey !== null && (
        <div className="glass-card" style={{ padding: 24, marginBottom: 20 }}>
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 20 }}>
            <h2 style={{ fontSize: 15, fontWeight: 700, color: "var(--text-primary)" }}>
              {editingKey === "new" ? t.agents.addAgentTitle : t.agents.editAgentTitle}
            </h2>
            <button onClick={closeForm} style={{ display: "flex", alignItems: "center", justifyContent: "center", width: 32, height: 32, borderRadius: 8, border: "1px solid var(--border-subtle)", background: "transparent", cursor: "pointer", color: "var(--text-muted)" }}>
              <X size={16} />
            </button>
          </div>

          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(220px, 1fr))", gap: 16 }}>
            <FormField label={t.agents.hostKey} required>
              {editingKey === "new" ? (
                <input className="date-input" style={{ width: "100%", fontFamily: "var(--font-mono), monospace" }}
                  placeholder="192.168.1.10:9101" value={form.host_key}
                  onChange={(e) => updateField("host_key", e.target.value)} />
              ) : (
                <div style={{ padding: "7px 10px", borderRadius: 8, background: "var(--bg-tertiary)", border: "1px solid var(--border-subtle)", fontFamily: "var(--font-mono), monospace", fontSize: 13, color: "var(--text-muted)", userSelect: "all" }}>
                  {form.host_key}
                </div>
              )}
            </FormField>
            <FormField label={t.agents.displayName} required>
              <input className="date-input" style={{ width: "100%" }}
                placeholder="Production Server" value={form.display_name}
                onChange={(e) => updateField("display_name", e.target.value)} />
            </FormField>
            <FormField label={t.agents.scrapeInterval}>
              <input className="date-input" style={{ width: "100%" }} type="number" min={1}
                value={form.scrape_interval_secs}
                onChange={(e) => updateField("scrape_interval_secs", parseInt(e.target.value) || 10)} />
            </FormField>
            <FormField label={t.agents.loadThreshold}>
              <input className="date-input" style={{ width: "100%" }} type="number" step="0.1"
                value={form.load_threshold}
                onChange={(e) => updateField("load_threshold", parseFloat(e.target.value) || 4.0)} />
            </FormField>
            <FormField label={t.agents.monitoredPorts}>
              <input className="date-input" style={{ width: "100%", fontFamily: "var(--font-mono), monospace" }}
                placeholder="80, 443, 5432" value={form.ports}
                onChange={(e) => updateField("ports", e.target.value)} />
            </FormField>
            <FormField label={t.agents.dockerContainers}>
              <input className="date-input" style={{ width: "100%" }}
                placeholder="empty = monitor all" value={form.containers}
                onChange={(e) => updateField("containers", e.target.value)} />
            </FormField>
          </div>

          {formError && (
            <div style={{ marginTop: 16, padding: "10px 14px", borderRadius: 8, background: "var(--status-offline-bg)", border: "1px solid var(--badge-offline-border)", color: "var(--badge-offline-text)", fontSize: 13, display: "flex", alignItems: "center", gap: 8 }}>
              <AlertTriangle size={14} /> {formError}
            </div>
          )}

          <div style={{ display: "flex", gap: 10, marginTop: 20, justifyContent: "flex-end" }}>
            <button onClick={closeForm} style={{ padding: "8px 20px", borderRadius: 8, border: "1px solid var(--border-subtle)", background: "var(--bg-secondary)", color: "var(--text-secondary)", fontSize: 13, fontWeight: 500, cursor: "pointer" }}>
              {t.agents.cancel}
            </button>
            <button onClick={handleSave} disabled={saving} style={{ display: "flex", alignItems: "center", gap: 6, padding: "8px 20px", borderRadius: 8, border: "1px solid var(--accent-blue)", background: saving ? "var(--preset-hover-border)" : "var(--accent-blue)", color: "white", fontSize: 13, fontWeight: 600, cursor: saving ? "not-allowed" : "pointer" }}>
              <Save size={14} /> {saving ? t.agents.saving : t.agents.save}
            </button>
          </div>
        </div>
      )}

      {/* Host list */}
      <div>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 14 }}>
          <h2 style={{ fontSize: 15, fontWeight: 700, color: "var(--text-primary)" }}>{t.agents.registeredAgents}</h2>
          <span style={{ fontSize: 11, color: "var(--text-muted)", background: "var(--bg-card-hover)", padding: "2px 8px", borderRadius: 6 }}>
            {hosts?.length ?? 0} {t.agents.agentCount}
          </span>
        </div>

        {isLoading && (
          <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
            {[1, 2, 3].map((i) => <div key={i} className="skeleton" style={{ height: 80 }} />)}
          </div>
        )}

        {error && (
          <div className="glass-card" style={{ padding: 24, textAlign: "center", color: "var(--accent-red)" }}>
            {t.agents.errorLoadHost}
          </div>
        )}

        {hosts && hosts.length === 0 && (
          <div className="glass-card" style={{ padding: "48px 24px", textAlign: "center", color: "var(--text-muted)" }}>
            <Server size={40} style={{ margin: "0 auto 12px", opacity: 0.3 }} />
            <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 6 }}>{t.agents.noAgents}</div>
            <div style={{ fontSize: 13 }}>{t.agents.noAgentsHint}</div>
          </div>
        )}

        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          {hosts?.map((host) => (
            <div key={host.host_key} className="glass-card" style={{ padding: "16px 20px", display: "flex", alignItems: "center", gap: 16 }}>
              <div style={{ width: 42, height: 42, borderRadius: 8, background: host.is_online ? "var(--status-online-bg)" : "var(--status-offline-bg)", display: "flex", alignItems: "center", justifyContent: "center", flexShrink: 0 }}>
                <Server size={20} color={host.is_online ? "var(--accent-green)" : "var(--accent-red)"} />
              </div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 15, fontWeight: 700, color: "var(--text-primary)", marginBottom: 2 }}>
                  {host.display_name}
                </div>
                <div style={{ fontSize: 12, color: "var(--text-muted)", fontFamily: "var(--font-mono), monospace" }}>
                  {host.host_key}
                </div>
              </div>
              <span style={{ padding: "3px 10px", borderRadius: 6, fontSize: 11, fontWeight: 600, background: host.is_online ? "var(--status-online-bg)" : "var(--status-offline-bg)", color: host.is_online ? "var(--badge-online-text)" : "var(--badge-offline-text)", border: `1px solid ${host.is_online ? "var(--badge-online-border)" : "var(--badge-offline-border)"}` }}>
                {host.is_online ? t.common.online : t.common.offline}
              </span>

              <div style={{ display: "flex", gap: 6, flexShrink: 0 }}>
                <IconButton icon={<Pencil size={14} />} onClick={() => openEdit(host.host_key)} title="Edit" />
                {deleteConfirm === host.host_key ? (
                  <div style={{ display: "flex", gap: 4 }}>
                    <button onClick={() => handleDelete(host.host_key)} style={{ padding: "6px 12px", borderRadius: 6, border: "1px solid var(--accent-red)", background: "var(--accent-red)", color: "white", fontSize: 11, fontWeight: 600, cursor: "pointer" }}>{t.agents.deleteConfirmText}</button>
                    <button onClick={() => setDeleteConfirm(null)} style={{ padding: "6px 12px", borderRadius: 6, border: "1px solid var(--border-subtle)", background: "var(--bg-secondary)", color: "var(--text-secondary)", fontSize: 11, cursor: "pointer" }}>{t.agents.cancel}</button>
                  </div>
                ) : (
                  <IconButton icon={<Trash2 size={14} />} onClick={() => setDeleteConfirm(host.host_key)} title="Delete" danger />
                )}
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function FormField({ label, required, children }: { label: string; required?: boolean; children: React.ReactNode }) {
  return (
    <div>
      <label style={{ display: "block", fontSize: 12, fontWeight: 600, color: "var(--text-secondary)", marginBottom: 6 }}>
        {label}{required && <span style={{ color: "var(--accent-red)", marginLeft: 2 }}>*</span>}
      </label>
      {children}
    </div>
  );
}

function IconButton({ icon, onClick, title, danger }: { icon: React.ReactNode; onClick: () => void; title: string; danger?: boolean }) {
  return (
    <button onClick={onClick} title={title} style={{ display: "flex", alignItems: "center", justifyContent: "center", width: 32, height: 32, borderRadius: 8, border: "1px solid", borderColor: danger ? "var(--badge-offline-border)" : "var(--border-subtle)", background: danger ? "var(--status-offline-bg)" : "var(--bg-secondary)", color: danger ? "var(--accent-red)" : "var(--text-muted)", cursor: "pointer", transition: "all 0.15s ease" }}>
      {icon}
    </button>
  );
}
