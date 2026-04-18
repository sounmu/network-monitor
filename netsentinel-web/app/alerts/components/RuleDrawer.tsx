"use client";

import { useEffect, useState } from "react";
import useSWR, { useSWRConfig } from "swr";
import { toast } from "sonner";
import { Save, Trash2, X } from "lucide-react";
import {
  AlertConfigRow,
  fetcher,
  getAlertConfigsUrl,
  getHostAlertConfigsUrl,
  updateHostAlertConfigs,
  deleteHostAlertConfigs,
} from "@/app/lib/api";
import { useI18n } from "@/app/i18n/I18nContext";
import type { HostSummary } from "@/app/types/metrics";
import type { AlertFormData, MetricPrefix } from "./shared";
import { apiErrorMessage, configsToForm, formToRequests } from "./shared";
import { MetricRuleCard } from "./MetricRuleCard";

interface Props {
  host: HostSummary;
  metric: MetricPrefix;
  onClose: () => void;
}

/**
 * Side-sheet editor for a single (host, metric) rule.
 * Fetches the host override, falls back to global, edits only the target
 * metric's row, and writes back all three metric overrides at once
 * (the existing PUT /api/alert-configs/{host_key} contract is all-or-nothing).
 */
export function RuleDrawer({ host, metric, onClose }: Props) {
  const { t } = useI18n();
  const { mutate: mutateCache } = useSWRConfig();

  const { data: hostConfigs, mutate: mutateHost } = useSWR<AlertConfigRow[]>(
    getHostAlertConfigsUrl(host.host_key),
    fetcher,
    { revalidateOnFocus: false },
  );
  const { data: globalConfigs } = useSWR<AlertConfigRow[]>(
    getAlertConfigsUrl(),
    fetcher,
    { revalidateOnFocus: false },
  );

  const hasOverride = !!hostConfigs && hostConfigs.length > 0;
  const [form, setForm] = useState<AlertFormData | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (hostConfigs !== undefined && globalConfigs !== undefined) {
      setForm(configsToForm(hostConfigs.length > 0 ? hostConfigs : globalConfigs));
    }
  }, [hostConfigs, globalConfigs]);

  // Close on Escape key.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const handleSave = async () => {
    if (!form) return;
    setSaving(true);
    try {
      await updateHostAlertConfigs(host.host_key, formToRequests(form));
      await mutateHost();
      toast.success(t.alerts.saved);
      onClose();
    } catch (e) {
      toast.error(apiErrorMessage(e, t));
    } finally {
      setSaving(false);
    }
  };

  const handleRevert = async () => {
    setSaving(true);
    try {
      await deleteHostAlertConfigs(host.host_key);
      await mutateHost(undefined, { revalidate: false });
      await mutateCache(getHostAlertConfigsUrl(host.host_key));
      toast.success(t.alerts.revertedToGlobal);
      onClose();
    } catch (e) {
      toast.error(apiErrorMessage(e, t));
    } finally {
      setSaving(false);
    }
  };

  const label = {
    cpu: t.alerts.cpuAlert,
    memory: t.alerts.memoryAlert,
    disk: t.alerts.diskAlert,
  }[metric];

  return (
    <>
      <div
        className="alerts-drawer-scrim"
        onClick={onClose}
        role="presentation"
      />
      <aside
        className="alerts-drawer"
        role="dialog"
        aria-labelledby="alerts-drawer-title"
        aria-modal="true"
      >
        <div className="alerts-drawer__header">
          <div>
            <h3 id="alerts-drawer-title" className="alerts-drawer__title">
              {label}
            </h3>
            <div className="alerts-drawer__subtitle">{host.host_key}</div>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="alerts-icon-btn"
            aria-label="Close"
          >
            <X size={16} aria-hidden="true" />
          </button>
        </div>

        <div className="alerts-drawer__body">
          {form ? (
            <MetricRuleCard label={label} prefix={metric} form={form} setForm={setForm} />
          ) : (
            <div className="skeleton" style={{ height: 180 }} />
          )}
        </div>

        <div className="alerts-drawer__footer">
          {hasOverride && (
            <button
              type="button"
              onClick={handleRevert}
              disabled={saving}
              className="alerts-btn alerts-btn--sm alerts-btn--danger"
            >
              <Trash2 size={12} aria-hidden="true" />
              {t.alerts.deleteOverride}
            </button>
          )}
          <button
            type="button"
            onClick={onClose}
            className="alerts-btn alerts-btn--sm alerts-btn--tonal"
          >
            {t.common.cancel}
          </button>
          <button
            type="button"
            onClick={handleSave}
            disabled={saving || !form}
            className="alerts-btn alerts-btn--sm alerts-btn--filled"
          >
            <Save size={12} aria-hidden="true" />
            {saving ? t.alerts.saving : t.alerts.save}
          </button>
        </div>
      </aside>
    </>
  );
}
