"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import useSWR from "swr";
import { Save, Trash2, ChevronDown, ChevronUp } from "lucide-react";
import {
  AlertConfigRow,
  getAlertConfigsUrl,
  getHostAlertConfigsUrl,
  getHostsUrl,
  fetcher,
  updateGlobalAlertConfigs,
  updateHostAlertConfigs,
  deleteHostAlertConfigs,
} from "@/app/lib/api";
import { HostSummary } from "@/app/types/metrics";
import { useI18n } from "@/app/i18n/I18nContext";
import {
  AlertFormData,
  apiErrorMessage,
  configsToForm,
  formToRequests,
  type MetricPrefix,
} from "./shared";
import { MetricRuleCard } from "./MetricRuleCard";
import { RulesMatrix } from "./RulesMatrix";
import { BulkApplyBar } from "./BulkApplyBar";
import { RuleDrawer } from "./RuleDrawer";

export function RulesPanel() {
  const { t } = useI18n();
  const { data: globalConfigs, mutate: mutateGlobal } = useSWR<AlertConfigRow[]>(
    getAlertConfigsUrl(),
    fetcher,
    { revalidateOnFocus: false },
  );
  const { data: hosts } = useSWR<HostSummary[]>(getHostsUrl(), fetcher, { revalidateOnFocus: false });

  const [globalForm, setGlobalForm] = useState<AlertFormData | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveMsg, setSaveMsg] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [drawer, setDrawer] = useState<{ host: HostSummary; metric: MetricPrefix } | null>(null);

  useEffect(() => {
    if (globalConfigs) setGlobalForm(configsToForm(globalConfigs));
  }, [globalConfigs]);

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
      setSaveMsg(apiErrorMessage(e, t));
    } finally {
      setSaving(false);
    }
  }, [globalForm, mutateGlobal, t]);

  const clearSelection = useCallback(() => setSelected(new Set()), []);
  const onBulkApplied = useCallback(() => setSelected(new Set()), []);

  const toggleSelect = useCallback((hostKey: string, checked: boolean) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (checked) next.add(hostKey);
      else next.delete(hostKey);
      return next;
    });
  }, []);

  const handleEdit = useCallback((host: HostSummary, metric: MetricPrefix) => {
    setDrawer({ host, metric });
  }, []);

  const saveMsgIsSuccess = saveMsg === t.alerts.globalSaved;

  return (
    <div className="alerts-panel" id="alerts-panel-rules" role="tabpanel" aria-labelledby="alerts-tab-rules">
      {selected.size > 0 && globalForm && (
        <BulkApplyBar
          selectedCount={selected.size}
          selectedHosts={Array.from(selected)}
          form={globalForm}
          onClear={clearSelection}
          onApplied={onBulkApplied}
        />
      )}

      <section className="glass-card alerts-section-card">
        <div className="alerts-section-card__head">
          <h2 className="alerts-section-card__title">{t.alerts.globalDefaults}</h2>
          <button
            type="button"
            onClick={handleGlobalSave}
            disabled={saving || !globalForm}
            className="alerts-btn alerts-btn--filled"
          >
            <Save size={14} aria-hidden="true" />
            {saving ? t.alerts.saving : t.alerts.save}
          </button>
        </div>

        {saveMsg && (
          <div
            role="status"
            aria-live="polite"
            className={`alerts-feedback ${
              saveMsgIsSuccess ? "alerts-feedback--success" : "alerts-feedback--error"
            }`}
          >
            {saveMsg}
          </div>
        )}

        {globalForm ? (
          <div className="alerts-metric-grid">
            <MetricRuleCard label={t.alerts.cpuAlert} prefix="cpu" form={globalForm} setForm={setGlobalForm} />
            <MetricRuleCard label={t.alerts.memoryAlert} prefix="memory" form={globalForm} setForm={setGlobalForm} />
            <MetricRuleCard label={t.alerts.diskAlert} prefix="disk" form={globalForm} setForm={setGlobalForm} />
          </div>
        ) : (
          <div className="skeleton" style={{ height: 200 }} />
        )}
      </section>

      <section className="alerts-section">
        <h2 className="alerts-section-title">{t.alerts.rules.matrix}</h2>
        <p className="alerts-section-description">{t.alerts.rules.matrixDescription}</p>

        <RulesMatrix
          hosts={hosts ?? []}
          globalConfigs={globalConfigs ?? []}
          selected={selected}
          onToggle={toggleSelect}
          onEdit={handleEdit}
        />
      </section>

      <section className="alerts-section">
        <h2 className="alerts-section-title">{t.alerts.perHostOverrides}</h2>
        <p className="alerts-section-description">{t.alerts.perHostDescription}</p>

        {hosts?.map((host) => (
          <HostAlertOverride
            key={host.host_key}
            host={host}
            globalConfigs={globalConfigs ?? []}
          />
        ))}

        {(!hosts || hosts.length === 0) && (
          <div className="glass-card alerts-card-empty">{t.alerts.noHosts}</div>
        )}
      </section>

      {drawer && (
        <RuleDrawer
          host={drawer.host}
          metric={drawer.metric}
          onClose={() => setDrawer(null)}
        />
      )}
    </div>
  );
}

