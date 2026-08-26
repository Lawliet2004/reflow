import React, { useEffect, useState } from "react";
import { AppSettings, AppState, InjectionFeedback, StreamingTranscriptPayload, normalizeSettings } from "../types";
import { api, safeListen } from "../services/tauriApi";
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

  useEffect(() => {
    document.documentElement.classList.add("overlay-window");
    document.body.classList.add("overlay-window");
  }, []);

  useEffect(() => {
    api.getSettings().then((cfg) => setSettings(normalizeSettings(cfg))).catch(() => {});
  }, []);

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
    const unsubs: (() => void)[] = [];
    const setup = async () => {
      unsubs.push(
        ...(await Promise.all([
          safeListen<AppState>("app:state-changed", (state) => {
            setAppState(state);
            if (state === "RECORDING") {
              setInjection(null);
              setTranscript(EMPTY);
            }
          }),
          safeListen<StreamingTranscriptPayload>("transcript:partial", setTranscript),
          safeListen<StreamingTranscriptPayload>("transcript:final", setTranscript),
          safeListen<number>("recording:audio-level", (lvl) =>
            setTranscript((prev) => ({ ...prev, audio_level: lvl }))
          ),
          safeListen<InjectionFeedback>("injection:result", setInjection),
          safeListen<AppSettings>("settings:changed", (cfg) =>
            setSettings(normalizeSettings(cfg))
          ),
        ]))
      );
      try {
        setAppState(await api.getAppState());
      } catch {
        /* overlay still listens */
      }
    };
    setup();
    return () => unsubs.forEach((fn) => fn());
  }, []);

  return (
    <Overlay
      appState={appState}
      transcript={transcript}
      standalone
      extraMessage={injection?.message ?? null}
      hudTheme={settings?.overlay_theme ?? "dark"}
      waveformStyle={settings?.waveform_style ?? "bars"}
      hudScale={settings?.hud_scale ?? "standard"}
    />
  );
};
