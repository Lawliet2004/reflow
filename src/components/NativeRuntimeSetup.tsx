import { useEffect, useRef, useState } from "react";
import { api, safeListen } from "../services/tauriApi";
import { AppSettings, RuntimeDownloadEvent } from "../types";

/** Native ASR shares the same verified llama runtime as optional writing. */
export function NativeRuntimeSetup({
  settings,
  onReady,
  onBusyChange,
  onUpdateSettings,
}: {
  settings: AppSettings;
  onReady: () => void;
  onBusyChange: (busy: boolean) => void;
  onUpdateSettings: (changes: Partial<AppSettings>) => Promise<boolean> | void;
}) {
  const [busy, setBusy] = useState(false);
  const [checking, setChecking] = useState(true);
  const [download, setDownload] = useState<RuntimeDownloadEvent | null>(null);
  const latest = useRef<RuntimeDownloadEvent | null>(null);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [phase, setPhase] = useState("Checking runtime…");
  const alive = useRef(true);
  const verifying = useRef(false);
  const readyCallback = useRef(onReady);
  useEffect(() => {
    readyCallback.current = onReady;
  }, [onReady]);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const verify = async (quiet = false) => {
    if (verifying.current) return;
    verifying.current = true;
    try {
      const generation = await api.verifyLlamaRuntime();
      if (!generation) throw new Error("Runtime verification requires the desktop app.");
      if (alive.current) {
        setReady(true);
        setError(null);
        setBusy(false);
        readyCallback.current();
        onBusyChange(false);
      }
    } catch (failure) {
      if (alive.current && !quiet) {
        setError(String(failure));
        setBusy(false);
        onBusyChange(false);
      }
    } finally {
      verifying.current = false;
    }
  };
  const receive = (event: RuntimeDownloadEvent) => {
    if (
      !alive.current ||
      (latest.current?.revision !== undefined &&
        event.revision !== undefined &&
        event.revision <= latest.current.revision)
    )
      return;
    latest.current = event;
    setDownload(event);
    setPhase(
      event.phase === "complete" ? "Verifying runtime…" : `${event.phase} · ${event.progress_pct}%`,
    );
    if (event.phase === "complete") void verify();
    else if (event.phase === "error" || event.phase === "cancelled") {
      setError(
        event.error ||
          (event.phase === "cancelled"
            ? "Runtime setup cancelled. Retry when ready."
            : "Runtime setup failed. Retry setup."),
      );
      setBusy(false);
      onBusyChange(false);
    } else {
      setBusy(true);
      onBusyChange(true);
    }
  };
  useEffect(() => {
    void Promise.resolve()
      .then(async () => {
        const snapshot = await api.getRuntimeDownloadStatus();
        if (snapshot && !["complete", "error", "cancelled"].includes(snapshot.phase))
          receive(snapshot);
        else await verify(true);
      })
      .catch((failure) => {
        if (alive.current) setError(String(failure));
      })
      .finally(() => {
        if (alive.current) setChecking(false);
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    if (!busy) return;
    let closed = false;
    let unsubscribe: (() => void) | undefined;
    void safeListen<RuntimeDownloadEvent>("runtime:download-progress", (event) => {
      if (!closed) receive(event);
    })
      .then((unsub) => {
        if (closed) unsub();
        else unsubscribe = unsub;
      })
      .catch((failure) => {
        if (!closed) setError(String(failure));
      });
    const timer = setInterval(() => {
      void api
        .getRuntimeDownloadStatus()
        .then((snapshot) => {
          if (closed) return;
          if (snapshot) receive(snapshot);
          if (snapshot?.phase === "complete") void verify();
        })
        .catch((failure) => {
          if (!closed) setError(`Could not refresh runtime setup: ${String(failure)}`);
        });
    }, 2_000);
    return () => {
      closed = true;
      clearInterval(timer);
      unsubscribe?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [busy]);
  const install = async () => {
    setBusy(true);
    onBusyChange(true);
    setError(null);
    setPhase("Preparing runtime…");
    try {
      if (settings.offline_mode && (await onUpdateSettings({ offline_mode: false })) === false)
        throw new Error("Downloads could not be enabled. Try again.");
      await api.installLlamaRuntime("auto");
    } catch (failure) {
      if (alive.current) {
        setError(String(failure));
        setBusy(false);
        onBusyChange(false);
      }
    }
  };
  return (
    <div className="space-y-3">
      <p className="text-sm text-muted">
        Native speech uses the shared local runtime. Setup checks its files and starts a short
        health check before model downloads.
      </p>
      {ready ? (
        <p role="status" className="text-sm text-success">
          Native speech runtime verified.
        </p>
      ) : checking ? (
        <p role="status" className="text-sm text-muted">
          Checking runtime…
        </p>
      ) : busy ? (
        <div className="space-y-2">
          <p role="status" className="text-sm text-accent">
            {phase}
          </p>
          {download?.attempt_id && download.phase !== "activating" && (
            <button
              type="button"
              className="btn btn-secondary"
              disabled={download.phase === "cancelling"}
              onClick={() => {
                void api
                  .cancelRuntimeDownload(download.attempt_id!)
                  .catch((failure) => setError(String(failure)));
              }}
            >
              Cancel runtime setup
            </button>
          )}
        </div>
      ) : (
        <button
          type="button"
          className="btn btn-primary"
          onClick={() => {
            void install();
          }}
        >
          {settings.offline_mode ? "Allow downloads and install runtime" : "Install speech runtime"}
        </button>
      )}
      {error && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
    </div>
  );
}
