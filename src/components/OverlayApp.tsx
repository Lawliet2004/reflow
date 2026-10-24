import { AlertCircle, Mic } from "lucide-react";
import React, { useEffect, useRef, useState } from "react";
import {
  AppSettings,
  AppState,
  Appearance,
  InjectionFeedback,
  StreamingTranscriptPayload,
  normalizeSettings,
  pickAppearance,
} from "../types";
import { api } from "../services/tauriApi";
import { createEventScope } from "../services/eventScope";
import { BackendStage } from "./hud/stages";
import { AssistantAnswer } from "./AssistantAnswer";
import { Overlay } from "./Overlay";
import { applyTheme } from "../utils/theme";
import { useRecordingPause } from "../hooks/useRecordingPause";
import { useViewVisibility } from "../hooks/useViewVisibility";

const EMPTY: StreamingTranscriptPayload = {
  committed_prefix: "",
  mutable_suffix: "",
  full_text: "",
  language: "en",
  audio_level: 0,
  stage: "",
};

export const OverlayApp: React.FC = () => {
  // Own visibility even when the HUD uses only CSS waveform animation.
  useViewVisibility();
  const [response, setResponse] = useState<string | null>(null);
  const [exiting, setExiting] = useState(false);
  const [appState, setAppState] = useState<AppState>("READY");
  const [transcript, setTranscript] = useState<StreamingTranscriptPayload>(EMPTY);
  const [injection, setInjection] = useState<InjectionFeedback | null>(null);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [idle, setIdle] = useState(true);
  const [busy, setBusy] = useState(false);
  const { paused, can_pause: canPause } = useRecordingPause();
  const actionPending = useRef(false);
  const [controlError, setControlError] = useState<string | null>(null);
  const soundVolume = useRef(0.7);
  useEffect(() => {
    soundVolume.current = settings?.sounds_volume ?? 0.7;
  }, [settings?.sounds_volume]);
  const [backendStage, setBackendStage] = useState<BackendStage | null>(null);

  useEffect(() => {
    document.documentElement.classList.add("overlay-window");
    document.body.classList.add("overlay-window");
  }, []);

  // Same narrowing as App.tsx: depend on the fields applyTheme reads.
  const appearanceKey = settings ? JSON.stringify(pickAppearance(settings)) : null;

  useEffect(() => {
    if (!appearanceKey) return;
    return applyTheme(JSON.parse(appearanceKey) as Appearance);
  }, [appearanceKey]);

  useEffect(() => {
    const scope = createEventScope();
    let active = true;
    let stateEvents = 0;
    let settingsEvents = 0;
    const setup = async () => {
      await Promise.all([
        scope.listen<AppState>("app:state-changed", (state) => {
          stateEvents++;
          setAppState(state);
          if (state === "RECORDING") {
            setIdle(false);
            setControlError(null);
            setExiting(false);
            setResponse(null);
            setInjection(null);
            setTranscript(EMPTY);
            setBackendStage(null);
          }
        }),
        scope.listen<boolean>("hud:hide", setExiting),
        scope.listen("hud:idle", () => {
          setResponse(null);
          setIdle(true);
          setExiting(false);
          setInjection(null);
          setTranscript(EMPTY);
          setBackendStage(null);
        }),
        scope.listen<string | { message: string }>("recording:error", (error) => {
          setControlError(typeof error === "string" ? error : error.message);
        }),
        scope.listen<boolean>("window:visibility", (visible) => {
          if (visible) setExiting(false);
        }),
        scope.listen<{ kind: "start" | "stop" }>("hud:sound", ({ kind }) => {
          if (kind !== "start" && kind !== "stop") return;
          const sound = new Audio(`${import.meta.env.BASE_URL}assets/${kind}.wav`);
          sound.volume = Math.max(0, Math.min(1, soundVolume.current));
          void sound.play().catch(() => {});
        }),
        scope.listen<string>("assistant:response", setResponse),
        scope.listen<BackendStage>("pipeline:stage", setBackendStage),
        scope.listen<StreamingTranscriptPayload>("transcript:partial", setTranscript),
        scope.listen<StreamingTranscriptPayload>("transcript:final", setTranscript),
        scope.listen<number>("recording:audio-level", (lvl) =>
          setTranscript((prev) => ({ ...prev, audio_level: lvl })),
        ),
        scope.listen<InjectionFeedback>("injection:result", setInjection),
        scope.listen<AppSettings>("settings:changed", (cfg) => {
          settingsEvents++;
          setSettings(normalizeSettings(cfg));
        }),
      ]);
      if (!active) return;
      const stateVersion = stateEvents;
      const settingsVersion = settingsEvents;
      await Promise.all([
        api
          .getAppState()
          .then((state) => {
            if (active && stateEvents === stateVersion) setAppState(state);
          })
          .catch(() => {}),
        api
          .getSettings()
          .then((cfg) => {
            if (active && settingsEvents === settingsVersion) setSettings(normalizeSettings(cfg));
          })
          .catch(() => {}),
      ]);
    };
    void setup();
    return () => {
      active = false;
      scope.dispose();
    };
  }, []);

  const recording = appState === "RECORDING";
  const processing = appState === "PROCESSING" || appState === "INJECTING";
  const floatingEnabled = settings?.floating_dictation_enabled ?? false;

  const runAction = async (togglePause = false) => {
    if (actionPending.current || processing) return;
    actionPending.current = true;
    setBusy(true);
    setControlError(null);
    try {
      if (togglePause && recording) {
        await api.setRecordingPaused(!paused);
      } else if (recording) {
        await api.stopRecording();
      } else {
        await api.startFloatingRecording();
      }
    } catch (error) {
      setControlError(error instanceof Error ? error.message : String(error));
    } finally {
      actionPending.current = false;
      setBusy(false);
    }
  };

  if (response)
    return (
      <AssistantAnswer
        key={response}
        response={response}
        onDismiss={() => {
          setResponse(null);
          void api.dismissAssistant();
        }}
      />
    );

  if (floatingEnabled && idle && !recording && !processing) {
    const light =
      settings?.overlay_theme === "light" ||
      (settings?.overlay_theme === "auto" && !document.documentElement.classList.contains("dark"));
    return (
      <div className="floating-dictation-launcher">
        <button
          type="button"
          className={`hud hud-standalone hud-launcher ${light ? "hud-light" : "hud-dark"} ${settings?.hud_scale === "compact" ? "hud-scale-compact" : settings?.hud_scale === "large" ? "hud-scale-large" : ""}`}
          style={{ "--hud-alpha": settings?.hud_opacity ?? 0.96 } as React.CSSProperties}
          aria-label={controlError ? `Retry dictation: ${controlError}` : "Start dictation"}
          title={controlError ? `${controlError} · Click to retry` : "Start dictation"}
          disabled={busy || (appState !== "READY" && appState !== "IDLE")}
          onClick={() => {
            void runAction();
          }}
        >
          {busy ? (
            <span className="hud-spinner" />
          ) : controlError ? (
            <AlertCircle size={18} aria-hidden="true" />
          ) : (
            <Mic size={18} aria-hidden="true" />
          )}
        </button>
        {controlError && (
          <span className="sr-only" role="alert">
            {controlError}
          </span>
        )}
      </div>
    );
  }

  return (
    <Overlay
      appState={appState}
      transcript={transcript}
      standalone
      exiting={
        exiting && appState !== "RECORDING" && appState !== "PROCESSING" && appState !== "INJECTING"
      }
      extraMessage={controlError ?? injection?.message ?? null}
      backendStage={backendStage}
      hudTheme={settings?.overlay_theme ?? "dark"}
      waveformStyle={settings?.waveform_style ?? "bars"}
      hudScale={settings?.hud_scale ?? "standard"}
      hudShape={settings?.hud_shape ?? "pill"}
      hudStyle={floatingEnabled ? "waveform" : (settings?.hud_style ?? "status")}
      onStop={
        floatingEnabled
          ? () => {
              void runAction();
            }
          : undefined
      }
      paused={paused}
      controlError={controlError}
      onTogglePause={
        floatingEnabled && canPause && (settings?.recording_pause_enabled || paused)
          ? () => {
              void runAction(true);
            }
          : undefined
      }
      stopping={busy}
      hudOpacity={settings?.hud_opacity ?? 0.96}
    />
  );
};
