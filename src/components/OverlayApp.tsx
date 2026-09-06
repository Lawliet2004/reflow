import React, { useEffect, useState } from "react";
import {
  AppSettings,
  AppState,
  InjectionFeedback,
  StreamingTranscriptPayload,
  normalizeSettings,
} from "../types";
import { api } from "../services/tauriApi";
import { createEventScope } from "../services/eventScope";
import { BackendStage } from "./hud/stages";
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
  const [appState, setAppState] = useState<AppState>("READY");
  const [transcript, setTranscript] = useState<StreamingTranscriptPayload>(EMPTY);
  const [injection, setInjection] = useState<InjectionFeedback | null>(null);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [backendStage, setBackendStage] = useState<BackendStage | null>(null);

  useEffect(() => {
    document.documentElement.classList.add("overlay-window");
    document.body.classList.add("overlay-window");
  }, []);

  useEffect(() => {
    api
      .getSettings()
      .then((cfg) => setSettings(normalizeSettings(cfg)))
      .catch(() => {});
  }, []);

  // Same narrowing as App.tsx: depend on the fields applyTheme reads.
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
    const scope = createEventScope();
    const setup = async () => {
      await Promise.all([
        scope.listen<AppState>("app:state-changed", (state) => {
          setAppState(state);
          if (state === "RECORDING") {
            setInjection(null);
            setTranscript(EMPTY);
            setBackendStage(null);
          }
        }),
        scope.listen<BackendStage>("pipeline:stage", setBackendStage),
        scope.listen<StreamingTranscriptPayload>("transcript:partial", setTranscript),
        scope.listen<StreamingTranscriptPayload>("transcript:final", setTranscript),
        scope.listen<number>("recording:audio-level", (lvl) =>
          setTranscript((prev) => ({ ...prev, audio_level: lvl })),
        ),
        scope.listen<InjectionFeedback>("injection:result", setInjection),
        scope.listen<AppSettings>("settings:changed", (cfg) => setSettings(normalizeSettings(cfg))),
      ]);
      try {
        setAppState(await api.getAppState());
      } catch {
        /* overlay still listens */
      }
    };
    setup();
    return () => scope.dispose();
  }, []);

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
    />
  );
};
