"use client";

import { useState } from "react";
import { toast } from "sonner";
import { useSWRConfig } from "swr";
import { CheckCheck, X } from "lucide-react";
import { bulkUpdateHostAlertConfigs, getHostAlertConfigsUrl } from "@/app/lib/api";
import { useI18n } from "@/app/i18n/I18nContext";
import type { AlertFormData } from "./shared";
import { apiErrorMessage, formToRequests } from "./shared";

interface Props {
  selectedCount: number;
  selectedHosts: string[];
  form: AlertFormData;
  onClear: () => void;
  onApplied: () => void;
}

export function BulkApplyBar({ selectedCount, selectedHosts, form, onClear, onApplied }: Props) {
  const { t } = useI18n();
  const { mutate } = useSWRConfig();
  const [applying, setApplying] = useState(false);

  const handleApply = async () => {
    setApplying(true);
    try {
      const configs = formToRequests(form);
      await bulkUpdateHostAlertConfigs({ host_keys: selectedHosts, configs });
      await Promise.all(
        selectedHosts.map((hostKey) => mutate(getHostAlertConfigsUrl(hostKey))),
      );
      toast.success(t.alerts.saved);
      onApplied();
    } catch (e) {
      toast.error(apiErrorMessage(e, t));
    } finally {
      setApplying(false);
    }
  };

  return (
    <div className="alerts-bulk-bar" role="region" aria-live="polite">
      <span className="alerts-bulk-bar__label">
        {t.alerts.rules.selectedHosts.replace("{count}", String(selectedCount))}
      </span>
      <div className="alerts-row alerts-row--tight">
        <button type="button" onClick={onClear} className="alerts-btn alerts-btn--sm alerts-btn--tonal">
          <X size={12} aria-hidden="true" />
          {t.alerts.rules.clearSelection}
        </button>
        <button
          type="button"
          onClick={handleApply}
          disabled={applying}
          className="alerts-btn alerts-btn--sm alerts-btn--filled"
        >
          <CheckCheck size={12} aria-hidden="true" />
          {applying ? t.alerts.saving : t.alerts.rules.bulkApply}
        </button>
      </div>
    </div>
  );
}
