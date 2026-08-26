import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  AppState,
  AppSettings,
  IntelligenceTier,
  LatencyMetrics,
  ModelStatus,
  RuntimeDownloadEvent,
  StreamingTranscriptPayload,
  isModelReady,
  normalizeSettings,
} from "./types";
import { api, safeListen, isTauri } from "./services/tauriApi";
import { Navigation, NavTab } from "./components/Navigation";
import { TitleBar } from "./components/TitleBar";
import { DictateHome } from "./components/DictateHome";
import { HistoryView } from "./components/HistoryView";
import { SettingsView } from "./components/SettingsView";
import { Overlay } from "./components/Overlay";
import { Onboarding } from "./components/Onboarding";
import { applyTheme } from "./utils/theme";

const EMPTY_TRANSCRIPT: StreamingTranscriptPayload = {
  committed_prefix: "",
  mutable_suffix: "",
  full_text: "",
  language: "en",
  audio_level: 0.0,
  stage: "",
};

export interface IntelligenceDownloadEvent {
  tier: IntelligenceTier;
  progress_pct: number;
  speed_mbps: number;
  phase: "starting" | "downloading" | "complete" | "error";
  error?: string;
  filename?: string;
}

export const App: React.FC = () => {
  const [activeTab, setActiveTab] = useState<NavTab>("dictate");
  const [appState, setAppState] = useState<AppState>("READY");
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [transcript, setTranscript] = useState<StreamingTranscriptPayload>(EMPTY_TRANSCRIPT);
  const [modelStatus, setModelStatus] = useState<ModelStatus | null>(null);
  const [latencyMetrics, setLatencyMetrics] = useState<LatencyMetrics | null>(null);
  const [onboardingComplete, setOnboardingComplete] = useState(false);
  const [wizardOpen, setWizardOpen] = useState(false);
  const [toast, setToast] = useState<{ kind: "error" | "info"; text: string } | null>(null);
  const [intelligenceDownload, setIntelligenceDownload] =
    useState<IntelligenceDownloadEvent | null>(null);
  // Tiers that currently have a download in flight. The 2-second status poll
  // reports `is_downloading: false` until the file is fully written, so we
  // need a separate "download is active" signal to keep the Download button
  // hidden and to suspend the poll while a download is running.
  const [activeDownloadTiers, setActiveDownloadTiers] = useState<Set<IntelligenceTier>>(
    () => new Set()
  );
  // Runtime (llama-server) download state. There is only one runtime, so a
  // single event + a single boolean (for "in flight") is enough.
  const [runtimeDownload, setRuntimeDownload] = useState<RuntimeDownloadEvent | null>(
    null
  );
  const [runtimeDownloadActive, setRuntimeDownloadActive] = useState(false);
  const [runtimeDownloadError, setRuntimeDownloadError] = useState<string | null>(null);

  const settingsTokenRef = useRef(0);

  useEffect(() => {
    if (settings) {
      return applyTheme(settings);
    }
  }, [
    settings?.app_theme,
    settings?.accent_color,
    settings?.reduce_motion,
    settings?.ui_font_scale,
    settings?.overlay_theme,
  ]);

  useEffect(() => {
    const init = async () => {
      try {
        const [st, cfg, model] = await Promise.all([
          api.getAppState(),
          api.getSettings(),
          api.getModelStatus(),
        ]);
        setAppState(st);
        setSettings(normalizeSettings(cfg));
        setModelStatus(model);
        if (!model.installed && !isModelReady(model)) {
          setWizardOpen(true);
        }
      } catch (err) {
        console.error("Initial load failed:", err);
      }
    };
    init();
  }, []);

  // Live events: model status, app state, transcript streaming, audio level.
  useEffect(() => {
    const unsubs: (() => void)[] = [];
    const setup = async () => {
      unsubs.push(
        await safeListen<ModelStatus>("model:status", (status) => setModelStatus(status))
      );
      unsubs.push(
        await safeListen<AppState>("app:state-changed", (st) => setAppState(st))
      );
      unsubs.push(
        await safeListen<number>("recording:audio-level", (lvl) =>
          setTranscript((prev) => ({ ...prev, audio_level: lvl }))
        )
      );
      unsubs.push(
        await safeListen<StreamingTranscriptPayload>("transcript:partial", (payload) =>
          setTranscript(payload)
        )
      );
      unsubs.push(
        await safeListen<StreamingTranscriptPayload>("transcript:final", (payload) => {
          setTranscript(payload);
          api.getLatencyMetrics().then(setLatencyMetrics).catch(() => {});
        })
      );
      unsubs.push(
        await safeListen<AppSettings>("settings:changed", (cfg) => {
          setSettings(normalizeSettings(cfg));
        })
      );
      unsubs.push(
        await safeListen<IntelligenceDownloadEvent>(
          "intelligence:download-progress",
          (event) => {
            // Track which tiers currently have a download in flight.
            setActiveDownloadTiers((prev) => {
              const next = new Set(prev);
              if (event.phase === "starting" || event.phase === "downloading") {
                next.add(event.tier);
              } else if (event.phase === "complete" || event.phase === "error") {
                next.delete(event.tier);
              }
              return next;
            });
            // Monotonic guard: ignore events that would move the bar backwards
            // while a download for this tier is already in progress, unless
            // the new event is `complete` or `error` (those represent a final
            // state, not progress).
            setIntelligenceDownload((prev) => {
              if (
                prev &&
                prev.tier === event.tier &&
                (prev.phase === "starting" || prev.phase === "downloading") &&
                (event.phase === "starting" || event.phase === "downloading") &&
                event.progress_pct < prev.progress_pct
              ) {
                return prev;
              }
              return event;
            });
          }
        )
      );
      unsubs.push(
        await safeListen<RuntimeDownloadEvent>(
          "runtime:download-progress",
          (event) => {
            // The runtime downloader is single-shot, so a single boolean
            // tracks "in flight" cleanly.
            const isInFlight =
              event.phase === "starting" ||
              event.phase === "downloading" ||
              event.phase === "verifying" ||
              event.phase === "extracting";
            const isTerminal =
              event.phase === "complete" || event.phase === "error";
            setRuntimeDownloadActive(isInFlight);
            setRuntimeDownload(event);
            if (event.phase === "error") {
              setRuntimeDownloadError(event.error ?? "Runtime download failed");
              setToast({
                kind: "error",
                text:
                  event.error ??
                  "llama-server runtime download failed. Will retry next time.",
              });
            } else if (event.phase === "complete") {
              setRuntimeDownloadError(null);
              setToast({
                kind: "info",
                text: `llama-server ${event.kind_label ?? ""} runtime installed.`,
              });
            }
            // Touch isTerminal to keep the linter quiet; the boolean
            // is the only state we actually need.
            void isTerminal;
          }
        )
      );
    };
    setup();
    return () => unsubs.forEach((fn) => fn());
  }, []);

  const handleUpdateSettings = useCallback(async (partial: Partial<AppSettings>) => {
    // Optimistic update; guard against stale responses overwriting newer ones.
    const token = ++settingsTokenRef.current;
    setSettings((prev) => (prev ? normalizeSettings({ ...prev, ...partial }) : prev));
    try {
      const updated = await api.updateSettings(partial);
      if (token === settingsTokenRef.current) {
        setSettings(normalizeSettings(updated));
      }
    } catch (e) {
      console.error("Failed to update settings:", e);
      if (token === settingsTokenRef.current) {
        // Rollback to last known server state.
        try {
          const fresh = await api.getSettings();
          if (token === settingsTokenRef.current) {
            setSettings(normalizeSettings(fresh));
          }
        } catch {
          /* leave optimistic state */
        }
        setToast({ kind: "error", text: "Could not save settings. Reverted." });
      }
    }
  }, []);

  const handleStartRecording = async () => {
    setTranscript({ ...EMPTY_TRANSCRIPT });
    setAppState("RECORDING");
    try {
      await api.startRecording();
    } catch (e) {
      console.error("Start recording failed:", e);
      setAppState("READY");
      setToast({ kind: "error", text: "Could not start dictation." });
    }
  };

  const handleStopRecording = async () => {
    // The transcript:final event will deliver the result; only flip state.
    setAppState("PROCESSING");
    try {
      await api.stopRecording();
    } catch (e) {
      console.error("Stop recording failed:", e);
      setAppState("READY");
      setToast({ kind: "error", text: "Transcription failed." });
    }
  };

  const handleInjectText = async (text: string) => {
    try {
      const ok = await api.injectText(text);
      if (!ok) {
        setToast({ kind: "info", text: "Copied to clipboard — press Ctrl+V to paste." });
      }
    } catch (e) {
      console.error("Inject failed:", e);
      setToast({ kind: "error", text: "Could not inject text." });
    }
  };

  // Auto-dismiss toast.
  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 3000);
    return () => clearTimeout(t);
  }, [toast]);

  if (!settings) {
    return (
      <div className="h-screen w-screen flex flex-col items-center justify-center bg-base text-ink gap-3">
        <div className="w-8 h-8 rounded-full border-2 border-accent border-t-transparent animate-spin" />
        <p className="text-[13px] text-muted font-medium tracking-wide">Starting Reflow…</p>
      </div>
    );
  }

  const showOnboarding = wizardOpen && !onboardingComplete;

  return (
    <div className="flex flex-col h-screen w-screen bg-base text-ink overflow-hidden font-sans select-none">
      <TitleBar appState={appState} />

      <div className="flex flex-1 min-h-0 overflow-hidden">
        <Navigation
          activeTab={activeTab}
          setActiveTab={setActiveTab}
          appState={appState}
          modelStatus={modelStatus}
        />

        <main
          className={`flex-1 min-w-0 ${
            activeTab === "settings" ? "overflow-hidden flex" : "overflow-y-auto select-text"
          }`}
        >
          {activeTab === "dictate" &&
            (showOnboarding ? (
              <Onboarding
                settings={settings}
                modelStatus={modelStatus}
                onUpdateSettings={handleUpdateSettings}
                onComplete={() => {
                  setOnboardingComplete(true);
                  setWizardOpen(false);
                }}
              />
            ) : (
              <DictateHome
                appState={appState}
                settings={settings}
                modelStatus={modelStatus}
                transcript={transcript}
                latencyMetrics={latencyMetrics}
                onStartRecording={handleStartRecording}
                onStopRecording={handleStopRecording}
                onUpdateSettings={handleUpdateSettings}
                onOpenHistory={() => setActiveTab("history")}
              />
            ))}

          {activeTab === "history" && <HistoryView onInjectText={handleInjectText} />}

          {activeTab === "settings" && (
            <SettingsView
              settings={settings}
              onUpdateSettings={handleUpdateSettings}
              modelStatus={modelStatus}
              onReloadModel={() => api.reloadModel()}
              intelligenceDownload={intelligenceDownload}
              activeDownloadTiers={activeDownloadTiers}
              runtimeDownload={runtimeDownload}
              runtimeDownloadActive={runtimeDownloadActive}
              runtimeDownloadError={runtimeDownloadError}
              onInstallRuntime={async () => {
                try {
                  await api.installLlamaRuntime(
                    settings?.compute_backend ?? "auto"
                  );
                } catch (e) {
                  console.error("installLlamaRuntime failed:", e);
                  setToast({
                    kind: "error",
                    text: `Could not start runtime install: ${String(e)}`,
                  });
                }
              }}
              onRemoveRuntime={async () => {
                try {
                  await api.removeLlamaRuntime();
                  setToast({
                    kind: "info",
                    text: "llama-server runtime removed.",
                  });
                } catch (e) {
                  console.error("removeLlamaRuntime failed:", e);
                  setToast({
                    kind: "error",
                    text: `Could not remove runtime: ${String(e)}`,
                  });
                }
              }}
            />
          )}
        </main>
      </div>

      {toast && (
        <div
          className={`fixed bottom-5 left-1/2 -translate-x-1/2 z-50 px-4 py-2.5 rounded-xl shadow-pop text-[12.5px] font-medium ${
            toast.kind === "error"
              ? "bg-rose-600 text-white"
              : "bg-slate-900 text-white"
          }`}
          role="status"
        >
          {toast.text}
        </div>
      )}

      {!isTauri() && <Overlay appState={appState} transcript={transcript} />}
    </div>
  );
};
export default App;
