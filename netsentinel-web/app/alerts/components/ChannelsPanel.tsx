"use client";

import { useEffect, useState } from "react";
import useSWR from "swr";
import { toast } from "sonner";
import { Plus, Save, Send, Trash2 } from "lucide-react";
import {
  NotificationChannel,
  getNotificationChannelsUrl,
  fetcher,
  createNotificationChannel,
  updateNotificationChannel,
  deleteNotificationChannel,
  testNotificationChannel,
} from "@/app/lib/api";
import { useI18n } from "@/app/i18n/I18nContext";
import { Switch } from "@/app/components/Switch";
import { apiErrorMessage } from "./shared";

interface Props {
  onCountChange?: (count: number | null) => void;
}

export function ChannelsPanel({ onCountChange }: Props) {
  const { t } = useI18n();
  const { data: channels, mutate } = useSWR<NotificationChannel[]>(
    getNotificationChannelsUrl(),
    fetcher,
    { revalidateOnFocus: false },
  );

  const [showForm, setShowForm] = useState(false);
  const [formType, setFormType] = useState<"discord" | "slack" | "email">("discord");
  const [formName, setFormName] = useState("");
  const [formConfig, setFormConfig] = useState<Record<string, string>>({});
  const [testMsg, setTestMsg] = useState<Record<number, string>>({});

  useEffect(() => {
    onCountChange?.(channels?.length ?? null);
  }, [channels, onCountChange]);

  const handleCreate = async () => {
    if (!formName.trim()) return;
    try {
      await createNotificationChannel({
        name: formName,
        channel_type: formType,
        config: formConfig,
      });
      setShowForm(false);
      setFormName("");
      setFormConfig({});
      await mutate();
    } catch (e) {
      toast.error(apiErrorMessage(e, t, t.notifications.testFailed));
    }
  };

  const handleDelete = async (id: number) => {
    try {
      await deleteNotificationChannel(id);
      await mutate();
    } catch (e) {
      toast.error(apiErrorMessage(e, t, t.notifications.testFailed));
    }
  };

  const handleToggle = async (ch: NotificationChannel) => {
    try {
      await updateNotificationChannel(ch.id, { enabled: !ch.enabled });
      await mutate();
    } catch (e) {
      toast.error(apiErrorMessage(e, t));
    }
  };

  const handleTest = async (id: number) => {
    try {
      await testNotificationChannel(id);
      setTestMsg((prev) => ({ ...prev, [id]: t.notifications.testSuccess }));
    } catch (e) {
      setTestMsg((prev) => ({ ...prev, [id]: apiErrorMessage(e, t, t.notifications.testFailed) }));
    }
    setTimeout(() => {
      setTestMsg((prev) => {
        const n = { ...prev };
        delete n[id];
        return n;
      });
    }, 3000);
  };

  const configFields =
    formType === "email"
      ? ["smtp_host", "smtp_port", "smtp_user", "smtp_pass", "from", "to"]
      : ["webhook_url"];

  const configLabels: Record<string, string> = {
    webhook_url: t.notifications.webhookUrl,
    smtp_host: t.notifications.smtpHost,
    smtp_port: t.notifications.smtpPort,
    smtp_user: t.notifications.smtpUser,
    smtp_pass: t.notifications.smtpPass,
    from: t.notifications.emailFrom,
    to: t.notifications.emailTo,
  };

  return (
    <div
      className="alerts-panel"
      id="alerts-panel-channels"
      role="tabpanel"
      aria-labelledby="alerts-tab-channels"
    >
      <div className="alerts-row alerts-row--between" style={{ marginBottom: 6 }}>
        <h2 className="alerts-section-title" style={{ marginBottom: 0 }}>
          {t.notifications.title}
        </h2>
        <button
          type="button"
          onClick={() => setShowForm((v) => !v)}
          className="alerts-btn alerts-btn--sm alerts-btn--filled"
        >
          <Plus size={14} aria-hidden="true" />
          {t.notifications.addChannel}
        </button>
      </div>

      <p className="alerts-section-description">{t.notifications.description}</p>

      {showForm && (
        <div className="glass-card alerts-section-card" style={{ marginBottom: 12 }}>
          <div className="alerts-form-grid-2">
            <div>
              <label htmlFor="notif-channel-name" className="alerts-field__label">
                {t.notifications.channelName}
              </label>
              <input
                id="notif-channel-name"
                className="alerts-field__input"
                value={formName}
                onChange={(e) => setFormName(e.target.value)}
                placeholder="My Slack"
              />
            </div>
            <div>
              <label htmlFor="notif-channel-type" className="alerts-field__label">
                {t.notifications.channelType}
              </label>
              <select
                id="notif-channel-type"
                className="alerts-field__input"
                value={formType}
                onChange={(e) => {
                  setFormType(e.target.value as "discord" | "slack" | "email");
                  setFormConfig({});
                }}
              >
                <option value="discord">Discord</option>
                <option value="slack">Slack</option>
                <option value="email">Email</option>
              </select>
            </div>
          </div>
          <div className="alerts-form-grid-auto">
            {configFields.map((field) => (
              <div key={field}>
                <label htmlFor={`notif-${field}`} className="alerts-field__label">
                  {configLabels[field] ?? field}
                </label>
                <input
                  id={`notif-${field}`}
                  className="alerts-field__input"
                  type={field === "smtp_pass" ? "password" : "text"}
                  value={formConfig[field] ?? ""}
                  onChange={(e) =>
                    setFormConfig((prev) => ({ ...prev, [field]: e.target.value }))
                  }
                />
              </div>
            ))}
          </div>
          <div className="alerts-row alerts-row--end alerts-row--tight">
            <button
              type="button"
              onClick={() => setShowForm(false)}
              className="alerts-btn alerts-btn--sm alerts-btn--tonal"
            >
              {t.common.cancel}
            </button>
            <button
              type="button"
              onClick={handleCreate}
              className="alerts-btn alerts-btn--sm alerts-btn--filled"
            >
              <Save size={12} aria-hidden="true" />
              {t.alerts.save}
            </button>
          </div>
        </div>
      )}

      {channels?.map((ch) => (
        <div key={ch.id} className="glass-card alerts-channel-card" style={{ marginBottom: 8 }}>
          <Switch
            checked={ch.enabled}
            onChange={() => handleToggle(ch)}
            aria-label={ch.name}
          />
          <div className="alerts-row__grow">
            <div className="alerts-channel__name">{ch.name}</div>
            <div className="alerts-channel__type">{ch.channel_type}</div>
          </div>
          {testMsg[ch.id] && (
            <span
              className={`alerts-channel__test-msg ${
                testMsg[ch.id] === t.notifications.testSuccess
                  ? "alerts-channel__test-msg--success"
                  : "alerts-channel__test-msg--error"
              }`}
            >
              {testMsg[ch.id]}
            </span>
          )}
          <button
            type="button"
            onClick={() => handleTest(ch.id)}
            className="alerts-btn alerts-btn--sm alerts-btn--tonal"
            aria-label={`${t.notifications.testSend}: ${ch.name}`}
          >
            <Send size={10} aria-hidden="true" />
            {t.notifications.testSend}
          </button>
          <button
            type="button"
            onClick={() => handleDelete(ch.id)}
            className="alerts-icon-btn alerts-icon-btn--danger"
            aria-label={`Delete ${ch.name}`}
          >
            <Trash2 size={14} aria-hidden="true" />
          </button>
        </div>
      ))}

      {(!channels || channels.length === 0) && !showForm && (
        <div className="glass-card alerts-card-empty">{t.notifications.noChannels}</div>
      )}
    </div>
  );
}
