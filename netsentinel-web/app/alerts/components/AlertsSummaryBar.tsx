"use client";

import { useI18n } from "@/app/i18n/I18nContext";

interface Props {
  rulesCount: number | null;
  hostsCount: number | null;
  activeCount: number | null;
  channelsCount: number | null;
}

function formatCount(value: number | null) {
  if (value === null) return "—";
  return value.toLocaleString();
}

export function AlertsSummaryBar({ rulesCount, hostsCount, activeCount, channelsCount }: Props) {
  const { t } = useI18n();
  const activeCritical = typeof activeCount === "number" && activeCount > 0;

  return (
    <div className="alerts-summary" aria-live="polite">
      <div className={`alerts-summary__item ${activeCritical ? "alerts-summary__item--accent" : ""}`}>
        <span className="alerts-summary__label">{t.alerts.summary.active}</span>
        <span
          className={`alerts-summary__value ${
            activeCritical ? "alerts-summary__value--critical" : activeCount === 0 ? "alerts-summary__value--ok" : ""
          }`}
        >
          {formatCount(activeCount)}
        </span>
      </div>
      <div className="alerts-summary__item">
        <span className="alerts-summary__label">{t.alerts.summary.rules}</span>
        <span className="alerts-summary__value">{formatCount(rulesCount)}</span>
      </div>
      <div className="alerts-summary__item">
        <span className="alerts-summary__label">{t.alerts.summary.hosts}</span>
        <span className="alerts-summary__value">{formatCount(hostsCount)}</span>
      </div>
      <div className="alerts-summary__item">
        <span className="alerts-summary__label">{t.alerts.summary.channels}</span>
        <span className="alerts-summary__value">{formatCount(channelsCount)}</span>
      </div>
    </div>
  );
}
