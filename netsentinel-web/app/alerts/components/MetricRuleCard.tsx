"use client";

import type React from "react";
import { useI18n } from "@/app/i18n/I18nContext";
import { Switch } from "@/app/components/Switch";
import type { AlertFormData, MetricPrefix } from "./shared";

interface Props {
  label: string;
  prefix: MetricPrefix;
  form: AlertFormData;
  setForm: React.Dispatch<React.SetStateAction<AlertFormData | null>>;
  showPreview?: boolean;
}

export function MetricRuleCard({ label, prefix, form, setForm, showPreview = true }: Props) {
  const { t } = useI18n();
  const enabled = form[`${prefix}_enabled`];
  const threshold = form[`${prefix}_threshold`];
  const sustained = form[`${prefix}_sustained_secs`];
  const cooldown = form[`${prefix}_cooldown_secs`];

  const update = (field: string, value: number | boolean) => {
    setForm((prev) => (prev ? { ...prev, [`${prefix}_${field}`]: value } : prev));
  };

  const preview = enabled
    ? t.alerts.preview.sentence
        .replace("{metric}", label)
        .replace("{threshold}", String(threshold))
        .replace("{unit}", "%")
        .replace("{sustained}", String(sustained))
        .replace("{cooldown}", String(cooldown))
    : t.alerts.preview.sentenceDisabled.replace("{metric}", label);

  return (
    <div className={`alerts-metric ${enabled ? "" : "alerts-metric--disabled"}`}>
      <div className="alerts-metric__head">
        <span className="alerts-metric__label">{label}</span>
        <Switch
          checked={enabled}
          onChange={(next) => update("enabled", next)}
          aria-label={`${label} ${t.alerts.enabled}`}
        />
      </div>

      <div className="alerts-metric__fields">
        <div>
          <label htmlFor={`alert-${prefix}-threshold`} className="alerts-field__label">
            {t.alerts.threshold}
          </label>
          <div className="alerts-slider">
            <input
              id={`alert-${prefix}-threshold`}
              className="alerts-slider__track"
              type="range"
              min="0"
              max="100"
              step="1"
              value={threshold}
              onChange={(e) => update("threshold", parseFloat(e.target.value) || 0)}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={threshold}
            />
            <span className="alerts-slider__value">{threshold}%</span>
          </div>
        </div>

        <div>
          <label htmlFor={`alert-${prefix}-sustained`} className="alerts-field__label">
            {t.alerts.sustained}
          </label>
          <input
            id={`alert-${prefix}-sustained`}
            className="alerts-field__input"
            type="number"
            min={0}
            max={3600}
            value={sustained}
            onChange={(e) => update("sustained_secs", parseInt(e.target.value) || 0)}
          />
        </div>

        <div>
          <label htmlFor={`alert-${prefix}-cooldown`} className="alerts-field__label">
            {t.alerts.cooldown}
          </label>
          <input
            id={`alert-${prefix}-cooldown`}
            className="alerts-field__input"
            type="number"
            min={0}
            max={86400}
            value={cooldown}
            onChange={(e) => update("cooldown_secs", parseInt(e.target.value) || 0)}
          />
        </div>
      </div>

      {showPreview && <div className="alerts-preview">{preview}</div>}
    </div>
  );
}
