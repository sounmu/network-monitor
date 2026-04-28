"use client";

import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useCallback } from "react";
import { useI18n } from "@/app/i18n/I18nContext";

export type AlertTab = "active" | "rules" | "history" | "channels";

export const ALERT_TABS: readonly AlertTab[] = ["active", "rules", "history", "channels"] as const;

export function parseTab(value: string | null | undefined): AlertTab {
  return (ALERT_TABS as readonly string[]).includes(value ?? "")
    ? (value as AlertTab)
    : "active";
}

export function useAlertsTab(): [AlertTab, (next: AlertTab) => void] {
  const router = useRouter();
  const pathname = usePathname();
  const params = useSearchParams();
  const current = parseTab(params.get("tab"));

  const setTab = useCallback(
    (next: AlertTab) => {
      const search = new URLSearchParams(params.toString());
      if (next === "active") search.delete("tab");
      else search.set("tab", next);
      const q = search.toString();
      router.replace(q ? `${pathname}?${q}` : pathname, { scroll: false });
    },
    [router, pathname, params],
  );

  return [current, setTab];
}

interface Props {
  current: AlertTab;
  onChange: (tab: AlertTab) => void;
  counts: Record<AlertTab, number | null>;
}

export function AlertsTabs({ current, onChange, counts }: Props) {
  const { t } = useI18n();
  const labels: Record<AlertTab, string> = {
    active: t.alerts.tabs.active,
    rules: t.alerts.tabs.rules,
    history: t.alerts.tabs.history,
    channels: t.alerts.tabs.channels,
  };

  return (
    <div className="alerts-tabs" role="tablist" aria-label={t.alerts.title}>
      {ALERT_TABS.map((tab) => {
        const selected = tab === current;
        const count = counts[tab];
        const critical = tab === "active" && typeof count === "number" && count > 0;
        return (
          <button
            key={tab}
            type="button"
            role="tab"
            aria-selected={selected}
            aria-controls={`alerts-panel-${tab}`}
            id={`alerts-tab-${tab}`}
            className="alerts-tab"
            onClick={() => onChange(tab)}
          >
            {labels[tab]}
            {typeof count === "number" && count > 0 && (
              <span
                className={`alerts-tab__count ${critical ? "alerts-tab__count--critical" : ""}`}
                aria-label={`${count}`}
              >
                {count}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}
