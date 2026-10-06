import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  AppState,
  AppSettings,
  Appearance,
  IntelligenceTier,
  LatencyMetrics,
  LatencyPercentiles,
  ModelStatus,
  StreamingTranscriptPayload,
  isModelReady,
  normalizeSettings,
  pickAppearance,
} from "./types";
import { api, isTauri } from "./services/tauriApi";
import { createEventScope } from "./services/eventScope";
import { useIntelligenceHub } from "./hooks/useIntelligenceHub";
import { Navigation, NavTab, PageTab } from "./components/Navigation";
import { TitleBar } from "./components/TitleBar";
import { DictateHome } from "./components/DictateHome";
import { useFileTranscriptionController } from "./components/FileTranscription";
import { BackendStage } from "./components/hud/stages";
import { NotesView } from "./components/NotesView";
import { HistoryView } from "./components/HistoryView";
import { SettingsDialog, SettingsView } from "./components/SettingsView";
import { Overlay } from "./components/Overlay";
import { Onboarding } from "./components/Onboarding";
import { applyTheme, syncWindowChrome } from "./utils/theme";
import logoUrl from "./assets/logo.png";
import { AlertCircle, Info, X } from "lucide-react";

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
  const [activeTab, setActiveTab] = useState<PageTab>("dictate");
  const [settingsInitialPage, setSettingsInitialPage] = useState<"general" | "model">("general");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const openSettings = useCallback((page: "general" | "model" = "general") => {
    setSettingsInitialPage(page);
    setSettingsOpen(true);
  }, []);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    try {
      return localStorage.getItem("reflow.sidebar-collapsed") === "1";
    } catch {
      return false;
    }
  });
  const toggleSidebar = () =>
    setSidebarCollapsed((collapsed) => {
      try {
        localStorage.setItem("reflow.sidebar-collapsed", collapsed ? "0" : "1");
      } catch {
        /* Preference is cosmetic; an unavailable store just means it is not remembered. */
      }
      return !collapsed;
    });
  const [noteDraft, setNoteDraft] = useState("");
  const [notesRevision, setNotesRevision] = useState(0);
  const [appState, setAppState] = useState<AppState>("READY");
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [startupError, setStartupError] = useState(false);
  const [startupAttempt, setStartupAttempt] = useState(0);
  const [transcript, setTranscript] = useState<StreamingTranscriptPayload>(EMPTY_TRANSCRIPT);
  const [sourceTranscript, setSourceTranscript] = useState<{
    raw: string;
    final_text: string;
  } | null>(null);
  const [modelStatus, setModelStatus] = useState<ModelStatus | null>(null);
  // Latest backend pipeline stage (`DoryEvent::Stage`, forwarded as
  // `pipeline:stage`). Cleared on each new recording so a stale stage from the
  // previous dictation never leaks into the next one.
  const [backendStage, setBackendStage] = useState<BackendStage | null>(null);
  const [asrProgress, setAsrProgress] = useState<{ completed: number; total: number } | null>(null);
  useEffect(() => {
    if (appState !== "PROCESSING") return;
    let alive = true,
      pending = false;
    const refresh = async () => {
      if (pending) return;
      pending = true;
      try {
        const progress = await api.getAsrProgress();
        if (alive) setAsrProgress(progress);
      } catch {
        /* Progress is optional; final transcription remains authoritative. */
      } finally {
        pending = false;
      }
    };
    void refresh();
    const timer = setInterval(refresh, 500);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [appState]);
  const [latencyMetrics, setLatencyMetrics] = useState<LatencyMetrics | null>(null);
  const [latencyPercentiles, setLatencyPercentiles] = useState<LatencyPercentiles | null>(null);
  const [onboardingComplete, setOnboardingComplete] = useState(false);
  const [wizardOpen, setWizardOpen] = useState(false);
  const [toast, setToast] = useState<{ kind: "error" | "info"; text: string } | null>(null);
  const fileTranscription = useFileTranscriptionController(
    appState === "RECORDING" || appState === "PROCESSING" || appState === "INJECTING",
  );
  const fileFailure = fileTranscription.failure;
  useEffect(() => {
    let alive = true;
    if (activeTab !== "dictate" && fileFailure)
      queueMicrotask(() => {
        if (alive) setToast({ kind: "error", text: `File import failed: ${fileFailure}` });
      });
    return () => {
      alive = false;
    };
  }, [activeTab, fileFailure]);
  const [historyRecovery, setHistoryRecovery] = useState<string | null>(null);
  // Single intelligence hub: download progress arrives as events, status via
  // polling only when idle. Replaces the old inline listeners + per-page polls.
  const intelligence = useIntelligenceHub();
  useEffect(() => {
    let alive = true;
    api
      .queryHistory({ limit: 1 })
      .then((page) => {
        if (alive) setHistoryRecovery(page.recovery_notice);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  const settingsTokenRef = useRef(0);
  const settingsEpochRef = useRef(0);
  const appStateEpochRef = useRef(0);
  const modelStatusEpochRef = useRef(0);
  const settingsEventDuringSave = useRef(false);
  const settingsQueueRef = useRef<Promise<void>>(Promise.resolve());
  const pendingSettingsRef = useRef(0);

  // Depend on exactly the fields `applyTheme` reads, not on the whole
  // settings object: re-applying the theme on every unrelated settings write
  // would rebind the system colour-scheme listener for no reason.
  const appearanceKey = settings ? JSON.stringify(pickAppearance(settings)) : null;

  useEffect(() => {
    if (!appearanceKey) return;
    const appearance = JSON.parse(appearanceKey) as Appearance;
    if (isTauri()) void syncWindowChrome(appearance.app_theme, appearance.window_material);
    return applyTheme(appearance);
  }, [appearanceKey]);

  useEffect(() => {
    let cancelled = false;
    const settingsEpoch = settingsEpochRef.current;
    const stateEpoch = appStateEpochRef.current;
    const modelEpoch = modelStatusEpochRef.current;

    // Settings are the only data required to render the application shell.
    // Do not couple them to model status: the ASR sidecar can be slow or
    // unavailable, but that must never hold the whole UI on its startup gate.
    void api
      .getSettings()
      .then((cfg) => {
        if (!cancelled && settingsEpoch === settingsEpochRef.current)
          setSettings(normalizeSettings(cfg));
      })
      .catch((err) => {
        console.error("Settings load failed:", err);
        if (!cancelled) setStartupError(true);
      });

    void api
      .getAppState()
      .then((st) => {
        if (!cancelled && stateEpoch === appStateEpochRef.current) setAppState(st);
      })
      .catch((err) => {
        console.error("App state load failed:", err);
      });

    void api
      .getModelStatus()
      .then((model) => {
        if (cancelled || modelEpoch !== modelStatusEpochRef.current) return;
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
    const scope = createEventScope();
    void Promise.all([
      scope.listen<ModelStatus>("model:status", (status) => {
        modelStatusEpochRef.current += 1;
        setModelStatus(status);
      }),
      scope.listen<NavTab>("ui:navigate", (tab) =>
        tab === "settings" ? openSettings() : setActiveTab(tab),
      ),
      scope.listen<AppState>("app:state-changed", (st) => {
        appStateEpochRef.current += 1;
        setAppState(st);
        if (st === "RECORDING") {
          setTranscript({ ...EMPTY_TRANSCRIPT });
          setSourceTranscript(null);
          setAsrProgress(null);
          setBackendStage(null);
        }
      }),
      scope.listen<string>("recording:error", (error) => {
        setToast({ kind: "error", text: error || "Dictation failed. Please try again." });
      }),
      scope.listen<string>("app:warning", (warning) => {
        setToast({ kind: "error", text: warning });
      }),
      scope.listen<string>("hotkey:error", (text) => setToast({ kind: "error", text })),
      scope.listen("app:auto-stop", () =>
        setToast({ kind: "info", text: "Recording stopped automatically." }),
      ),
      scope.listen<number>("recording:audio-level", (lvl) =>
        setTranscript((prev) => ({ ...prev, audio_level: lvl })),
      ),
      scope.listen<StreamingTranscriptPayload>("transcript:partial", (payload) =>
        setTranscript(payload),
      ),
      scope.listen<StreamingTranscriptPayload>("transcript:final", (payload) => {
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
      scope.listen<{ raw: string; final_text: string }>("transcript:source", setSourceTranscript),
      scope.listen<BackendStage>("pipeline:stage", (stage) => setBackendStage(stage)),
      scope.listen<AppSettings>("settings:changed", (cfg) => {
        settingsEpochRef.current += 1;
        if (pendingSettingsRef.current === 0) setSettings(normalizeSettings(cfg));
        else settingsEventDuringSave.current = true;
      }),
    ]);
    return () => scope.dispose();
  }, [openSettings]);

  const handleUpdateSettings = useCallback((partial: Partial<AppSettings>): Promise<boolean> => {
    const token = ++settingsTokenRef.current;
    settingsEpochRef.current += 1;
    pendingSettingsRef.current += 1;
    setSettings((prev) => (prev ? normalizeSettings({ ...prev, ...partial }) : prev));
    const write = settingsQueueRef.current.then(async () => {
      const epoch = settingsEpochRef.current;
      try {
        const updated = await api.updateSettings(partial);
        if (token === settingsTokenRef.current && epoch === settingsEpochRef.current)
          setSettings(normalizeSettings(updated));
        return true;
      } catch (error) {
        console.error("Failed to update settings:", error);
        if (token === settingsTokenRef.current) {
          try {
            const reloadEpoch = settingsEpochRef.current;
            const fresh = await api.getSettings();
            if (token === settingsTokenRef.current && reloadEpoch === settingsEpochRef.current)
              setSettings(normalizeSettings(fresh));
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
        if (pendingSettingsRef.current === 0 && settingsEventDuringSave.current) {
          settingsEventDuringSave.current = false;
          const reloadEpoch = settingsEpochRef.current;
          try {
            const fresh = await api.getSettings();
            if (pendingSettingsRef.current === 0 && reloadEpoch === settingsEpochRef.current)
              setSettings(normalizeSettings(fresh));
          } catch {
            setToast({
              kind: "error",
              text: "Could not refresh the latest settings. Your changes are not confirmed.",
            });
          }
        }
      }
    });
    settingsQueueRef.current = write.then(() => {});
    return write;
  }, []);

  const handleStartRecording = async () => {
    appStateEpochRef.current += 1;
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
    appStateEpochRef.current += 1;
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

  // Auto-dismiss toast. Errors stay long enough to read and act on.
  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), toast.kind === "error" ? 8000 : 3000);
    return () => clearTimeout(t);
  }, [toast]);

  // Hub toasts (runtime download complete/error) flow into the same slot.
  const hubToast = intelligence.lastToast;
  useEffect(() => {
    let alive = true;
    if (hubToast)
      queueMicrotask(() => {
        if (alive) setToast(hubToast);
      });
    return () => {
      alive = false;
    };
  }, [hubToast]);

  if (!settings) {
    return (
      <div className="flex flex-col h-screen w-screen text-ink overflow-hidden font-sans select-none">
        <TitleBar />
        {startupError ? (
          <div className="flex-1 flex flex-col items-center justify-center gap-3 bg-base">
            <img
              src={logoUrl}
              alt=""
              aria-hidden
              className="w-12 h-12 mb-1 opacity-90 grayscale pointer-events-none"
              draggable={false}
            />
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
          </div>
        ) : (
          // Shell-shaped placeholder: launch reads as "almost there", not "blocked".
          <div className="flex flex-1 min-h-0" aria-busy="true">
            <p role="status" className="sr-only">
              Starting Reflow…
            </p>
            <aside className="app-sidebar gap-2" aria-hidden="true">
              {[0, 1, 2, 3].map((i) => (
                <div key={i} className="skeleton h-9" />
              ))}
            </aside>
            <div className="app-canvas" aria-hidden="true">
              <div className="workspace-page space-y-6">
                <div className="skeleton h-8 w-56" />
                <div className="skeleton h-72 rounded-2xl" />
                <div className="skeleton h-40 rounded-2xl" />
              </div>
            </div>
          </div>
        )}
      </div>
    );
  }

  const showOnboarding = wizardOpen && !onboardingComplete;
  const toastElement = toast && (
    <div
      className={`toast ${toast.kind === "error" ? "toast-error" : "toast-info"}`}
      role={toast.kind === "error" ? "alert" : "status"}
    >
      {toast.kind === "error" ? (
        <AlertCircle className="w-4 h-4" aria-hidden />
      ) : (
        <Info className="w-4 h-4" aria-hidden />
      )}
      <p>{toast.text}</p>
      <button aria-label="Dismiss notification" onClick={() => setToast(null)}>
        <X className="w-3.5 h-3.5" />
      </button>
    </div>
  );

  return (
    <div className="flex flex-col h-screen w-screen text-ink overflow-hidden font-sans select-none">
      <TitleBar
        offline={settings?.offline_mode}
        sidebarCollapsed={sidebarCollapsed}
        onToggleSidebar={toggleSidebar}
      />

      <div className="flex flex-1 min-h-0 overflow-hidden">
        <Navigation
          activeTab={activeTab}
          setActiveTab={setActiveTab}
          appState={appState}
          modelStatus={modelStatus}
          collapsed={sidebarCollapsed}
          settingsOpen={settingsOpen}
          onOpenSettings={openSettings}
        />

        <main className="app-canvas">
          {historyRecovery && (
            <div
              role="alert"
              className="flex flex-wrap items-center justify-between gap-3 px-5 py-3 text-sm text-danger bg-danger-soft"
            >
              <p>{historyRecovery}</p>
              <button className="btn btn-ghost" onClick={() => setActiveTab("history")}>
                Open history and export
              </button>
            </div>
          )}
          <div className="flex-1 min-h-0 overflow-y-auto select-text">
            {activeTab === "dictate" &&
              (showOnboarding ? (
                <Onboarding
                  settings={settings}
                  intelligenceTiers={intelligence.intelligenceTiers}
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
                  intelligenceTiers={intelligence.intelligenceTiers}
                  settings={settings}
                  modelStatus={modelStatus}
                  transcript={transcript}
                  originalText={
                    sourceTranscript?.final_text === transcript.full_text
                      ? sourceTranscript.raw
                      : undefined
                  }
                  backendStage={backendStage}
                  asrProgress={asrProgress}
                  latencyMetrics={latencyMetrics}
                  latencyPercentiles={latencyPercentiles}
                  onStartRecording={handleStartRecording}
                  onStopRecording={handleStopRecording}
                  onUpdateSettings={handleUpdateSettings}
                  fileTranscription={fileTranscription}
                  onOpenHistory={() => setActiveTab("history")}
                  onOpenSettings={() => openSettings("model")}
                />
              ))}

            {activeTab === "notes" && (
              <NotesView
                settings={settings}
                onUpdateSettings={handleUpdateSettings}
                appState={appState}
                draft={noteDraft}
                onDraftChange={setNoteDraft}
                onDraftSaved={(submitted) =>
                  setNoteDraft((current) => (current === submitted ? "" : current))
                }
                externalRevision={notesRevision}
                onNotesChanged={() => setNotesRevision((current) => current + 1)}
              />
            )}
            {activeTab === "history" && <HistoryView />}
          </div>
        </main>
      </div>

      <SettingsDialog open={settingsOpen} onClose={() => setSettingsOpen(false)}>
        <SettingsView
          initialPage={settingsInitialPage}
          settings={settings}
          onUpdateSettings={handleUpdateSettings}
          modelStatus={modelStatus}
          onReloadModel={async () => {
            try {
              await api.reloadModel();
            } catch (error) {
              console.error("Speech model reload failed:", error);
              setToast({
                kind: "error",
                text: "Could not reload the speech model. Check Settings for details and try again.",
              });
            }
          }}
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
        {/* The modal sits in the top layer; its toasts must live inside it to be seen. */}
        {toastElement}
      </SettingsDialog>

      {!settingsOpen && toastElement}

      {!isTauri() && (
        <Overlay
          appState={appState}
          transcript={transcript}
          backendStage={backendStage}
          hudTheme={settings?.overlay_theme}
          waveformStyle={settings?.waveform_style}
          hudScale={settings?.hud_scale}
          hudShape={settings?.hud_shape}
          hudStyle={settings?.hud_style}
          hudOpacity={settings?.hud_opacity}
        />
      )}
    </div>
  );
};
export default App;
