import { ApiError } from "@/app/lib/api";
import type { AlertConfigRow, UpsertAlertRequest } from "@/app/lib/api";
import type { Translations } from "@/app/i18n/translations";

export type MetricPrefix = "cpu" | "memory" | "disk";

export interface AlertFormData {
  cpu_enabled: boolean;
  cpu_threshold: number;
  cpu_sustained_secs: number;
  cpu_cooldown_secs: number;
  memory_enabled: boolean;
  memory_threshold: number;
  memory_sustained_secs: number;
  memory_cooldown_secs: number;
  disk_enabled: boolean;
  disk_threshold: number;
  disk_sustained_secs: number;
  disk_cooldown_secs: number;
}

export function configsToForm(configs: AlertConfigRow[]): AlertFormData {
  const cpu = configs.find((c) => c.metric_type === "cpu");
  const mem = configs.find((c) => c.metric_type === "memory");
  const disk = configs.find((c) => c.metric_type === "disk");
  return {
    cpu_enabled: cpu?.enabled ?? true,
    cpu_threshold: cpu?.threshold ?? 80,
    cpu_sustained_secs: cpu?.sustained_secs ?? 300,
    cpu_cooldown_secs: cpu?.cooldown_secs ?? 60,
    memory_enabled: mem?.enabled ?? true,
    memory_threshold: mem?.threshold ?? 90,
    memory_sustained_secs: mem?.sustained_secs ?? 300,
    memory_cooldown_secs: mem?.cooldown_secs ?? 60,
    disk_enabled: disk?.enabled ?? true,
    disk_threshold: disk?.threshold ?? 90,
    disk_sustained_secs: disk?.sustained_secs ?? 0,
    disk_cooldown_secs: disk?.cooldown_secs ?? 300,
  };
}

export function formToRequests(form: AlertFormData): UpsertAlertRequest[] {
  return [
    { metric_type: "cpu", enabled: form.cpu_enabled, threshold: form.cpu_threshold, sustained_secs: form.cpu_sustained_secs, cooldown_secs: form.cpu_cooldown_secs },
    { metric_type: "memory", enabled: form.memory_enabled, threshold: form.memory_threshold, sustained_secs: form.memory_sustained_secs, cooldown_secs: form.memory_cooldown_secs },
    { metric_type: "disk", enabled: form.disk_enabled, threshold: form.disk_threshold, sustained_secs: form.disk_sustained_secs, cooldown_secs: form.disk_cooldown_secs },
  ];
}

export function alertTypeSeverity(alertType: string): "critical" | "resolved" | "warn" {
  if (alertType.endsWith("_recovery")) return "resolved";
  if (alertType.endsWith("_overload") || alertType.endsWith("_down")) return "critical";
  return "warn";
}

export function alertTypeEmoji(alertType: string): string {
  const map: Record<string, string> = {
    cpu_overload: "🔥", cpu_recovery: "✅",
    memory_overload: "🔥", memory_recovery: "✅",
    disk_overload: "💾", disk_recovery: "✅",
    load_overload: "⚡", load_recovery: "✅",
    port_down: "🚫", port_recovery: "✅",
    host_down: "🔴", host_recovery: "✅",
    temperature_overload: "🌡️", temperature_recovery: "✅",
    network_overload: "📡", network_recovery: "✅",
    gpu_overload: "🎮", gpu_recovery: "✅",
    monitor_down: "🔌", monitor_recovery: "✅",
  };
  return map[alertType] ?? "🔔";
}

export function sanitizeMarkdown(msg: string): string {
  return msg.replace(/\*\*/g, "").replace(/`/g, "");
}

/**
 * Extracts a user-friendly toast message from an error thrown by the API
 * client. Recognizes 429 specifically so callers can show the translated
 * rate-limit notice without every handler duplicating the branch.
 */
export function apiErrorMessage(error: unknown, t: Translations, fallback?: string): string {
  if (error instanceof ApiError && error.status === 429) {
    return t.alerts.tooManyRequests;
  }
  if (error instanceof Error && error.message) {
    return error.message;
  }
  return fallback ?? t.alerts.saveFailed;
}

export function formatRelative(iso: string, locale: "en" | "ko", now: number): string {
  const then = new Date(iso).getTime();
  const diffSec = Math.max(0, Math.floor((now - then) / 1000));
  if (diffSec < 60) {
    return locale === "ko" ? `${diffSec}초 전` : `${diffSec}s ago`;
  }
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) {
    return locale === "ko" ? `${diffMin}분 전` : `${diffMin}m ago`;
  }
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) {
    return locale === "ko" ? `${diffHr}시간 전` : `${diffHr}h ago`;
  }
  const diffDay = Math.floor(diffHr / 24);
  return locale === "ko" ? `${diffDay}일 전` : `${diffDay}d ago`;
}
