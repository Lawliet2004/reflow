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

const EMPTY: StreamingTranscriptPayload = {
  committed_prefix: "",
  mutable_suffix: "",
  full_text: "",
  language: "en",
  audio_level: 0,
  stage: "",
};

export const OverlayApp: React.FC = () => {
  const [response, setResponse] = useState<string | null>(null);
  const [appState, setAppState] = useState<AppState>("READY");
  const [transcript, setTranscript] = useState<StreamingTranscriptPayload>(EMPTY);
  const [injection, setInjection] = useState<InjectionFeedback | null>(null);
  const [settings, setSettings] = useState<AppSettings | null>(null);
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
            setResponse(null);
            setInjection(null);
            setTranscript(EMPTY);
            setBackendStage(null);
          }
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

  return (
    <Overlay
      appState={appState}
      transcript={transcript}
      standalone
      extraMessage={injection?.message ?? null}
      backendStage={backendStage}
      hudTheme={settings?.overlay_theme ?? "dark"}
      waveformStyle={settings?.waveform_style ?? "bars"}
      hudScale={settings?.hud_scale ?? "standard"}
      hudShape={settings?.hud_shape ?? "pill"}
      hudStyle={settings?.hud_style ?? "status"}
      hudOpacity={settings?.hud_opacity ?? 0.96}
    />
  );
};
