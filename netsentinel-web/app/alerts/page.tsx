"use client";

import { Suspense, useCallback, useState } from "react";
import useSWR from "swr";
import { Bell } from "lucide-react";
import {
  AlertConfigRow,
  NotificationChannel,
  getAlertConfigsUrl,
  getHostsUrl,
  getNotificationChannelsUrl,
  fetcher,
} from "@/app/lib/api";
import { HostSummary } from "@/app/types/metrics";
import { useI18n } from "@/app/i18n/I18nContext";
import { PageHeader } from "@/app/components/PageHeader";
import { AlertsTabs, type AlertTab, useAlertsTab } from "./components/AlertsTabs";
import { AlertsSummaryBar } from "./components/AlertsSummaryBar";
import { ActiveAlertsPanel } from "./components/ActiveAlertsPanel";
import { RulesPanel } from "./components/RulesPanel";
import { HistoryPanel } from "./components/HistoryPanel";
import { ChannelsPanel } from "./components/ChannelsPanel";

export const dynamic = "force-static";

export default function AlertsPage() {
  return (
    <Suspense
      fallback={
        <div className="page-content">
          <div className="skeleton" style={{ height: 320 }} />
        </div>
      }
    >
      <AlertsPageInner />
    </Suspense>
  );
}

function AlertsPageInner() {
  const { t } = useI18n();
  const [tab, setTab] = useAlertsTab();

  const { data: hosts } = useSWR<HostSummary[]>(getHostsUrl(), fetcher, {
    revalidateOnFocus: false,
  });
  const { data: globalConfigs } = useSWR<AlertConfigRow[]>(getAlertConfigsUrl(), fetcher, {
    revalidateOnFocus: false,
  });
  const { data: channels } = useSWR<NotificationChannel[]>(getNotificationChannelsUrl(), fetcher, {
    revalidateOnFocus: false,
  });

  const [activeCount, setActiveCount] = useState<number | null>(null);
  const [channelsCount, setChannelsCount] = useState<number | null>(null);

  const handleActiveCount = useCallback((n: number | null) => setActiveCount(n), []);
  const handleChannelsCount = useCallback((n: number | null) => setChannelsCount(n), []);

  const rulesCount = globalConfigs ? globalConfigs.filter((c) => c.enabled).length : null;
  const hostsCount = hosts?.length ?? null;
  const channelsLive = channels?.length ?? channelsCount;

  const counts: Record<AlertTab, number | null> = {
    active: activeCount,
    rules: null,
    history: null,
    channels: channelsLive,
  };

  return (
    <div className="page-content fade-in">
      <PageHeader
        icon={<Bell size={18} aria-hidden="true" />}
        title={t.alerts.title}
        description={t.alerts.description}
      />

      <AlertsSummaryBar
        rulesCount={rulesCount}
        hostsCount={hostsCount}
        activeCount={activeCount}
        channelsCount={channelsLive}
      />

      <AlertsTabs current={tab} onChange={setTab} counts={counts} />

      {tab === "active" && <ActiveAlertsPanel onCountChange={handleActiveCount} />}
      {tab === "rules" && <RulesPanel />}
      {tab === "history" && <HistoryPanel />}
      {tab === "channels" && <ChannelsPanel onCountChange={handleChannelsCount} />}
    </div>
  );
}
