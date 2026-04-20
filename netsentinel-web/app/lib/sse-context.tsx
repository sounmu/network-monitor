"use client";

import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { HostMetricsPayload, HostStatusPayload } from "@/app/types/metrics";
import { issueSseTicket } from "@/app/lib/api";
import { useAuth } from "@/app/auth/AuthContext";

// ──────────────────────────────────────────
// Context type definitions
// ──────────────────────────────────────────

interface SSEContextValue {
  /** host_key -> latest metrics payload (updated every 10s) */
  metricsMap: Record<string, HostMetricsPayload>;
  /** host_key -> latest status payload (updated on initial connection + on change) */
  statusMap: Record<string, HostStatusPayload>;
  /** EventSource connection state */
  isConnected: boolean;
  /** Pre-computed host list derived from statusMap */
  hostList: HostStatusPayload[];
  /** Number of online hosts */
  onlineCount: number;
  /** Number of offline hosts */
  offlineCount: number;
  /**
   * Purge a host from both live maps after deletion.
   * Without this the statusMap entry lingers forever and a "ghost" row keeps
   * rendering on the Overview until a full page reload.
   */
  removeHost: (hostKey: string) => void;
}

const SSEContext = createContext<SSEContextValue>({
  metricsMap: {},
  statusMap: {},
  isConnected: false,
  hostList: [],
  onlineCount: 0,
  offlineCount: 0,
  removeHost: () => {},
});

// ──────────────────────────────────────────
// Reconnection settings
// ──────────────────────────────────────────

const INITIAL_RETRY_MS = 1000;
const MAX_RETRY_MS = 30000;

// ──────────────────────────────────────────
// Provider
// ──────────────────────────────────────────

