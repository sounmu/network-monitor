"use client";

import { useMemo, useSyncExternalStore } from "react";

/**
 * Module-level cached "wall clock tick" exposed to React via
 * useSyncExternalStore. Returns a monotonically increasing millisecond
 * timestamp that refreshes on a shared interval.
 *
 * Why a cached snapshot matters:
 *   useSyncExternalStore calls `getSnapshot` on every render and compares
 *   the result with the previous snapshot via Object.is. Passing
 *   `() => Date.now()` as getSnapshot returns a NEW value on almost
 *   every call (wall clock advances between two synchronous React
 *   renders), which convinces React the external store has changed and
 *   triggers another render — immediately causing
 *   "Maximum update depth exceeded" as the loop compounds.
 *
 *   This helper keeps a single `nowTickValue` that only changes on
 *   interval ticks, so consecutive getSnapshot calls return the same
 *   value and the loop is impossible.
 *
 * The interval is shared between every subscriber; it starts on first
 * subscribe and stops when the last listener unmounts.
 */

const DEFAULT_INTERVAL_MS = 15_000;

interface TickStore {
  value: number;
  listeners: Set<() => void>;
  intervalId: ReturnType<typeof setInterval> | null;
  intervalMs: number;
}

const stores = new Map<number, TickStore>();

function getStore(intervalMs: number): TickStore {
  let s = stores.get(intervalMs);
  if (!s) {
    s = { value: 0, listeners: new Set(), intervalId: null, intervalMs };
    stores.set(intervalMs, s);
  }
  return s;
}

function subscribe(store: TickStore) {
  return (cb: () => void) => {
    store.listeners.add(cb);
    if (store.listeners.size === 1) {
      store.value = Date.now();
      store.intervalId = setInterval(() => {
        store.value = Date.now();
        store.listeners.forEach((l) => l());
      }, store.intervalMs);
    }
    return () => {
      store.listeners.delete(cb);
      if (store.listeners.size === 0 && store.intervalId !== null) {
        clearInterval(store.intervalId);
        store.intervalId = null;
      }
    };
  };
}

function getSnapshot(store: TickStore) {
  return () => store.value;
}

function getServerSnapshot() {
  // SSR: return a deterministic value so hydration matches. Callers treat
  // 0 as "not yet hydrated".
  return 0;
}

export function useNowTick(intervalMs: number = DEFAULT_INTERVAL_MS): number {
  // Memoize subscribe / getSnapshot per intervalMs so React doesn't
  // tear down & re-attach the listener on every render.
  const [subscribeFn, snapshotFn] = useMemo(() => {
    const store = getStore(intervalMs);
    return [subscribe(store), getSnapshot(store)] as const;
  }, [intervalMs]);
  return useSyncExternalStore(subscribeFn, snapshotFn, getServerSnapshot);
}
