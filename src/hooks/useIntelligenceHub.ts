import { useCallback, useEffect, useRef, useState } from "react";
import {
  FlowStatus,
  WritingModelDownloadId,
  IntelligenceTierState,
  RuntimeDownloadEvent,
  SystemMetrics,
} from "../types";
import { api, safeListen } from "../services/tauriApi";
import { useViewVisibility } from "./useViewVisibility";
import type { IntelligenceDownloadEvent } from "../App";

/** Shared writing-runtime state. Events provide immediate progress; versioned
 * backend snapshots recover missed events and reject stale poll results. */
export interface IntelligenceHub {
  flowStatus: FlowStatus | null;
  systemMetrics: SystemMetrics | null;
  intelligenceTiers: IntelligenceTierState[] | null;
  capabilitiesRefreshKey: number;
  intelligenceDownload: IntelligenceDownloadEvent | null;
  intelligenceDownloads?: Partial<Record<WritingModelDownloadId, IntelligenceDownloadEvent>>;
  activeDownloadTiers: Set<WritingModelDownloadId>;
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

const TERMINAL = new Set(["complete", "error", "cancelled"]);

function isInFlight(phase: string): boolean {
  return !TERMINAL.has(phase);
}

export function useIntelligenceHub(pollEnabled = true): IntelligenceHub {
  const visible = useViewVisibility();
  const [flowStatus, setFlowStatus] = useState<FlowStatus | null>(null);
  const [systemMetrics, setSystemMetrics] = useState<SystemMetrics | null>(null);
  const [intelligenceTiers, setIntelligenceTiers] = useState<IntelligenceTierState[] | null>(null);
  const [capabilitiesRefreshKey, setCapabilitiesRefreshKey] = useState(0);
  const [intelligenceDownload, setIntelligenceDownload] =
    useState<IntelligenceDownloadEvent | null>(null);
  const [intelligenceDownloads, setIntelligenceDownloads] = useState<
    Partial<Record<WritingModelDownloadId, IntelligenceDownloadEvent>>
  >({});
  const latestDownloads = useRef<
    Partial<Record<WritingModelDownloadId, IntelligenceDownloadEvent>>
  >({});
  const [activeDownloadTiers, setActiveDownloadTiers] = useState<Set<WritingModelDownloadId>>(
    () => new Set(),
  );
  const [runtimeDownload, setRuntimeDownload] = useState<RuntimeDownloadEvent | null>(null);
  const latestRuntime = useRef<RuntimeDownloadEvent | null>(null);
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

  const recordRuntime = useCallback((event: RuntimeDownloadEvent) => {
    const previous = latestRuntime.current;
    if (
      previous?.revision !== undefined &&
      event.revision !== undefined &&
      event.revision <= previous.revision
    )
      return false;
    latestRuntime.current = event;
    setRuntimeDownload(event);
    setRuntimeDownloadActive(isInFlight(event.phase));
    setRuntimeDownloadError(
      event.phase === "error" ? (event.error ?? "Runtime setup failed") : null,
    );
    return true;
  }, []);

  const recordDownload = useCallback((event: IntelligenceDownloadEvent) => {
    const previous = latestDownloads.current[event.tier];
    if (
      event.revision !== undefined &&
      previous?.revision !== undefined &&
      event.revision <= previous.revision
    )
      return false;
    if (
      event.revision === undefined &&
      previous &&
      previous.tier === event.tier &&
      previous.attempt_id === event.attempt_id &&
      isInFlight(previous.phase) &&
      isInFlight(event.phase) &&
      event.progress_pct < previous.progress_pct
    )
      return false;
    latestDownloads.current[event.tier] = event;
    setIntelligenceDownloads((current) => ({ ...current, [event.tier]: event }));
    setIntelligenceTiers(
      (current) =>
        current?.map((tier) =>
          tier.tier === event.tier
            ? {
                ...tier,
                download: event,
                downloading: isInFlight(event.phase),
                ...(event.phase === "complete" ? { installed: true, weights_installed: true } : {}),
              }
            : tier,
        ) ?? current,
    );
    setActiveDownloadTiers((current) => {
      const next = new Set(current);
      if (isInFlight(event.phase)) next.add(event.tier);
      else next.delete(event.tier);
      return next;
    });
    setIntelligenceDownload((current) =>
      current?.revision !== undefined &&
      event.revision !== undefined &&
      current.revision > event.revision
        ? current
        : event,
    );
    return true;
  }, []);

  const refresh = useCallback(async () => {
    if (refreshing.current) return refreshing.current;
    const request = (async () => {
      await Promise.allSettled([
        api.getIntelligenceStatus().then((status) => {
          if (mounted.current) {
            if (status.runtime_download) recordRuntime(status.runtime_download);
            setFlowStatus(status);
          }
        }),
        api.getSystemMetrics().then((metrics) => {
          if (mounted.current) setSystemMetrics(metrics);
        }),
        api.getIntelligenceTiers().then((tiers) => {
          if (mounted.current) {
            tiers.forEach((tier) => {
              if (tier.download) recordDownload(tier.download);
            });
            setIntelligenceTiers((previous) =>
              tiers.map((tier) => {
                const latest = latestDownloads.current[tier.tier];
                if (
                  latest?.revision !== undefined &&
                  tier.download?.revision !== undefined &&
                  latest.revision > tier.download.revision
                ) {
                  return (
                    previous?.find((current) => current.tier === tier.tier) ?? {
                      ...tier,
                      download: latest,
                      downloading: isInFlight(latest.phase),
                      ...(latest.phase === "complete"
                        ? { installed: true, weights_installed: true }
                        : {}),
                    }
                  );
                }
                return tier;
              }),
            );
          }
        }),
      ]);
    })();
    refreshing.current = request;
    try {
      await request;
    } finally {
      refreshing.current = null;
    }
  }, [recordDownload, recordRuntime]);

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
        if (recordDownload(event) && TERMINAL.has(event.phase)) {
          if (event.phase === "error")
            notifyToast("error", `Model setup failed: ${event.error ?? "Open Models to retry."}`);
          else if (event.phase === "complete")
            notifyToast("info", "Model download verified. Ready to use.");
          void refresh();
        }
      }),
      subscribe<RuntimeDownloadEvent>("runtime:download-progress", (event) => {
        if (!alive || !recordRuntime(event)) return;
        if (event.phase === "error") {
          setRuntimeDownloadError(event.error ?? "Runtime download failed");
          notifyToast(
            "error",
            event.error ?? "llama-server runtime download failed. Will retry next time.",
          );
        } else if (event.phase === "complete") {
          setRuntimeDownloadError(null);
          notifyToast("info", `llama-server ${event.kind_label ?? ""} runtime installed.`);
          void refresh();
        }
      }),
    ]);
    return () => {
      alive = false;
      unsubs.forEach((fn) => fn());
    };
  }, [refresh, notifyToast, recordDownload, recordRuntime]);

  // Backend snapshots carry event revisions, so polling can recover missed
  // completion events without replacing newer progress.
  useEffect(() => {
    if (!visible || !pollEnabled) {
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
  }, [pollEnabled, refresh, visible]);

  return {
    flowStatus,
    systemMetrics,
    intelligenceTiers,
    capabilitiesRefreshKey,
    intelligenceDownload,
    intelligenceDownloads,
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