export function SSEProvider({ children }: { children: React.ReactNode }) {
  const { user } = useAuth();
  const [metricsMap, setMetricsMap] = useState<
    Record<string, HostMetricsPayload>
  >({});
  const [statusMap, setStatusMap] = useState<
    Record<string, HostStatusPayload>
  >({});
  const [isConnected, setIsConnected] = useState(false);
  const esRef = useRef<EventSource | null>(null);

  // ── Batched SSE update buffers ──
  // Accumulate SSE events in refs, then flush once per animation frame.
  // With 100 hosts this reduces 100+ setState calls per scrape cycle to 1.
  const metricsBufRef = useRef<Record<string, HostMetricsPayload>>({});
  const statusBufRef = useRef<Record<string, HostStatusPayload>>({});
  const offlineKeysBufRef = useRef<Set<string>>(new Set());
  const rafRef = useRef<number | null>(null);

  const flushBuffers = useCallback(() => {
    rafRef.current = null;

    const metricsBuf = metricsBufRef.current;
    const statusBuf = statusBufRef.current;
    const offlineKeys = offlineKeysBufRef.current;

    const hasMetrics = Object.keys(metricsBuf).length > 0;
    const hasStatus = Object.keys(statusBuf).length > 0;
    const hasOffline = offlineKeys.size > 0;

    if (hasMetrics || hasOffline) {
      setMetricsMap((prev) => {
        // Only remove offline keys that actually exist in the map — avoids
        // creating a new object reference when there's nothing to change.
        const keysToRemove = hasOffline
          ? [...offlineKeys].filter((k) => k in prev)
          : [];
        if (!hasMetrics && keysToRemove.length === 0) return prev;

        const next = hasMetrics ? { ...prev, ...metricsBuf } : { ...prev };
        for (const key of keysToRemove) {
          delete next[key];
        }
        return next;
      });
    }

    if (hasStatus) {
      setStatusMap((prev) => ({ ...prev, ...statusBuf }));
    }

    metricsBufRef.current = {};
    statusBufRef.current = {};
    offlineKeysBufRef.current = new Set();
  }, []);

  const scheduleFlush = useCallback(() => {
    if (rafRef.current === null) {
      rafRef.current = requestAnimationFrame(flushBuffers);
    }
  }, [flushBuffers]);

  const removeHost = useCallback((hostKey: string) => {
    // Drop the host from both user-facing maps and any pending buffered events
    // so a late-arriving SSE frame for the doomed host cannot resurrect it.
    delete metricsBufRef.current[hostKey];
    delete statusBufRef.current[hostKey];
    offlineKeysBufRef.current.delete(hostKey);
    setMetricsMap((prev) => {
      if (!(hostKey in prev)) return prev;
      const next = { ...prev };
      delete next[hostKey];
      return next;
    });
    setStatusMap((prev) => {
      if (!(hostKey in prev)) return prev;
      const next = { ...prev };
      delete next[hostKey];
      return next;
    });
  }, []);

  useEffect(() => {
    // Only connect when authenticated
    if (!user) return;

    let retryMs = INITIAL_RETRY_MS;
    let retryTimer: ReturnType<typeof setTimeout> | null = null;
    let unmounted = false;

    // Ticket-first reconnection: a fresh single-use SSE ticket is issued on
    // every (re)connect. Tickets are atomic/one-shot, so re-using the previous
    // one on EventSource's internal retry would always fail — we must loop
    // through our own handler.
    async function connect() {
      if (unmounted) return;

      // Clean up existing connection if any
      if (esRef.current) {
        esRef.current.close();
        esRef.current = null;
      }

      let ticket: string;
      try {
        const res = await issueSseTicket();
        ticket = res.ticket;
      } catch {
        // `apiCall` already redirects to /login on 401 (stale / rotated JWT).
        // Any other failure — network flap, server restarting — is transient
        // and gets the same exponential backoff as a dropped SSE stream.
        if (unmounted) return;
        setIsConnected(false);
        const delay = retryMs;
        retryMs = Math.min(retryMs * 2, MAX_RETRY_MS);
        retryTimer = setTimeout(() => {
          void connect();
        }, delay);
        return;
      }

      if (unmounted) return;

      const apiBase = process.env.NEXT_PUBLIC_API_URL ?? "";
      // The ticket is opaque, short-lived, and single-use — safe to carry as
      // a query parameter. Never put the long-lived JWT on the URL.
      const url = `${apiBase}/api/stream?key=${encodeURIComponent(ticket)}`;

      const es = new EventSource(url);
      esRef.current = es;

      es.onopen = () => {
        setIsConnected(true);
        retryMs = INITIAL_RETRY_MS; // Reset backoff on successful connection
      };

      es.onerror = () => {
        setIsConnected(false);
        es.close();
        esRef.current = null;

        // Exponential backoff reconnection — each attempt mints a new ticket.
        if (!unmounted) {
          const delay = retryMs;
          retryMs = Math.min(retryMs * 2, MAX_RETRY_MS);
          retryTimer = setTimeout(() => {
            void connect();
          }, delay);
        }
      };

      // event: metrics — dynamic data (CPU, memory, network speed)
      // Buffered: accumulated in ref, flushed once per animation frame
      es.addEventListener("metrics", (e: MessageEvent) => {
        try {
          const payload: HostMetricsPayload = JSON.parse(e.data);
          metricsBufRef.current[payload.host_key] = payload;
          scheduleFlush();
        } catch {
          // Ignore parse errors
        }
      });

      // event: status — static data (Docker, port status)
      // Buffered: accumulated in ref, flushed once per animation frame
      es.addEventListener("status", (e: MessageEvent) => {
        try {
          const payload: HostStatusPayload = JSON.parse(e.data);
          statusBufRef.current[payload.host_key] = payload;
          if (!payload.is_online) {
            offlineKeysBufRef.current.add(payload.host_key);
          }
          scheduleFlush();
        } catch {
          // Ignore parse errors
        }
      });
    }

    void connect();

    // Must close on component unmount — prevent memory leaks
    return () => {
      unmounted = true;
      if (retryTimer) clearTimeout(retryTimer);
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
      if (esRef.current) {
        esRef.current.close();
        esRef.current = null;
      }
    };
  }, [user, scheduleFlush]); // Reconnect when auth state changes

  // ── Pre-computed derived state (avoids duplicate O(n) in page + sidebar) ──
  const { hostList, onlineCount, offlineCount } = useMemo(() => {
    const list = Object.values(statusMap);
    let online = 0;
    let offline = 0;
    for (const h of list) {
      if (h.is_online) online++;
      else offline++;
    }
    return { hostList: list, onlineCount: online, offlineCount: offline };
  }, [statusMap]);

  const contextValue = useMemo(
    () => ({
      metricsMap,
      statusMap,
      isConnected,
      hostList,
      onlineCount,
      offlineCount,
      removeHost,
    }),
    [metricsMap, statusMap, isConnected, hostList, onlineCount, offlineCount, removeHost],
  );

  return (
    <SSEContext.Provider value={contextValue}>
      {children}
    </SSEContext.Provider>
  );
}

// ──────────────────────────────────────────
// Hook
// ──────────────────────────────────────────

/** Custom hook to access SSE data */
export function useSSE(): SSEContextValue {
  return useContext(SSEContext);
}
