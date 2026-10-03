import { useCallback, useEffect, useRef, useState } from "react";
import {
  FlowStatus,
  IntelligenceTier,
  IntelligenceTierState,
  RuntimeDownloadEvent,
  SystemMetrics,
} from "../types";
import { api, safeListen } from "../services/tauriApi";
import type { IntelligenceDownloadEvent } from "../App";

/**
 * Single source of truth for intelligence state.
 *
 * Replaces three competing sources that used to race: the 2s
 * `getIntelligenceStatus` poll in `useIntelligenceStatus`, the
 * `intelligence:download-progress` events in `App.tsx`, and the per-page
 * polling loops in `ModelPage` (2s) and `CleanupPage` (1.5s). Event pushes
 * update download state immediately; polling only runs when no download is in
 * flight (the backend reports `installed: false` until the file is fully
 * written, so polling mid-download would clobber the UI and resurrect the
 * Download button).
 */
export interface IntelligenceHub {
  flowStatus: FlowStatus | null;
  systemMetrics: SystemMetrics | null;
  intelligenceTiers: IntelligenceTierState[] | null;
  capabilitiesRefreshKey: number;
  intelligenceDownload: IntelligenceDownloadEvent | null;
  activeDownloadTiers: Set<IntelligenceTier>;
  runtimeDownload: RuntimeDownloadEvent | null;
  runtimeDownloadActive: boolean;
  runtimeDownloadError: string | null;
  /** Force a status refresh (e.g. after install/remove settles). */
  refresh: () => Promise<void>;
  notifyToast: (kind: "error" | "info", text: string) => void;
  lastToast: { kind: "error" | "info"; text: string } | null;
  /** Bump to re-run hardware capability probes in consumers. */
  refreshCapabilities: () => void;
}

const TERMINAL = new Set(["complete", "error"]);

function isInFlight(phase: string): boolean {
  return !TERMINAL.has(phase);
}

export function useIntelligenceHub(): IntelligenceHub {
  const [flowStatus, setFlowStatus] = useState<FlowStatus | null>(null);
  const [systemMetrics, setSystemMetrics] = useState<SystemMetrics | null>(null);
  const [intelligenceTiers, setIntelligenceTiers] = useState<IntelligenceTierState[] | null>(null);
  const [capabilitiesRefreshKey, setCapabilitiesRefreshKey] = useState(0);
  const [intelligenceDownload, setIntelligenceDownload] =
    useState<IntelligenceDownloadEvent | null>(null);
  const [activeDownloadTiers, setActiveDownloadTiers] = useState<Set<IntelligenceTier>>(
    () => new Set(),
  );
  const [runtimeDownload, setRuntimeDownload] = useState<RuntimeDownloadEvent | null>(null);
  const [runtimeDownloadActive, setRuntimeDownloadActive] = useState(false);
  const [runtimeDownloadError, setRuntimeDownloadError] = useState<string | null>(null);
  const [lastToast, setLastToast] = useState<{ kind: "error" | "info"; text: string } | null>(null);
  const pollTimer = useRef<ReturnType<typeof setInterval> | null>(null);
  const refreshing = useRef<Promise<void> | null>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const notifyToast = useCallback((kind: "error" | "info", text: string) => {
    setLastToast({ kind, text });
  }, []);

  const refresh = useCallback(async () => {
    if (refreshing.current) return refreshing.current;
    const request = (async () => {
      await Promise.allSettled([
        api.getIntelligenceStatus().then((status) => {
          if (mounted.current) setFlowStatus(status);
        }),
        api.getSystemMetrics().then((metrics) => {
          if (mounted.current) setSystemMetrics(metrics);
        }),
        api.getIntelligenceTiers().then((tiers) => {
          if (mounted.current) setIntelligenceTiers(tiers);
        }),
      ]);
    })();
    refreshing.current = request;
    try {
      await request;
    } finally {
      refreshing.current = null;
    }
  }, []);

  const refreshCapabilities = useCallback(() => {
    setCapabilitiesRefreshKey((k) => k + 1);
    void refresh();
  }, [refresh]);

  // Event pushes: authoritative for download progress, no polling involved.
  useEffect(() => {
    const unsubs: (() => void)[] = [];
    let alive = true;
    const track = (fn: () => void) => {
      if (alive) unsubs.push(fn);
      else fn();
    };
    const subscribe = async <T>(event: string, handler: (payload: T) => void) => {
      try {
        track(await safeListen<T>(event, handler));
      } catch (error) {
        if (alive) notifyToast("error", `Could not subscribe to ${event}. ${String(error)}`);
      }
    };
    void Promise.all([
      subscribe<IntelligenceDownloadEvent>("intelligence:download-progress", (event) => {
        if (!alive) return;
        setActiveDownloadTiers((prev) => {
          const next = new Set(prev);
          if (isInFlight(event.phase)) next.add(event.tier);
          else next.delete(event.tier);
          return next;
        });
        // Monotonic guard: ignore events that would move the bar backwards
        // mid-download; terminal phases always win.
        setIntelligenceDownload((prev) => {
          if (
            prev &&
            prev.tier === event.tier &&
            isInFlight(prev.phase) &&
            isInFlight(event.phase) &&
            event.progress_pct < prev.progress_pct
          ) {
            return prev;
          }
          return event;
        });
        if (TERMINAL.has(event.phase)) {
          // The file just landed on disk; re-read installed flags once.
          void refresh();
        }
      }),
      subscribe<RuntimeDownloadEvent>("runtime:download-progress", (event) => {
        if (!alive) return;
        const inFlight =
          event.phase === "starting" ||
          event.phase === "downloading" ||
          event.phase === "verifying" ||
          event.phase === "extracting";
        setRuntimeDownloadActive(inFlight);
        setRuntimeDownload(event);
        if (event.phase === "error") {
          setRuntimeDownloadError(event.error ?? "Runtime download failed");
          setLastToast({
            kind: "error",
            text: event.error ?? "llama-server runtime download failed. Will retry next time.",
          });
        } else if (event.phase === "complete") {
          setRuntimeDownloadError(null);
          setLastToast({
            kind: "info",
            text: `llama-server ${event.kind_label ?? ""} runtime installed.`,
          });
          void refresh();
        }
      }),
    ]);
    return () => {
      alive = false;
      unsubs.forEach((fn) => fn());
    };
  }, [refresh, notifyToast]);

  // Polling fallback: runs only when no download is in flight, so it can never
  // clobber event-driven progress or resurrect Download buttons mid-write.
  const anyActive = activeDownloadTiers.size > 0 || runtimeDownloadActive;
  useEffect(() => {
    if (anyActive) {
      if (pollTimer.current) {
        clearInterval(pollTimer.current);
        pollTimer.current = null;
      }
      return;
    }
    void refresh();
    pollTimer.current = setInterval(() => {
      void refresh();
    }, 2000);
    return () => {
      if (pollTimer.current) {
        clearInterval(pollTimer.current);
        pollTimer.current = null;
      }
    };
  }, [anyActive, refresh]);

  return {
    flowStatus,
    systemMetrics,
    intelligenceTiers,
    capabilitiesRefreshKey,
    intelligenceDownload,
    activeDownloadTiers,
    runtimeDownload,
    runtimeDownloadActive,
    runtimeDownloadError,
    refresh,
    notifyToast,
    lastToast,
    refreshCapabilities,
  };
}
