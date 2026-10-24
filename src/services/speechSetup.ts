import { api, safeListen } from "./tauriApi";
import { PythonSetupProgress } from "../types";

export function canCancelSpeechSetup(progress?: PythonSetupProgress | null): boolean {
  return Boolean(
    progress?.attempt_id &&
    !["activating", "complete", "error", "cancelled"].includes(progress.phase),
  );
}

/** Native revisions order events and snapshots across setup attempts. */
export function newestSpeechSetupProgress(
  current?: PythonSetupProgress | null,
  next?: PythonSetupProgress | null,
): PythonSetupProgress | null {
  if (!next) return current ?? null;
  if (
    current?.revision !== undefined &&
    (next.revision === undefined || next.revision <= current.revision)
  )
    return current;
  return next;
}

/** Wait for the background installer before attempting a model download. */
export async function prepareSpeechRuntime(
  signal: AbortSignal,
  onProgress: (progress: PythonSetupProgress) => void,
  failureKind?: string | null,
  joinAttempt?: string,
): Promise<void> {
  const status = joinAttempt ? null : await api.getPythonRuntimeStatus();
  signal.throwIfAborted();
  const missing = failureKind === "python_missing" || failureKind === "missing_dependency";
  const changed = () => new Error("That speech setup attempt has ended or changed. Check again.");
  if (status?.overridden) {
    try {
      await api.verifySpeechRuntime();
      signal.throwIfAborted();
    } catch (failure) {
      signal.throwIfAborted();
      throw new Error(
        `Your selected Python environment could not run speech dependencies. Clear Python interpreter in Model settings to use automatic setup, or repair that environment. ${String(failure)}`,
      );
    }
    return;
  }
  if (status?.managed_installed && !status.setting_up && !missing) {
    await api.verifySpeechRuntime();
    signal.throwIfAborted();
    return;
  }

  if (!joinAttempt && !status?.setting_up) {
    if (await api.activateCachedSpeechRuntime()) {
      signal.throwIfAborted();
      await api.verifySpeechRuntime();
      signal.throwIfAborted();
      return;
    }
  }

  let unsubscribe: (() => void) | undefined;
  let subscribed = false;
  let abort: (() => void) | undefined;
  let finished = false;
  let expectedAttempt = joinAttempt;
  let attached = false;
  let lastProgress: PythonSetupProgress | null = null;
  let pollTimer: ReturnType<typeof setTimeout> | undefined;
  const buffered: PythonSetupProgress[] = [];
  try {
    await new Promise<void>((resolve, reject) => {
      abort = () => reject(new DOMException("Setup stopped", "AbortError"));
      signal.addEventListener("abort", abort, { once: true });
      if (signal.aborted) {
        abort();
        return;
      }
      const receive = (progress: PythonSetupProgress) => {
        if (signal.aborted || finished) return;
        if (progress.attempt_id && !attached) {
          buffered.push(progress);
          if (buffered.length > 16) buffered.shift();
          return;
        }
        if (expectedAttempt && progress.attempt_id !== expectedAttempt) return;
        const newest = newestSpeechSetupProgress(lastProgress, progress);
        if (newest === lastProgress) return;
        lastProgress = newest;
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
      };
      const poll = async () => {
        if (finished || signal.aborted) return;
        let snapshot: PythonSetupProgress | null;
        try {
          snapshot = await api.getPythonSetupProgress();
        } catch {
          // An IPC interruption does not end a still-owned backend job.
          if (!finished && !signal.aborted) pollTimer = setTimeout(poll, 1000);
          return;
        }
        if (finished || signal.aborted) return;
        // A reply captured before a newer event cannot revoke that observation.
        const stale =
          snapshot && newestSpeechSetupProgress(lastProgress, snapshot) === lastProgress;
        if (!stale && expectedAttempt && snapshot?.attempt_id !== expectedAttempt) {
          finished = true;
          reject(changed());
          return;
        }
        if (!stale && snapshot && expectedAttempt) receive(snapshot);
        if (!finished) pollTimer = setTimeout(poll, 1000);
      };
      void (async () => {
        try {
          unsubscribe = await safeListen<PythonSetupProgress>(
            "python-runtime:setup-progress",
            receive,
          );
          subscribed = true;
        } catch {
          // The owned snapshot can recover even if event subscription failed.
        }
        if (signal.aborted) {
          unsubscribe?.();
          return;
        }
        if (joinAttempt) {
          // Reopening observes this exact job, including a terminal snapshot.
          // It cannot turn a disappeared/failed job into a fresh installation.
          attached = true;
          buffered.forEach(receive);
          if (!finished) void poll();
          return;
        }
        if (!status) throw changed();
        // Subscribe, then read the current snapshot: completion cannot be
        // lost while a window is opening or an IPC reply is in flight.
        let current = await api.getPythonRuntimeStatus();
        signal.throwIfAborted();
        if (finished) return;
        if (
          status.setting_up &&
          status.progress?.attempt_id &&
          current.progress?.attempt_id === status.progress.attempt_id &&
          ["complete", "error", "cancelled"].includes(current.progress.phase)
        ) {
          expectedAttempt = current.progress.attempt_id;
          attached = true;
          receive(current.progress);
          return;
        }
        if (current.managed_installed && !current.setting_up && !missing) {
          resolve();
          return;
        }
        if (!current.setting_up) {
          onProgress({ phase: "downloading" });
          try {
            await api.setupPythonRuntime();
            if (!finished) current = await api.getPythonRuntimeStatus();
          } catch (failure) {
            current = await api.getPythonRuntimeStatus();
            signal.throwIfAborted();
            if (!current.setting_up && !current.managed_installed && !finished) throw failure;
          }
        }
        signal.throwIfAborted();
        expectedAttempt = current.progress?.attempt_id;
        if (!expectedAttempt && !subscribed)
          throw new Error("Setup progress is unavailable. Check again.");
        attached = true;
        if (current.progress) receive(current.progress);
        else onProgress({ phase: "downloading" });
        buffered.forEach(receive);
        if (expectedAttempt && !finished && !signal.aborted) pollTimer = setTimeout(poll, 1000);
      })().catch(reject);
    });
    if (expectedAttempt) {
      const verified = await api.getPythonRuntimeStatus();
      signal.throwIfAborted();
      if (!verified.managed_installed || verified.setting_up || verified.overridden) {
        throw new Error(
          "Speech settings or dependencies changed during setup. Check again before downloading a model.",
        );
      }
    }
    await api.verifySpeechRuntime();
    signal.throwIfAborted();
  } finally {
    finished = true;
    unsubscribe?.();
    if (pollTimer) clearTimeout(pollTimer);
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
  activating: "Activating the speech runtime",
  complete: "Speech dependencies ready",
  error: "Speech setup failed",
  cancelled: "Setup cancelled",
};
