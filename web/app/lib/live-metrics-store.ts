"use client";

import { useCallback, useSyncExternalStore } from "react";
import type { ChartMetricsRow, HostMetricsPayload } from "@/app/types/metrics";
// Relative import keeps the module testable under vitest, which does not
// resolve the `@/` path alias by default. The two files always live in
// the same directory anyway, so the alias gave no real abstraction win.
import { appendLiveMetricRow } from "./live-metrics";

type Listener = () => void;

const EMPTY_ROWS: readonly ChartMetricsRow[] = Object.freeze([]);
const rowsByHost = new Map<string, readonly ChartMetricsRow[]>();
const listenersByHost = new Map<string, Set<Listener>>();

function emit(hostKey: string) {
  const listeners = listenersByHost.get(hostKey);
  if (!listeners) return;
  for (const listener of listeners) {
    listener();
  }
}

function subscribe(hostKey: string, listener: Listener): () => void {
  let listeners = listenersByHost.get(hostKey);
  if (!listeners) {
    listeners = new Set();
    listenersByHost.set(hostKey, listeners);
  }

  listeners.add(listener);
  return () => {
    listeners?.delete(listener);
    if (listeners?.size === 0) {
      // Drop the buffer alongside the listener set when the last
      // chart for this host unmounts. Otherwise a user who navigates
      // through 100 host pages would leave behind 100 buffers each
      // capped at LIVE_METRICS_MAX_POINTS rows × ~1 KB per row, even
      // after all chart subscribers are gone. Re-subscribing later
      // simply starts a fresh buffer; the REST baseline still
      // provides historical context, so nothing user-visible is lost.
      listenersByHost.delete(hostKey);
      rowsByHost.delete(hostKey);
    }
  };
}

function getSnapshot(hostKey: string): readonly ChartMetricsRow[] {
  return rowsByHost.get(hostKey) ?? EMPTY_ROWS;
}

function getServerSnapshot(): readonly ChartMetricsRow[] {
  return EMPTY_ROWS;
}

export function pushLiveMetricPayload(payload: HostMetricsPayload) {
  // Drop SSE samples that nobody is consuming. Without this guard the
  // store accumulates a per-host ring buffer for every host the user has
  // ever viewed in this tab, even after every chart for that host has
  // unmounted. Newly mounted charts get their historical baseline from
  // the REST `/api/metrics/.../chart` fetch, so missing the gap between
  // mount and the first SSE tick is not user-visible — the live overlay
  // simply starts populating from the next event onward. See the
  // companion test in `live-metrics.test.ts` (`live-metrics-store cleanup`).
  const listeners = listenersByHost.get(payload.host_key);
  if (!listeners || listeners.size === 0) return;

  const previousRows = rowsByHost.get(payload.host_key) ?? EMPTY_ROWS;
  const nextRows = appendLiveMetricRow(previousRows, payload);
  // `appendLiveMetricRow` returns the same reference when the payload
  // had a non-finite timestamp (or otherwise produced no semantic
  // change); short-circuit here so a corrupted event doesn't fan out
  // a no-op re-render to every chart subscribed on this host.
  if (nextRows === previousRows) return;

  rowsByHost.set(payload.host_key, nextRows);
  emit(payload.host_key);
}

export function clearLiveMetricRows(hostKey: string) {
  if (!rowsByHost.delete(hostKey)) return;
  emit(hostKey);
}

// Test-only escape hatch: lets the unit test verify that the store has
// fully released its per-host state after the last subscriber unmounts.
// Not exported to consumers (`export function` would still be reachable
// from app code if imported, but the name + JSDoc signal intent).
/** @internal */
export function __hasLiveMetricEntry(hostKey: string): boolean {
  return rowsByHost.has(hostKey) || listenersByHost.has(hostKey);
}

export function useHostLiveRows(hostKey: string): readonly ChartMetricsRow[] {
  const subscribeHost = useCallback(
    (listener: Listener) => subscribe(hostKey, listener),
    [hostKey],
  );
  const getHostSnapshot = useCallback(() => getSnapshot(hostKey), [hostKey]);

  return useSyncExternalStore(
    subscribeHost,
    getHostSnapshot,
    getServerSnapshot,
  );
}
