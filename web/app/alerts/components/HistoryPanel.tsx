"use client";

import { useState } from "react";
import useSWR from "swr";
import { Search, X } from "lucide-react";
import {
  AlertHistoryPage,
  getAlertHistoryUrl,
  fetcher,
  getHostsUrl,
} from "@/app/lib/api";
import { HostSummary } from "@/app/types/metrics";
import { useI18n } from "@/app/i18n/I18nContext";
import { useNowTick } from "@/app/lib/useNowTick";
import { alertTypeEmoji, alertTypeSeverity, formatRelative, sanitizeMarkdown } from "./shared";

type RangeKey = "24h" | "7d" | "30d" | "90d";

const RANGE_MS: Record<RangeKey, number> = {
  "24h": 24 * 60 * 60 * 1000,
  "7d": 7 * 24 * 60 * 60 * 1000,
  "30d": 30 * 24 * 60 * 60 * 1000,
  "90d": 90 * 24 * 60 * 60 * 1000,
};

const KNOWN_TYPES = [
  "cpu_overload",
  "cpu_recovery",
  "memory_overload",
  "memory_recovery",
  "disk_overload",
  "disk_recovery",
  "load_overload",
  "load_recovery",
  "port_down",
  "port_recovery",
  "host_down",
  "host_recovery",
  "monitor_down",
  "monitor_recovery",
  "temperature_overload",
  "temperature_recovery",
  "network_overload",
  "network_recovery",
  "gpu_overload",
  "gpu_recovery",
];

const PAGE_SIZE = 25;

