import React, { useCallback, useEffect, useState } from "react";
import { Download, Loader2, Trash2 } from "lucide-react";
import { AppSettings, ModelStatus, PythonRuntimeStatus, PythonSetupProgress } from "../types";
import { api } from "../services/tauriApi";
import { createEventScope } from "../services/eventScope";
import {
  canCancelSpeechSetup,
  newestSpeechSetupProgress,
  SPEECH_SETUP_PHASE,
} from "../services/speechSetup";

const IN_FLIGHT = new Set([
  "downloading",
  "extracting",
  "creating_env",
  "installing_torch",
  "installing_packages",
  "verifying",
  "activating",
]);

function formatBytes(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
  if (bytes >= 1_000_000) return `${Math.round(bytes / 1_000_000)} MB`;
  return `${Math.round(bytes / 1_000)} KB`;
}

interface PythonRuntimeSetupProps {
  settings: AppSettings;
  /** Needed to detect python_missing / missing_dependency failures. */
  modelStatus?: ModelStatus | null;
  onUpdateSettings: (changes: Partial<AppSettings>) => Promise<boolean> | void;
  /** Show the "Remove speech runtime" action when a managed install exists. */
  showRemove?: boolean;
}

/**
 * One-click managed speech runtime: downloads a pinned CPython, creates the
 * `python-runtime` venv the sidecar discovers automatically, and installs the
 * speech dependencies into it. Renders nothing when a working interpreter is
 * already configured and nothing failed.
 */
