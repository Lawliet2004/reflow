import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  AppState,
  AppSettings,
  IntelligenceTier,
  LatencyMetrics,
  LatencyPercentiles,
  ModelStatus,
  StreamingTranscriptPayload,
  isModelReady,
  normalizeSettings,
} from "./types";
import { api, safeListen, isTauri } from "./services/tauriApi";
import { useIntelligenceHub } from "./hooks/useIntelligenceHub";
import { Navigation, NavTab } from "./components/Navigation";
import { TitleBar } from "./components/TitleBar";
import { DictateHome } from "./components/DictateHome";
import { BackendStage } from "./components/hud/stages";
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
  const [startupError, setStartupError] = useState(false);
  const [startupAttempt, setStartupAttempt] = useState(0);
  const [transcript, setTranscript] = useState<StreamingTranscriptPayload>(EMPTY_TRANSCRIPT);
  const [modelStatus, setModelStatus] = useState<ModelStatus | null>(null);
  // Latest backend pipeline stage (`DoryEvent::Stage`, forwarded as
  // `pipeline:stage`). Cleared on each new recording so a stale stage from the
  // previous dictation never leaks into the next one.
  const [backendStage, setBackendStage] = useState<BackendStage | null>(null);
  const [latencyMetrics, setLatencyMetrics] = useState<LatencyMetrics | null>(null);
  const [latencyPercentiles, setLatencyPercentiles] = useState<LatencyPercentiles | null>(null);
  const [onboardingComplete, setOnboardingComplete] = useState(false);
  const [wizardOpen, setWizardOpen] = useState(false);
  const [toast, setToast] = useState<{ kind: "error" | "info"; text: string } | null>(null);
  // Single intelligence hub: download progress arrives as events, status via
  // polling only when idle. Replaces the old inline listeners + per-page polls.
  const intelligence = useIntelligenceHub();

  const settingsTokenRef = useRef(0);
  const settingsQueueRef = useRef<Promise<void>>(Promise.resolve());
  const pendingSettingsRef = useRef(0);

  // Depend on exactly the fields `applyTheme` reads, not on the whole
  // settings object: re-applying the theme on every unrelated settings write
  // would rebind the system colour-scheme listener for no reason.
  const appTheme = settings?.app_theme;
  const accentColor = settings?.accent_color;
  const reduceMotion = settings?.reduce_motion;
  const uiFontScale = settings?.ui_font_scale;
  const overlayTheme = settings?.overlay_theme;

  useEffect(() => {
    if (!appTheme) return;
    return applyTheme({
      app_theme: appTheme,
      accent_color: accentColor ?? "sky",
      reduce_motion: reduceMotion ?? false,
      ui_font_scale: uiFontScale ?? "normal",
      overlay_theme: overlayTheme ?? "dark",
    });
  }, [appTheme, accentColor, reduceMotion, uiFontScale, overlayTheme]);

  useEffect(() => {
    let cancelled = false;

    // Settings are the only data required to render the application shell.
    // Do not couple them to model status: the ASR sidecar can be slow or
    // unavailable, but that must never hold the whole UI on its startup gate.
    void api
      .getSettings()
      .then((cfg) => {
        if (!cancelled) setSettings(normalizeSettings(cfg));
      })
      .catch((err) => {
        console.error("Settings load failed:", err);
        if (!cancelled) setStartupError(true);
      });

    void api
      .getAppState()
      .then((st) => {
        if (!cancelled) setAppState(st);
      })
      .catch((err) => {
        console.error("App state load failed:", err);
      });

    void api
      .getModelStatus()
      .then((model) => {
        if (cancelled) return;
        setModelStatus(model);
        if (isTauri() && !model.installed && !isModelReady(model)) {
          setWizardOpen(true);
        }
      })
      .catch((err) => {
        // Model readiness is optional for rendering the home screen. The
        // settings/model UI can report and recover from an unavailable engine.
        console.error("Model status load failed:", err);
      });

    return () => {
      cancelled = true;
    };
  }, [startupAttempt]);

  // Live events: model status, app state, transcript streaming, audio level.
  useEffect(() => {
    const unsubs: (() => void)[] = [];
    let disposed = false;
    const track = (unsubscribe: () => void) => {
      if (disposed) unsubscribe();
      else unsubs.push(unsubscribe);
    };
    const setup = async () => {
      track(await safeListen<ModelStatus>("model:status", (status) => setModelStatus(status)));
      track(
        await safeListen<AppState>("app:state-changed", (st) => {
          setAppState(st);
          if (st === "RECORDING") {
            setTranscript({ ...EMPTY_TRANSCRIPT });
            setBackendStage(null);
          }
        }),
      );
      track(
        await safeListen<number>("recording:audio-level", (lvl) =>
          setTranscript((prev) => ({ ...prev, audio_level: lvl })),
        ),
      );
      track(
        await safeListen<StreamingTranscriptPayload>("transcript:partial", (payload) =>
          setTranscript(payload),
        ),
      );
      track(
        await safeListen<StreamingTranscriptPayload>("transcript:final", (payload) => {
          setTranscript(payload);
          // One round trip gives both the last waterfall and the rolling
          // p50/p95, so the two can never disagree in the UI.
          api
            .getLatencyReport()
            .then((report) => {
              setLatencyMetrics(report.last);
              setLatencyPercentiles(report.percentiles);
            })
            .catch(() => {});
        }),
      );
      track(await safeListen<BackendStage>("pipeline:stage", (stage) => setBackendStage(stage)));
      track(
        await safeListen<AppSettings>("settings:changed", (cfg) => {
          if (pendingSettingsRef.current === 0) setSettings(normalizeSettings(cfg));
        }),
      );
    };
    setup();
    return () => {
      disposed = true;
      unsubs.forEach((fn) => fn());
    };
  }, []);

  const handleUpdateSettings = useCallback((partial: Partial<AppSettings>): Promise<boolean> => {
    const token = ++settingsTokenRef.current;
    pendingSettingsRef.current += 1;
    setSettings((prev) => (prev ? normalizeSettings({ ...prev, ...partial }) : prev));
    const write = settingsQueueRef.current.then(async () => {
      try {
        const updated = await api.updateSettings(partial);
        if (token === settingsTokenRef.current) setSettings(normalizeSettings(updated));
        return true;
      } catch (error) {
        console.error("Failed to update settings:", error);
        if (token === settingsTokenRef.current) {
          try {
            const fresh = await api.getSettings();
            if (token === settingsTokenRef.current) setSettings(normalizeSettings(fresh));
          } catch {
            setToast({
              kind: "error",
              text: "Could not save or reload settings. Your changes are not confirmed.",
            });
            return false;
          }
        }
        setToast({ kind: "error", text: "Could not save this change. Please try again." });
        return false;
      } finally {
        pendingSettingsRef.current -= 1;
      }
    });
    settingsQueueRef.current = write.then(() => {});
    return write;
  }, []);

  const handleStartRecording = async () => {
    setTranscript({ ...EMPTY_TRANSCRIPT });
    setBackendStage(null);
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

  // Auto-dismiss toast.
  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 3000);
    return () => clearTimeout(t);
  }, [toast]);

  // Hub toasts (runtime download complete/error) flow into the same slot.
  const hubToast = intelligence.lastToast;
  useEffect(() => {
    if (hubToast) setToast(hubToast);
  }, [hubToast]);

  if (!settings) {
    return (
      <div className="h-screen w-screen flex flex-col items-center justify-center bg-base text-ink gap-3">
        {startupError ? (
          <>
            <h1 className="text-xl font-semibold">Reflow couldn’t start</h1>
            <p role="alert" className="text-sm text-muted">
              Your settings could not be loaded.
            </p>
            <button
              className="btn btn-primary"
              onClick={() => {
                setStartupError(false);
                setStartupAttempt((n) => n + 1);
              }}
            >
              Try again
            </button>
          </>
        ) : (
          <>
            <div className="w-8 h-8 rounded-full border-2 border-accent border-t-transparent animate-spin" />
            <p role="status" className="text-sm text-muted">
              Starting Reflow…
            </p>
          </>
        )}
      </div>
    );
  }

  const showOnboarding = wizardOpen && !onboardingComplete;

  return (
    <div className="flex flex-col h-screen w-screen bg-base text-ink overflow-hidden font-sans select-none">
      <TitleBar />

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
                backendStage={backendStage}
                latencyMetrics={latencyMetrics}
                latencyPercentiles={latencyPercentiles}
                onStartRecording={handleStartRecording}
                onStopRecording={handleStopRecording}
                onUpdateSettings={handleUpdateSettings}
                onOpenHistory={() => setActiveTab("history")}
                onOpenSettings={() => setActiveTab("settings")}
              />
            ))}

          {activeTab === "history" && <HistoryView />}

          {activeTab === "settings" && (
            <SettingsView
              settings={settings}
              onUpdateSettings={handleUpdateSettings}
              modelStatus={modelStatus}
              onReloadModel={() => api.reloadModel()}
              intelligence={intelligence}
              onInstallRuntime={async () => {
                try {
                  await api.installLlamaRuntime(settings?.refinement.device ?? "cpu");
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
            toast.kind === "error" ? "bg-rose-600 text-white" : "bg-slate-900 text-white"
          }`}
          role="status"
        >
          {toast.text}
        </div>
      )}

      {!isTauri() && (
        <Overlay appState={appState} transcript={transcript} backendStage={backendStage} />
      )}
    </div>
  );
};
export default App;
