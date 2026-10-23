import { useCallback, useEffect, useRef, useState } from "react";
import { Check, Download, Loader2 } from "lucide-react";
import { AppSettings, PythonRuntimeStatus, PythonSetupProgress, SpeechSetupPlan } from "../types";
import { createEventScope } from "../services/eventScope";
import { api } from "../services/tauriApi";
import {
  canCancelSpeechSetup,
  newestSpeechSetupProgress,
  prepareSpeechRuntime,
  SPEECH_SETUP_PHASE,
} from "../services/speechSetup";

interface Props {
  settings: AppSettings;
  failureKind?: string | null;
  disabled?: boolean;
  onUpdateSettings: (changes: Partial<AppSettings>) => Promise<boolean> | void;
  onBusyChange?: (busy: boolean) => void;
  onPlan?: (plan: SpeechSetupPlan | null) => void;
  onReady?: () => void;
  inspectionOnly?: boolean;
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
  inspectionOnly = false,
}: Props) {
  const [plan, setPlan] = useState<SpeechSetupPlan | null>(null);
  const [runtime, setRuntime] = useState<PythonRuntimeStatus | null>(null);
  const [checking, setChecking] = useState(true);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<PythonSetupProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [verified, setVerified] = useState(false);
  const refreshId = useRef({ value: 0 });
  const controller = useRef<AbortController | null>(null);
  const observedProgress = useRef<PythonSetupProgress | null>(null);
  const alive = useRef(true);
  const notifyPlan = useRef(onPlan);
  const notifyReady = useRef(onReady);
  useEffect(() => {
    notifyPlan.current = onPlan;
    notifyReady.current = onReady;
  }, [onPlan, onReady]);

  const rememberAttempt = useCallback((value?: PythonSetupProgress | null) => {
    const next = newestSpeechSetupProgress(observedProgress.current, value);
    const first = Boolean(
      next?.attempt_id && next.attempt_id !== observedProgress.current?.attempt_id,
    );
    observedProgress.current = next;
    return first;
  }, []);
  const owns = useCallback(
    (attempt: AbortController, id: number) =>
      alive.current && controller.current === attempt && refreshId.current.value === id,
    [],
  );
  const updateProgress = useCallback(
    (attempt: AbortController, id: number, value: PythonSetupProgress) => {
      if (!owns(attempt, id)) return;
      if (newestSpeechSetupProgress(observedProgress.current, value) !== value) return;
      rememberAttempt(value);
      setProgress(value);
    },
    [owns, rememberAttempt],
  );
  const confirmReady = useCallback(
    async (attempt: AbortController, id: number) => {
      attempt.signal.throwIfAborted();
      const status = await api.getPythonRuntimeStatus();
      attempt.signal.throwIfAborted();
      if (!owns(attempt, id)) return;
      if (status.setting_up || (!status.managed_installed && !status.overridden))
        throw new Error("Speech settings or setup changed during verification. Check again.");
      setRuntime(status);
      rememberAttempt(status.progress);
      setVerified(true);
      notifyReady.current?.();
    },
    [owns, rememberAttempt],
  );

  const refresh = useCallback(async () => {
    if (controller.current) return;
    const id = ++refreshId.current.value;
    let attempt: AbortController | undefined;
    await Promise.resolve();
    if (!alive.current || id !== refreshId.current.value) return;
    setChecking(true);
    setBusy(false);
    setVerified(false);
    setError(null);
    notifyPlan.current?.(null);
    try {
      const [nextPlan, status] = await Promise.all([
        api.getSpeechSetupPlan(),
        inspectionOnly ? Promise.resolve(null) : api.getPythonRuntimeStatus(),
      ]);
      if (!alive.current || id !== refreshId.current.value) return;
      setPlan(nextPlan);
      setRuntime(status);
      rememberAttempt(status?.progress);
      setProgress(observedProgress.current);
      notifyPlan.current?.(nextPlan);
      const observed = observedProgress.current;
      const settingUp =
        status?.setting_up ||
        Boolean(
          observed?.attempt_id && !["complete", "error", "cancelled"].includes(observed.phase),
        );
      if (
        !inspectionOnly &&
        status &&
        (settingUp || status.overridden || status.managed_installed)
      ) {
        attempt = new AbortController();
        controller.current = attempt;
        if (settingUp) {
          const joining = attempt;
          const attemptId = observed?.attempt_id ?? status.progress?.attempt_id;
          if (!attemptId) throw new Error("Setup progress is unavailable. Check again.");
          setBusy(true);
          setChecking(false);
          await prepareSpeechRuntime(
            joining.signal,
            (value) => updateProgress(joining, id, value),
            null,
            attemptId,
          );
        } else {
          // Passive readiness checks never install a missing environment.
          await api.verifySpeechRuntime();
        }
        await confirmReady(attempt, id);
      } else if (status?.progress?.phase === "error") {
        setError(status.progress.error || "Speech setup failed. Retry dependency setup.");
      } else if (status?.progress?.phase === "cancelled") {
        setError("Setup cancelled. You can retry when you’re ready.");
      }
    } catch (failure) {
      if (alive.current && id === refreshId.current.value)
        setError(
          failure instanceof DOMException && failure.name === "AbortError"
            ? "Setup cancelled. You can retry when you’re ready."
            : `Could not verify speech dependencies: ${String(failure)}`,
        );
    } finally {
      if (attempt && controller.current === attempt) controller.current = null;
      if (alive.current && id === refreshId.current.value) {
        setBusy(false);
        setChecking(false);
      }
    }
  }, [inspectionOnly, confirmReady, rememberAttempt, updateProgress]);

  useEffect(() => {
    alive.current = true;
    const generation = refreshId.current;
    void Promise.resolve().then(refresh);
    return () => {
      alive.current = false;
      generation.value++;
      const previous = controller.current;
      controller.current = null;
      previous?.abort();
    };
  }, [refresh, settings.asr.model, settings.asr.device, settings.asr.python_path]);

  useEffect(() => {
    onBusyChange?.(busy || (!inspectionOnly && checking));
  }, [busy, checking, inspectionOnly, onBusyChange]);

  useEffect(() => {
    if (inspectionOnly) return;
    const scope = createEventScope();
    void scope.listen<PythonSetupProgress>("python-runtime:setup-progress", (event) => {
      if (!alive.current || controller.current) return;
      if (newestSpeechSetupProgress(observedProgress.current, event) === observedProgress.current)
        return;
      const terminal = ["complete", "error", "cancelled"].includes(event.phase);
      if (
        terminal &&
        observedProgress.current &&
        event.attempt_id === observedProgress.current?.attempt_id &&
        ["complete", "error", "cancelled"].includes(observedProgress.current.phase)
      )
        return;
      const first = rememberAttempt(event);
      if (terminal || first) void refresh();
    });
    return () => scope.dispose();
  }, [inspectionOnly, refresh, rememberAttempt]);

  const start = async (allowDownloads = false) => {
    if (controller.current || !plan || plan.blocker) return;
    const attempt = new AbortController();
    const id = ++refreshId.current.value;
    controller.current = attempt;
    setBusy(true);
    setProgress(null);
    setVerified(false);
    setError(null);
    try {
      if (
        allowDownloads &&
        settings.offline_mode &&
        (await onUpdateSettings({ offline_mode: false })) === false
      ) {
        throw new Error("Downloads could not be enabled. Try again.");
      }
      attempt.signal.throwIfAborted();
      const latest = await api.getSpeechSetupPlan();
      attempt.signal.throwIfAborted();
      if (!latest) throw new Error("Open Reflow’s desktop app to install dependencies.");
      setPlan(latest);
      notifyPlan.current?.(latest);
      if (latest.blocker) throw new Error(latest.blocker);
      await prepareSpeechRuntime(
        attempt.signal,
        (value) => updateProgress(attempt, id, value),
        runtime?.managed_installed && !verified ? "missing_dependency" : failureKind,
      );
      await confirmReady(attempt, id);
    } catch (failure) {
      if (owns(attempt, id)) {
        setError(
          failure instanceof DOMException && failure.name === "AbortError"
            ? "Setup cancelled. You can retry when you’re ready."
            : `Could not install dependencies: ${failure instanceof Error ? failure.message : String(failure)}`,
        );
      }
    } finally {
      if (controller.current === attempt) controller.current = null;
      if (alive.current && id === refreshId.current.value) {
        setBusy(false);
      }
    }
  };

  const installed = verified;

  return (
    <div className="space-y-3" aria-busy={checking || busy}>
      <h3 className="text-sm font-semibold text-ink">
        {inspectionOnly ? "Your computer" : "Your computer and speech dependencies"}
      </h3>
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
          {!inspectionOnly && (
            <p className="text-sm text-ink-2">{plan.acceleration} speech setup</p>
          )}
          {!inspectionOnly && (
            <p className="text-xs text-muted leading-relaxed">{plan.explanation}</p>
          )}
          {!inspectionOnly && plan.cpu_has_avx2 === false && (
            <p className="text-xs text-muted">
              This processor lacks AVX2 instructions. Try the small Zipformer model first; larger
              models may be slow or unsupported.
            </p>
          )}
          {!inspectionOnly && !installed && (
            <p className="text-xs text-muted leading-relaxed">
              Includes Python, a private environment and speech libraries. Allow about{" "}
              {gb(plan.required_disk_bytes * 1.1)} of free storage, plus your model.
              {plan.available_disk_bytes !== null &&
                ` ${gb(plan.available_disk_bytes)} available.`}{" "}
              Download size varies. Setup may take several minutes. No terminal commands or
              environment activation needed.
            </p>
          )}
          {!inspectionOnly && plan.blocker && !installed && (
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
      {!inspectionOnly &&
        (busy ? (
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
              disabled={!canCancelSpeechSetup(progress)}
              onClick={() => {
                void api
                  .cancelPythonRuntimeSetup(progress?.attempt_id)
                  .catch((failure) => setError(`Could not cancel setup: ${String(failure)}`));
              }}
            >
              Cancel dependency setup
            </button>
          </div>
        ) : installed ? (
          <p role="status" className="text-sm text-success flex items-center gap-2">
            <Check className="w-4 h-4" aria-hidden />
            Dependencies ready.
          </p>
        ) : runtime?.overridden ? (
          <p className="text-xs text-muted">
            Using your selected Python environment. Clear Python interpreter in Model settings to
            use automatic dependency setup.
          </p>
        ) : (
          <div className="space-y-2">
            {settings.offline_mode && (
              <p className="text-xs text-muted leading-relaxed">
                Offline mode is on. Setup can use a complete runtime pack saved on this computer. If
                it is missing or damaged, allow downloads to replace it.
              </p>
            )}
            <div className="flex flex-wrap gap-2">
              <button
                type="button"
                className="btn btn-primary"
                disabled={disabled || checking || !plan || Boolean(plan.blocker)}
                onClick={() => {
                  void start();
                }}
              >
                {settings.offline_mode ? (
                  "Try cached dependencies"
                ) : (
                  <>
                    <Download className="w-4 h-4" aria-hidden />
                    {error ? "Retry dependency setup" : "Download all dependencies"}
                  </>
                )}
              </button>
              {settings.offline_mode && (
                <button
                  type="button"
                  className="btn btn-secondary"
                  disabled={disabled || checking || !plan || Boolean(plan.blocker)}
                  onClick={() => {
                    void start(true);
                  }}
                >
                  <Download className="w-4 h-4" aria-hidden />
                  Allow downloads and install dependencies
                </button>
              )}
            </div>
          </div>
        ))}
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