function HostAlertOverride({
  host,
  globalConfigs,
}: {
  host: HostSummary;
  globalConfigs: AlertConfigRow[];
}) {
  const { t } = useI18n();
  const [expanded, setExpanded] = useState(false);
  const { data: hostConfigs, mutate } = useSWR<AlertConfigRow[]>(
    expanded ? getHostAlertConfigsUrl(host.host_key) : null,
    fetcher,
    { revalidateOnFocus: false },
  );

  const hasOverride = useMemo(
    () => !!hostConfigs && hostConfigs.length > 0,
    [hostConfigs],
  );
  const [form, setForm] = useState<AlertFormData | null>(null);
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  useEffect(() => {
    if (expanded && hostConfigs !== undefined) {
      setForm(configsToForm(hostConfigs.length > 0 ? hostConfigs : globalConfigs));
    }
  }, [expanded, hostConfigs, globalConfigs]);

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
      setMsg(apiErrorMessage(e, t));
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
      /* noop */
    }
  };

  const toggle = () => {
    setExpanded((v) => !v);
    if (expanded) {
      setForm(null);
      setMsg(null);
    }
  };

  const msgIsSuccess = msg === t.alerts.saved || msg === t.alerts.revertedToGlobal;

  return (
    <div className="glass-card" style={{ overflow: "hidden", marginBottom: 8 }}>
      <button
        type="button"
        onClick={toggle}
        className="alerts-host-toggle"
        aria-expanded={expanded}
      >
        <span
          className={`alerts-host-toggle__dot ${
            host.is_online
              ? "alerts-host-toggle__dot--online"
              : "alerts-host-toggle__dot--offline"
          }`}
          aria-hidden="true"
        />
        <div className="alerts-row__grow">
          <div className="alerts-host-toggle__name">{host.display_name}</div>
          <div className="alerts-host-toggle__key">{host.host_key}</div>
        </div>
        {hasOverride && <span className="alerts-override-chip">{t.alerts.override}</span>}
        {expanded ? (
          <ChevronUp size={16} color="var(--text-muted)" aria-hidden="true" />
        ) : (
          <ChevronDown size={16} color="var(--text-muted)" aria-hidden="true" />
        )}
      </button>

      {expanded && form && (
        <div className="alerts-host-body">
          <div className="alerts-metric-grid alerts-host-body__grid">
            <MetricRuleCard label={t.alerts.cpu} prefix="cpu" form={form} setForm={setForm} />
            <MetricRuleCard label={t.alerts.memory} prefix="memory" form={form} setForm={setForm} />
            <MetricRuleCard label={t.alerts.disk} prefix="disk" form={form} setForm={setForm} />
          </div>

          {msg && (
            <div
              role="status"
              aria-live="polite"
              className={`alerts-feedback alerts-feedback--inline ${
                msgIsSuccess ? "alerts-feedback--success" : "alerts-feedback--error"
              }`}
            >
              {msg}
            </div>
          )}

          <div
            className="alerts-row alerts-row--end alerts-row--tight"
            style={{ marginTop: 16 }}
          >
            {hasOverride && (
              <button
                type="button"
                onClick={handleDelete}
                className="alerts-btn alerts-btn--sm alerts-btn--danger"
              >
                <Trash2 size={12} aria-hidden="true" />
                {t.alerts.deleteOverride}
              </button>
            )}
            <button
              type="button"
              onClick={handleSave}
              disabled={saving}
              className="alerts-btn alerts-btn--sm alerts-btn--filled"
            >
              <Save size={12} aria-hidden="true" />
              {saving ? t.alerts.saving : t.alerts.save}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
