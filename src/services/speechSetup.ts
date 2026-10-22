import { api, safeListen } from "./tauriApi";
import { PythonSetupProgress } from "../types";

/** Wait for the background installer before attempting a model download. */
export async function prepareSpeechRuntime(
  signal: AbortSignal,
  onProgress: (progress: PythonSetupProgress) => void,
  failureKind?: string | null,
): Promise<void> {
  const status = await api.getPythonRuntimeStatus();
  signal.throwIfAborted();
  const missing = failureKind === "python_missing" || failureKind === "missing_dependency";
  if (status.overridden) {
    if (missing) {
      throw new Error(
        "Your selected Python environment is missing speech dependencies. Clear Python interpreter in Model settings to use automatic setup, or repair that environment.",
      );
    }
    return;
  }
  if (status.managed_installed && !status.setting_up && !missing) return;

  let unsubscribe: (() => void) | undefined;
  let abort: (() => void) | undefined;
  let finished = false;
  try {
    await new Promise<void>((resolve, reject) => {
      abort = () => reject(new DOMException("Setup stopped", "AbortError"));
      signal.addEventListener("abort", abort, { once: true });
      if (signal.aborted) {
        abort();
        return;
      }
      void (async () => {
        unsubscribe = await safeListen<PythonSetupProgress>(
          "python-runtime:setup-progress",
          (progress) => {
            if (signal.aborted || finished) return;
            onProgress(progress);
            if (progress.phase === "complete") {
              finished = true;
              resolve();
            } else if (progress.phase === "error") {
              finished = true;
              reject(new Error(progress.error || "Speech runtime setup failed. Try again."));
            } else if (progress.phase === "cancelled") {
              finished = true;
              reject(new DOMException("Setup cancelled", "AbortError"));
            }
          },
        );
        // A registration can finish after the onboarding screen closes.
        if (signal.aborted) {
          unsubscribe();
          return;
        }
        // Recheck after subscribing so an existing installation cannot finish
        // between the initial status read and the listener registration.
        const current = await api.getPythonRuntimeStatus();
        signal.throwIfAborted();
        if (finished) return;
        if (current.managed_installed && !current.setting_up && !missing) {
          resolve();
          return;
        }
        onProgress({ phase: "downloading" });
        if (!current.setting_up) {
          try {
            await api.setupPythonRuntime();
          } catch (failure) {
            // Another setup surface may win the start race. Join its events.
            const latest = await api.getPythonRuntimeStatus();
            signal.throwIfAborted();
            if (!latest.setting_up && !finished) throw failure;
          }
        }
      })().catch(reject);
    });
  } finally {
    finished = true;
    unsubscribe?.();
    if (abort) signal.removeEventListener("abort", abort);
  }
}

export const SPEECH_SETUP_PHASE: Record<PythonSetupProgress["phase"], string> = {
  downloading: "Downloading Python",
  extracting: "Extracting Python",
  creating_env: "Preparing the speech environment",
  installing_torch: "Installing speech dependencies",
  installing_packages: "Installing speech packages",
  verifying: "Checking the speech runtime",
  complete: "Speech dependencies ready",
  error: "Speech setup failed",
  cancelled: "Setup cancelled",
};
