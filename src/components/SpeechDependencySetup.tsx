import { useCallback, useEffect, useRef, useState } from "react";
import { Check, Download, Loader2 } from "lucide-react";
import { AppSettings, PythonRuntimeStatus, PythonSetupProgress, SpeechSetupPlan } from "../types";
import { createEventScope } from "../services/eventScope";
import { api } from "../services/tauriApi";
import { prepareSpeechRuntime, SPEECH_SETUP_PHASE } from "../services/speechSetup";

interface Props {
  settings: AppSettings;
  failureKind?: string | null;
  disabled?: boolean;
  onUpdateSettings: (changes: Partial<AppSettings>) => Promise<boolean> | void;
  onBusyChange?: (busy: boolean) => void;
  onPlan?: (plan: SpeechSetupPlan | null) => void;
  onReady?: () => void;
}

const gb = (bytes: number) => `${(bytes / 1e9).toFixed(1)} GB`;

/** A dependency-only path: users can finish this before choosing a speech model. */
export function SpeechDependencySetup({
  settings,
  failureKind,
  disabled,
  onUpdateSettings,
  onBusyChange,
  onPlan,
  onReady,
}: Props) {
  const [plan, setPlan] = useState<SpeechSetupPlan | null>(null);
  const [runtime, setRuntime] = useState<PythonRuntimeStatus | null>(null);
  const [checking, setChecking] = useState(true);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<PythonSetupProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const controller = useRef<AbortController | null>(null);
  const alive = useRef(true);
  const notifyPlan = useRef(onPlan);
  useEffect(() => {
    notifyPlan.current = onPlan;
  }, [onPlan]);

  const refresh = useCallback(async () => {
    await Promise.resolve();
    if (!alive.current) return;
    setChecking(true);
    setError(null);
    try {
      const [nextPlan, status] = await Promise.all([
        api.getSpeechSetupPlan(),
        api.getPythonRuntimeStatus(),
      ]);
      if (!alive.current) return;
      setPlan(nextPlan);
      setRuntime(status);
      notifyPlan.current?.(nextPlan);
    } catch (failure) {
      if (alive.current) setError(`Could not check this computer: ${String(failure)}`);
    } finally {
      if (alive.current) setChecking(false);
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    void Promise.resolve().then(refresh);
    return () => {
      alive.current = false;
      controller.current?.abort();
    };
  }, [refresh]);

  useEffect(() => {
    const scope = createEventScope();
    void scope.listen<PythonSetupProgress>("python-runtime:setup-progress", (event) => {
      if (["complete", "error", "cancelled"].includes(event.phase)) {
        void api
          .getPythonRuntimeStatus()
          .then((status) => {
            if (alive.current) setRuntime(status);
          })
          .catch(() => {});
      }
    });
    return () => scope.dispose();
  }, []);

  const start = async () => {
    if (controller.current || !plan || plan.blocker) return;
    const attempt = new AbortController();
    controller.current = attempt;
    setBusy(true);
    onBusyChange?.(true);
    setError(null);
    try {
      if (settings.offline_mode && (await onUpdateSettings({ offline_mode: false })) === false) {
        throw new Error("Downloads could not be enabled. Try again.");
      }
      attempt.signal.throwIfAborted();
      const latest = await api.getSpeechSetupPlan();
      attempt.signal.throwIfAborted();
      if (!latest) throw new Error("Open Reflow’s desktop app to install dependencies.");
      setPlan(latest);
      notifyPlan.current?.(latest);
      if (latest.blocker) throw new Error(latest.blocker);
      await prepareSpeechRuntime(attempt.signal, setProgress, failureKind);
      attempt.signal.throwIfAborted();
      const status = await api.getPythonRuntimeStatus();
      attempt.signal.throwIfAborted();
      setRuntime(status);
      if (status.managed_installed) onReady?.();
    } catch (failure) {
      if (alive.current) {
        setError(
          failure instanceof DOMException && failure.name === "AbortError"
            ? "Setup cancelled. You can retry when you’re ready."
            : `Could not install dependencies: ${failure instanceof Error ? failure.message : String(failure)}`,
        );
      }
    } finally {
      controller.current = null;
      if (alive.current) {
        setBusy(false);
        onBusyChange?.(false);
      }
    }
  };

  const needsRepair = failureKind === "python_missing" || failureKind === "missing_dependency";
  const installed = runtime?.managed_installed && (!needsRepair || progress?.phase === "complete");

  return (
    <div className="space-y-3" aria-busy={checking || busy}>
      <h3 className="text-sm font-semibold text-ink">Your computer and speech dependencies</h3>
      {checking ? (
        <p role="status" className="text-sm text-muted">
          Checking this computer…
        </p>
      ) : plan ? (
        <>
          <dl className="text-xs divide-y divide-line-soft rounded-control bg-well px-3">
            <div className="py-2">
              <dt className="text-muted">System</dt>
              <dd className="text-ink-2 break-words">
                {plan.hardware.os_name} · {plan.architecture}
              </dd>
            </div>
            <div className="py-2">
              <dt className="text-muted">Processor</dt>
              <dd className="text-ink-2 break-words">
                {plan.hardware.cpu.model} · {plan.hardware.cpu.logical_cores} threads
              </dd>
            </div>
            <div className="py-2">
              <dt className="text-muted">Memory</dt>
              <dd className="text-ink-2">
                {gb(plan.hardware.ram.total_mb * 1_048_576)} total ·{" "}
                {gb(plan.hardware.ram.available_mb * 1_048_576)} available
              </dd>
            </div>
            <div className="py-2">
              <dt className="text-muted">Graphics</dt>
              <dd className="text-ink-2 break-words">
                {plan.hardware.gpus.map((gpu) => gpu.name).join(", ") ||
                  "No graphics card detected"}
              </dd>
            </div>
          </dl>
          <p className="text-sm text-ink-2">{plan.acceleration} speech setup</p>
          <p className="text-xs text-muted leading-relaxed">{plan.explanation}</p>
          {plan.cpu_has_avx2 === false && (
            <p className="text-xs text-muted">
              This processor lacks AVX2 instructions. Try the small Zipformer model first; larger
              models may be slow or unsupported.
            </p>
          )}
          {!installed && (
            <p className="text-xs text-muted leading-relaxed">
              Includes Python, a private environment and speech libraries. Allow about{" "}
              {gb(plan.required_disk_bytes * 1.1)} of free storage, plus your model.
              {plan.available_disk_bytes !== null &&
                ` ${gb(plan.available_disk_bytes)} available.`}{" "}
              Download size varies. Setup may take several minutes. No terminal commands or
              environment activation needed.
            </p>
          )}
          {plan.blocker && !installed && (
            <p role="alert" className="text-sm text-danger">
              {plan.blocker}
            </p>
          )}
        </>
      ) : (
        !error && (
          <p className="text-xs text-muted">
            System checks and dependency downloads are available in the desktop app.
          </p>
        )
      )}
      {error && (
        <p role="alert" className="text-sm text-danger whitespace-pre-line break-words">
          {error}
        </p>
      )}
      {busy ? (
        <div className="space-y-2">
          <p role="status" className="text-sm text-accent flex items-center gap-2">
            <Loader2 className="w-4 h-4 animate-spin" aria-hidden />
            {progress ? SPEECH_SETUP_PHASE[progress.phase] : "Preparing setup"}
            {typeof progress?.progress_pct === "number" ? ` · ${progress.progress_pct}%` : "…"}
          </p>
          {progress?.detail?.startsWith("Downloading a speech package:") ? (
            <p className="text-xs text-muted tabular-nums">{progress.detail}</p>
          ) : progress?.detail ? (
            <details className="text-xs text-muted">
              <summary className="cursor-pointer">Installation details</summary>
              <p className="mt-2 break-words whitespace-pre-wrap">{progress.detail}</p>
            </details>
          ) : null}
          <button
            type="button"
            className="btn btn-secondary"
            onClick={() => {
              void api
                .cancelPythonRuntimeSetup()
                .catch((failure) => setError(`Could not cancel setup: ${String(failure)}`));
            }}
          >
            Cancel dependency setup
          </button>
        </div>
      ) : runtime?.overridden ? (
        <p className="text-xs text-muted">
          Using your selected Python environment. Clear Python interpreter in Model settings to use
          automatic dependency setup.
        </p>
      ) : installed ? (
        <p role="status" className="text-sm text-success flex items-center gap-2">
          <Check className="w-4 h-4" aria-hidden />
          Dependencies ready. Choose a speech model below.
        </p>
      ) : (
        <button
          type="button"
          className="btn btn-primary"
          disabled={disabled || checking || !plan || Boolean(plan.blocker)}
          onClick={() => {
            void start();
          }}
        >
          <Download className="w-4 h-4" aria-hidden />
          {error
            ? "Retry dependency setup"
            : settings.offline_mode
              ? "Allow downloads and install dependencies"
              : "Download all dependencies"}
        </button>
      )}
      {!busy && (
        <button
          type="button"
          className="btn btn-ghost"
          disabled={disabled || checking}
          onClick={() => {
            void refresh();
          }}
        >
          Check again
        </button>
      )}
    </div>
  );
}
