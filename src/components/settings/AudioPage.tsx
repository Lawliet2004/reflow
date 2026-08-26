import React, { useEffect, useState } from "react";
import { AppSettings, AudioDevice } from "../../types";
import { api } from "../../services/tauriApi";
import { Section, Row } from "./ui";

import { Mic } from "lucide-react";

interface Props {
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => void;
}

export const AudioPage: React.FC<Props> = ({ settings, onUpdateSettings }) => {
  const [devices, setDevices] = useState<AudioDevice[]>([]);

  useEffect(() => {
    api.getAudioDevices().then(setDevices).catch(() => {});
  }, []);

  const change = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) =>
    onUpdateSettings({ [key]: value } as Partial<AppSettings>);

  return (
    <Section
      icon={<Mic className="w-4 h-4" />}
      title="Microphone"
    >
      <Row label="Microphone" hint="Input device used for dictation">
        <select
          className="field max-w-[240px]"
          value={settings.microphone_device_id ?? "default"}
          onChange={(e) => {
            const id = e.target.value;
            change("microphone_device_id", id === "default" ? null : id);
          }}
        >
          <option value="default">Auto (best input device)</option>
          {devices
            .filter((d) => d.id !== "default")
            .map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
                {d.is_default ? " · Windows default" : ""}
              </option>
            ))}
        </select>
      </Row>

      <Row label="Input gain" hint={`Current: ${settings.input_gain.toFixed(1)}×`}>
        <div className="flex items-center gap-3">
          <input
            type="range"
            min={0.5}
            max={3}
            step={0.1}
            value={settings.input_gain}
            onChange={(e) => change("input_gain", Number(e.target.value))}
            className="w-[140px]"
          />
          <span className="text-[12px] font-semibold text-ink w-8 text-right">
            {settings.input_gain.toFixed(1)}×
          </span>
        </div>
      </Row>

      <Row label="Silence auto-stop" hint="Ends recording after a pause">
        <select
          className="field"
          value={settings.auto_stop_silence_ms}
          onChange={(e) => change("auto_stop_silence_ms", Number(e.target.value))}
        >
          <option value={800}>0.8 s</option>
          <option value={1200}>1.2 s</option>
          <option value={1500}>1.5 s (recommended)</option>
          <option value={2000}>2.0 s</option>
          <option value={0}>Off (manual stop only)</option>
        </select>
      </Row>

      <Row label="Language" hint="Target language for transcription">
        <select
          className="field"
          value={settings.language}
          onChange={(e) => change("language", e.target.value)}
        >
          <option value="auto">Auto-detect per sentence</option>
          <option value="en">English</option>
          <option value="hi">Hindi</option>
        </select>
      </Row>
    </Section>
  );
};