export function HistoryPanel() {
  const { t, locale } = useI18n();
  const [type, setType] = useState<string>("all");
  const [hostKey, setHostKey] = useState<string>("all");
  const [range, setRange] = useState<RangeKey>("7d");
  const [search, setSearch] = useState<string>("");
  const [page, setPage] = useState<number>(0);

  const nowTick = useNowTick(30_000);

  const { data: hosts } = useSWR<HostSummary[]>(getHostsUrl(), fetcher, {
    revalidateOnFocus: false,
  });

  // Server-side filter: recompute "from" whenever nowTick advances a full range
  // bucket so SWR caches stay mostly hot between ticks.
  const now = nowTick || Date.parse(new Date().toISOString());
  const from = new Date(now - RANGE_MS[range]).toISOString();
  const limit = PAGE_SIZE;
  const offset = page * PAGE_SIZE;

  const { data: pageData } = useSWR<AlertHistoryPage>(
    nowTick === 0
      ? null
      : getAlertHistoryUrl({
          host_key: hostKey === "all" ? undefined : hostKey,
          type: type === "all" ? undefined : type,
          from,
          limit,
          offset,
        }),
    fetcher,
    { refreshInterval: 30000, revalidateOnFocus: false, keepPreviousData: true },
  );

  const rows = pageData?.rows ?? null;
  const total = pageData?.total ?? 0;
  const pageCount = Math.max(1, Math.ceil(total / PAGE_SIZE));

  // Client-side text search (server doesn't do LIKE yet)
  const q = search.trim().toLowerCase();
  const visible = rows
    ? q
      ? rows.filter((a) =>
          `${a.host_key} ${a.alert_type} ${a.message}`.toLowerCase().includes(q),
        )
      : rows
    : null;

  const handleClearFilters = () => {
    setType("all");
    setHostKey("all");
    setRange("7d");
    setSearch("");
    setPage(0);
  };

  const anyFilter = type !== "all" || hostKey !== "all" || range !== "7d" || search.length > 0;

  return (
    <div
      className="alerts-panel"
      id="alerts-panel-history"
      role="tabpanel"
      aria-labelledby="alerts-tab-history"
    >
      <div className="alerts-history-filters">
        <select
          className="alerts-chip-select"
          value={type}
          onChange={(e) => {
            setType(e.target.value);
            setPage(0);
          }}
          aria-label={t.alerts.history.filterType}
        >
          <option value="all">{t.alerts.history.allTypes}</option>
          {KNOWN_TYPES.map((tt) => (
            <option key={tt} value={tt}>
              {tt}
            </option>
          ))}
        </select>

        <select
          className="alerts-chip-select"
          value={hostKey}
          onChange={(e) => {
            setHostKey(e.target.value);
            setPage(0);
          }}
          aria-label={t.alerts.history.filterHost}
        >
          <option value="all">{t.alerts.history.allHosts}</option>
          {hosts?.map((h) => (
            <option key={h.host_key} value={h.host_key}>
              {h.display_name} ({h.host_key})
            </option>
          ))}
        </select>

        <select
          className="alerts-chip-select"
          value={range}
          onChange={(e) => {
            setRange(e.target.value as RangeKey);
            setPage(0);
          }}
          aria-label={t.alerts.history.filterRange}
        >
          {(Object.keys(RANGE_MS) as RangeKey[]).map((r) => (
            <option key={r} value={r}>
              {t.alerts.history.ranges[r]}
            </option>
          ))}
        </select>

        <div className="alerts-history-filters__search" style={{ position: "relative" }}>
          <Search
            size={14}
            style={{
              position: "absolute",
              left: 10,
              top: "50%",
              transform: "translateY(-50%)",
              color: "var(--md-sys-color-on-surface-variant)",
            }}
            aria-hidden="true"
          />
          <input
            type="search"
            className="alerts-field__input"
            placeholder={t.alerts.history.search}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            style={{ paddingLeft: 30 }}
          />
        </div>

        {anyFilter && (
          <button
            type="button"
            onClick={handleClearFilters}
            className="alerts-btn alerts-btn--sm alerts-btn--tonal"
          >
            <X size={12} aria-hidden="true" />
            {t.alerts.history.clearFilters}
          </button>
        )}
      </div>

      <div className="glass-card alerts-history-list" style={{ overflow: "hidden" }}>
        {visible === null && <div className="skeleton" style={{ height: 200 }} />}

        {visible && visible.length === 0 && (
          <div className="alerts-card-empty">{t.alerts.history.noResults}</div>
        )}

        {visible?.map((alert) => {
          const severity = alertTypeSeverity(alert.alert_type);
          return (
            <div key={alert.id} className="alerts-history-row">
              <span className="alerts-history-row__icon" aria-hidden="true">
                {alertTypeEmoji(alert.alert_type)}
              </span>
              <div className="alerts-row__grow">
                <div className="alerts-row alerts-row--tight" style={{ alignItems: "center" }}>
                  <span className={`alerts-severity alerts-severity--${severity}`}>
                    {alert.alert_type}
                  </span>
                </div>
                <div className="alerts-history-row__message" style={{ marginTop: 6 }}>
                  {sanitizeMarkdown(alert.message)}
                </div>
                <div className="alerts-history-row__meta">
                  <span className="alerts-history-row__host-key">{alert.host_key}</span>
                  <span className="alerts-history-row__sep">·</span>
                  <span>
                    {formatRelative(
                      alert.created_at,
                      locale,
                      nowTick || Date.parse(alert.created_at),
                    )}
                  </span>
                  <span className="alerts-history-row__sep">·</span>
                  <span>
                    {new Date(alert.created_at).toLocaleString(
                      locale === "ko" ? "ko-KR" : "en-US",
                    )}
                  </span>
                </div>
              </div>
            </div>
          );
        })}

        {visible && total > PAGE_SIZE && (
          <div className="alerts-history-footer">
            <span>
              {t.alerts.history.showingRange
                .replace("{count}", String(visible.length))
                .replace("{total}", String(total))}
            </span>
            <div className="alerts-row alerts-row--tight">
              <button
                type="button"
                className="alerts-btn alerts-btn--sm alerts-btn--tonal"
                disabled={page === 0}
                onClick={() => setPage((p) => Math.max(0, p - 1))}
              >
                {t.alerts.history.prev}
              </button>
              <button
                type="button"
                className="alerts-btn alerts-btn--sm alerts-btn--tonal"
                disabled={page >= pageCount - 1}
                onClick={() => setPage((p) => Math.min(pageCount - 1, p + 1))}
              >
                {t.alerts.history.next}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