export const PythonRuntimeSetup: React.FC<PythonRuntimeSetupProps> = ({
  settings,
  modelStatus,
  onUpdateSettings,
  showRemove = false,
}) => {
  const [status, setStatus] = useState<PythonRuntimeStatus | null>(null);
  const [eventProgress, setProgress] = useState<PythonSetupProgress | null>(null);
  const [removing, setRemoving] = useState(false);

  const refresh = useCallback(() => {
    return api
      .getPythonRuntimeStatus()
      .then(setStatus)
      .catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh, modelStatus?.failure_kind]);

  useEffect(() => {
    const scope = createEventScope();
    void scope.listen<PythonSetupProgress>("python-runtime:setup-progress", (payload) => {
      setProgress((current) => newestSpeechSetupProgress(current, payload));
      if (
        payload.phase === "complete" ||
        payload.phase === "error" ||
        payload.phase === "cancelled"
      ) {
        refresh();
      }
    });
    return () => scope.dispose();
  }, [refresh]);

  const settingUp = status?.setting_up;
  const snapshotAttempt = status?.progress?.attempt_id;
  useEffect(() => {
    if (!settingUp || !snapshotAttempt) return;
    let attached = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const snapshot = await api.getPythonSetupProgress();
        if (!attached) return;
        setProgress((current) => newestSpeechSetupProgress(current, snapshot));
        if (
          snapshot?.attempt_id !== snapshotAttempt ||
          ["complete", "error", "cancelled"].includes(snapshot.phase)
        )
          await refresh();
      } catch {
        // A temporary IPC failure does not start a replacement installer.
      }
      if (attached) timer = setTimeout(poll, 1000);
    };
    timer = setTimeout(poll, 1000);
    return () => {
      attached = false;
      clearTimeout(timer);
    };
  }, [settingUp, snapshotAttempt, refresh]);

  if (!status) return null;
  // Local request/error state has no owner yet; native observations are ordered separately.
  const progress =
    eventProgress && !eventProgress.attempt_id
      ? eventProgress
      : newestSpeechSetupProgress(status.progress, eventProgress);

  const failureKind = modelStatus?.failure_kind;
  const needsSetup =
    !status.overridden &&
    (failureKind === "python_missing" ||
      failureKind === "missing_dependency" ||
      !status.managed_installed);
  const inFlight = progress ? IN_FLIGHT.has(progress.phase) : status.setting_up;
  const failed = progress?.phase === "error";

  const startSetup = async () => {
    setProgress({ phase: "downloading" });
    try {
      await api.setupPythonRuntime();
      await refresh();
      setProgress((current) => (current?.attempt_id ? current : null));
    } catch (error) {
      setProgress({ phase: "error", error: String(error) });
    }
  };

  const allowDownloads = async () => {
    if ((await onUpdateSettings({ offline_mode: false })) !== false) void startSetup();
  };

  const removeRuntime = async () => {
    setRemoving(true);
    try {
      await api.removePythonRuntime();
      refresh();
    } catch {
      // surfaced via status refresh; nothing else to show
    } finally {
      setRemoving(false);
    }
  };

  if (
    status.overridden &&
    (failureKind === "python_missing" || failureKind === "missing_dependency")
  ) {
    return (
      <div className="rounded-xl border border-line bg-base-2 px-3 py-2.5">
        <p className="text-xs text-muted leading-snug">
          Using your own Python
          {settings.asr.python_path
            ? ` at ${settings.asr.python_path}`
            : " (REFLOW_PYTHON or a saved interpreter path)"}
          . Managed setup is skipped while an interpreter is configured — clear it in Model settings
          to use the managed runtime.
        </p>
      </div>
    );
  }

  if (failed) {
    return (
      <div className="rounded-xl border border-line bg-base-2 px-3 py-2.5 space-y-2">
        <p role="alert" className="text-sm text-danger">
          Speech runtime setup failed: {progress?.error}
        </p>
        <button type="button" className="btn btn-secondary" onClick={startSetup}>
          Retry
        </button>
      </div>
    );
  }

  if (inFlight) {
    const phase = progress?.phase ?? "downloading";
    return (
      <div className="rounded-xl border border-line bg-base-2 px-3 py-2.5 space-y-2" role="status">
        <div className="flex items-center justify-between gap-3">
          <p className="text-sm text-ink flex items-center gap-2">
            <Loader2 className="w-4 h-4 animate-spin" />
            {SPEECH_SETUP_PHASE[phase]}…
          </p>
          <button
            type="button"
            className="btn btn-secondary btn-sm shrink-0"
            disabled={!canCancelSpeechSetup(progress)}
            onClick={() => {
              api.cancelPythonRuntimeSetup(progress?.attempt_id).catch(() => {});
            }}
          >
            Cancel
          </button>
        </div>
        {phase === "downloading" && typeof progress?.progress_pct === "number" && (
          <div className="h-1.5 rounded-full bg-line overflow-hidden">
            <div
              className="progress-fill bg-accent"
              style={{ transform: `scaleX(${progress.progress_pct / 100})` }}
            />
          </div>
        )}
        {progress?.detail && (
          <p className="text-2xs text-muted font-mono truncate">{progress.detail}</p>
        )}
      </div>
    );
  }

  if (status.managed_installed && !needsSetup) {
    return (
      <div className="rounded-xl border border-line bg-base-2 px-3 py-2.5 space-y-2">
        <p className="text-xs text-muted leading-snug">
          Managed speech runtime installed
          {status.receipt
            ? ` (Python ${status.receipt.python_version} · torch ${status.receipt.torch_variant})`
            : ""}
          .
        </p>
        {showRemove && (
          <button
            type="button"
            className="btn btn-secondary"
            disabled={removing}
            onClick={removeRuntime}
          >
            <Trash2 className="w-4 h-4" />
            {removing
              ? "Removing…"
              : `Remove speech runtime (frees ${formatBytes(status.size_bytes)})`}
          </button>
        )}
      </div>
    );
  }

  if (!needsSetup) return null;

  if (settings.offline_mode) {
    return (
      <div className="rounded-xl border border-line bg-base-2 px-3 py-2.5 space-y-2">
        <p className="text-xs text-muted leading-snug">
          Offline mode is on. Setup can use a complete runtime pack saved on this computer. If it is
          missing or damaged, allow downloads to replace it.
        </p>
        <div className="flex flex-wrap gap-2">
          <button type="button" className="btn btn-primary" onClick={startSetup}>
            Try cached dependencies
          </button>
          <button type="button" className="btn btn-secondary" onClick={allowDownloads}>
            <Download className="w-4 h-4" />
            Allow downloads
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="rounded-xl border border-line bg-base-2 px-3 py-2.5 space-y-2">
      <p className="text-xs text-muted leading-snug">
        {status.torch_variant === "none"
          ? "Downloads Python and the Zipformer speech packages (under 1 GB) into Reflow's folder. One time. Qwen and Phonon models need Apple Silicon on macOS — Zipformer 20M works on this Mac."
          : "Downloads Python and the speech packages (allow 5.5 GB free storage, or 15.4 GB with compatible NVIDIA acceleration) into Reflow's folder. One time."}
      </p>
      <button type="button" className="btn btn-primary" onClick={startSetup}>
        <Download className="w-4 h-4" />
        Set up speech runtime
      </button>
    </div>
  );
};
