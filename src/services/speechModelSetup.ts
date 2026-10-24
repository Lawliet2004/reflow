import { useSyncExternalStore } from "react";
import { api } from "./tauriApi";
import { prepareSpeechRuntime } from "./speechSetup";
import type { PythonSetupProgress } from "../types";

type SetupState = {
  model: string;
  phase: "runtime" | "model" | "complete" | "error";
  progress?: PythonSetupProgress;
  error?: string;
};
let state: SetupState | null = null;
let pending: Promise<void> | null = null;
const listeners = new Set<() => void>();
function publish(next: SetupState) {
  state = next;
  listeners.forEach((listener) => listener());
}
export function useSpeechModelSetup() {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    () => state,
  );
}

/** Explicit user action only. Ownership survives closing the settings page. */
export function setupSpeechModel(model: string): Promise<void> {
  if (pending) {
    if (state?.model === model) return pending;
    return Promise.reject(
      new Error("Finish the current speech setup before choosing another model."),
    );
  }
  publish({ model, phase: "runtime" });
  pending = (async () => {
    try {
      const before = await api.getSettings();
      if (before.asr.model !== model)
        throw new Error("Speech model selection changed. Choose setup again.");
      if (before.asr.runtime === "python") {
        await prepareSpeechRuntime(new AbortController().signal, (progress) => {
          publish({ model, phase: "runtime", progress });
        });
      }
      const after = await api.getSettings();
      if (JSON.stringify(before.asr) !== JSON.stringify(after.asr)) {
        throw new Error("Speech settings changed during runtime setup. Choose setup again.");
      }
      publish({ model, phase: "model" });
      const status = await api.getModelStatus();
      if (status.installed) {
        if (!status.loaded) await api.reloadModel();
      } else {
        if (after.offline_mode)
          throw new Error("This model is not installed. Turn off offline mode to download it.");
        await api.installModel(model);
      }
      publish({ model, phase: "complete" });
    } catch (error) {
      publish({ model, phase: "error", error: String(error) });
      throw error;
    } finally {
      pending = null;
    }
  })();
  return pending;
}
