"use client";

import useSWR from "swr";
import type { AlertConfigRow } from "@/app/lib/api";
import { fetcher, getHostAlertConfigsUrl } from "@/app/lib/api";
import type { HostSummary } from "@/app/types/metrics";
import { useI18n } from "@/app/i18n/I18nContext";
import type { MetricPrefix } from "./shared";

interface Props {
  hosts: HostSummary[];
  globalConfigs: AlertConfigRow[];
  selected: Set<string>;
  onToggle: (hostKey: string, checked: boolean) => void;
  onEdit: (host: HostSummary, metric: MetricPrefix) => void;
}

const METRICS: readonly MetricPrefix[] = ["cpu", "memory", "disk"] as const;

export function RulesMatrix({ hosts, globalConfigs, selected, onToggle, onEdit }: Props) {
  const { t } = useI18n();

  if (hosts.length === 0) {
    return <div className="alerts-card-empty glass-card">{t.alerts.noHosts}</div>;
  }

  const metricLabels: Record<MetricPrefix, string> = {
    cpu: t.alerts.cpu,
    memory: t.alerts.memory,
    disk: t.alerts.disk,
  };

  return (
    <>
      {/* Desktop — sticky table */}
      <div className="alerts-matrix-wrap" role="region" aria-label={t.alerts.rules.matrix}>
        <table className="alerts-matrix">
          <thead>
            <tr>
              <th scope="col" className="alerts-matrix__select" aria-label="select" />
              <th scope="col" className="alerts-matrix__host">
                {t.common.host}
              </th>
              {METRICS.map((m) => (
                <th key={m} scope="col">
                  {metricLabels[m]}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {hosts.map((host) => (
              <MatrixRow
                key={host.host_key}
                host={host}
                globalConfigs={globalConfigs}
                selected={selected.has(host.host_key)}
                onToggle={onToggle}
                onEdit={onEdit}
              />
            ))}
          </tbody>
        </table>
      </div>

      {/* Mobile fallback — card list */}
      <div className="alerts-matrix-mobile" role="list">
        {hosts.map((host) => (
          <MatrixMobileItem
            key={host.host_key}
            host={host}
            globalConfigs={globalConfigs}
            selected={selected.has(host.host_key)}
            onToggle={onToggle}
            onEdit={onEdit}
          />
        ))}
      </div>
    </>
  );
}

interface ResolvedCell {
  threshold: number;
  enabled: boolean;
  overridden: boolean;
}

function resolveConfig(
  hostConfigs: AlertConfigRow[] | undefined,
  globalConfigs: AlertConfigRow[],
  metric: MetricPrefix,
): ResolvedCell | null {
  const hostOverride = hostConfigs?.find((c) => c.metric_type === metric && !c.sub_key);
  if (hostOverride) {
    return { threshold: hostOverride.threshold, enabled: hostOverride.enabled, overridden: true };
  }
  const g = globalConfigs.find((c) => c.metric_type === metric && !c.sub_key);
  if (g) return { threshold: g.threshold, enabled: g.enabled, overridden: false };
  return null;
}

function MatrixRow({
  host,
  globalConfigs,
  selected,
  onToggle,
  onEdit,
}: {
  host: HostSummary;
  globalConfigs: AlertConfigRow[];
  selected: boolean;
  onToggle: (hostKey: string, checked: boolean) => void;
  onEdit: (host: HostSummary, metric: MetricPrefix) => void;
}) {
  const { data: hostConfigs } = useSWR<AlertConfigRow[]>(
    getHostAlertConfigsUrl(host.host_key),
    fetcher,
    { revalidateOnFocus: false, shouldRetryOnError: false },
  );

  return (
    <tr>
      <td className="alerts-matrix__select">
        <input
          type="checkbox"
          className="alerts-matrix__checkbox"
          checked={selected}
          onChange={(e) => onToggle(host.host_key, e.target.checked)}
          aria-label={`Select ${host.display_name}`}
        />
      </td>
      <th scope="row" className="alerts-matrix__host">
        <div className="alerts-matrix__host-name">{host.display_name}</div>
        <div className="alerts-matrix__host-key">{host.host_key}</div>
      </th>
      {METRICS.map((metric) => {
        const cell = resolveConfig(hostConfigs, globalConfigs, metric);
        return (
          <MatrixCell
            key={metric}
            cell={cell}
            onClick={() => onEdit(host, metric)}
            aria-label={`Edit ${metric} rule for ${host.display_name}`}
          />
        );
      })}
    </tr>
  );
}

function MatrixCell({
  cell,
  onClick,
  "aria-label": ariaLabel,
}: {
  cell: ResolvedCell | null;
  onClick: () => void;
  "aria-label": string;
}) {
  if (!cell) {
    return (
      <td className="alerts-matrix__cell alerts-matrix__cell--disabled">
        <button type="button" onClick={onClick} aria-label={ariaLabel}>
          —
        </button>
      </td>
    );
  }
  const className = [
    "alerts-matrix__cell",
    cell.overridden ? "alerts-matrix__cell--override" : "",
    cell.enabled ? "" : "alerts-matrix__cell--disabled",
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <td className={className}>
      <button type="button" onClick={onClick} aria-label={ariaLabel}>
        {cell.enabled ? `${cell.threshold}%` : "off"}
      </button>
    </td>
  );
}

function MatrixMobileItem({
  host,
  globalConfigs,
  selected,
  onToggle,
  onEdit,
}: {
  host: HostSummary;
  globalConfigs: AlertConfigRow[];
  selected: boolean;
  onToggle: (hostKey: string, checked: boolean) => void;
  onEdit: (host: HostSummary, metric: MetricPrefix) => void;
}) {
  const { t } = useI18n();
  const { data: hostConfigs } = useSWR<AlertConfigRow[]>(
    getHostAlertConfigsUrl(host.host_key),
    fetcher,
    { revalidateOnFocus: false, shouldRetryOnError: false },
  );
  const metricLabels: Record<MetricPrefix, string> = {
    cpu: t.alerts.cpu,
    memory: t.alerts.memory,
    disk: t.alerts.disk,
  };
  return (
    <div className="alerts-matrix-mobile__item" role="listitem">
      <div className="alerts-matrix-mobile__head">
        <label style={{ display: "flex", alignItems: "center", gap: 10, cursor: "pointer" }}>
          <input
            type="checkbox"
            className="alerts-matrix__checkbox"
            checked={selected}
            onChange={(e) => onToggle(host.host_key, e.target.checked)}
          />
          <div>
            <div className="alerts-matrix__host-name">{host.display_name}</div>
            <div className="alerts-matrix__host-key">{host.host_key}</div>
          </div>
        </label>
      </div>
      <div className="alerts-matrix-mobile__grid">
        {METRICS.map((metric) => {
          const cell = resolveConfig(hostConfigs, globalConfigs, metric);
          const className = [
            "alerts-matrix-mobile__cell",
            cell?.overridden ? "alerts-matrix-mobile__cell--override" : "",
          ]
            .filter(Boolean)
            .join(" ");
          return (
            <button
              key={metric}
              type="button"
              className={className}
              onClick={() => onEdit(host, metric)}
              aria-label={`Edit ${metric} rule for ${host.display_name}`}
            >
              <span className="alerts-matrix-mobile__cell-label">{metricLabels[metric]}</span>
              <span className="alerts-matrix-mobile__cell-value">
                {cell === null ? "—" : cell.enabled ? `${cell.threshold}%` : "off"}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
