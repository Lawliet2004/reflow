import React, { useEffect, useState } from "react";
import { AppSettings, AudioDevice, MicrophoneHealth } from "../../types";
import { api } from "../../services/tauriApi";
import { Section, Row } from "./ui";
import { LanguageOptions } from "../LanguageOptions";
import { RecognitionCheck } from "../RecognitionCheck";

import { Mic } from "lucide-react";

interface Props {
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => void;
}

export const AudioPage: React.FC<Props> = ({ settings, onUpdateSettings }) => {
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [devicesError, setDevicesError] = useState<string | null>(null);
  const [loadingDevices, setLoadingDevices] = useState(false);
  const [testing, setTesting] = useState(false);
  const [health, setHealth] = useState<MicrophoneHealth | null>(null);
  const [testError, setTestError] = useState<string | null>(null);

  const testMicrophone = async () => {
    setTesting(true);
    setHealth(null);
    setTestError(null);
    try {
      setHealth(await api.testMicrophone());
    } catch (error) {
      setTestError(String(error));
    } finally {
      setTesting(false);
    }
  };

  const refreshDevices = async () => {
    setLoadingDevices(true);
    try {
      setDevices(await api.getAudioDevices());
      setDevicesError(null);
    } catch {
      setDevicesError("Could not list microphones. Check audio permissions and refresh.");
    } finally {
      setLoadingDevices(false);
    }
  };

  useEffect(() => {
    let alive = true;
    queueMicrotask(() => {
      if (alive) refreshDevices();
    });
    return () => {
      alive = false;
    };
  }, []);

  const change = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) =>
    onUpdateSettings({ [key]: value } as Partial<AppSettings>);

  return (
    <Section icon={<Mic className="w-4 h-4" />} title="Microphone">
      <Row label="Microphone" hint="Input device used for dictation">
        <select
          className="field max-w-[240px]"
          aria-label="Microphone"
          value={settings.microphone_device_id ?? "default"}
          onChange={(e) => {
            const id = e.target.value;
            change("microphone_device_id", id === "default" ? null : id);
          }}
        >
          <option value="default">System default</option>
          {settings.microphone_device_id &&
            !devices.some((d) => d.id === settings.microphone_device_id) && (
              <option value={settings.microphone_device_id}>Selected microphone unavailable</option>
            )}
          {devices
            .filter((d) => d.id !== "default")
            .map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
                {d.is_default ? " · System default" : ""}
              </option>
            ))}
        </select>
      </Row>
      {devicesError && (
        <p role="alert" className="text-sm text-danger">
          {devicesError}
        </p>
      )}
      <button className="btn btn-ghost" disabled={loadingDevices} onClick={refreshDevices}>
        {loadingDevices ? "Refreshing microphones…" : "Refresh microphones"}
      </button>
      <Row
        label="Check microphone"
        hint="Speak for three seconds to check signal, clipping and dropped audio. The sample is discarded."
      >
        <button className="btn btn-secondary" disabled={testing} onClick={testMicrophone}>
          {testing ? "Listening for 3 seconds…" : "Test microphone"}
        </button>
      </Row>
      <div aria-live="polite">
        {testing && <p className="text-sm text-muted">Speak normally into your microphone now.</p>}
        {testError && (
          <p role="alert" className="text-sm text-danger">
            {testError}
          </p>
        )}
        {health && (
          <div className="text-sm text-muted space-y-1">
            <p>{health.assessment}</p>
            <p>
              {health.device} · {(health.duration_ms / 1000).toFixed(1)} s · Peak{" "}
              {Math.round(health.peak * 100)}% · Clipped {health.clipped_pct.toFixed(1)}% ·{" "}
              {health.dropped_chunks} dropped frames
            </p>
          </div>
        )}
      </div>

      <Row label="Input gain" hint={`Current: ${settings.input_gain.toFixed(1)}×`}>
        <div className="flex items-center gap-3">
          <input
            type="range"
            aria-label="Input gain"
            min={0.5}
            max={3}
            step={0.1}
            value={settings.input_gain}
            onChange={(e) => change("input_gain", Number(e.target.value))}
            className="w-[140px]"
          />
          <span className="text-xs font-semibold text-ink w-8 text-right">
            {settings.input_gain.toFixed(1)}×
          </span>
        </div>
      </Row>

      <Row label="Silence auto-stop" hint="Ends recording after a pause">
        <select
          className="field"
          aria-label="Silence auto-stop"
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

      <Row
        label="Dictation language"
        hint="Use auto-detect for mixed-language speech or Chinese dialects"
      >
        <select
          className="field"
          aria-label="Dictation language"
          value={settings.language}
          onChange={(e) =>
            onUpdateSettings({
              language: e.target.value,
              auto_detect_language: e.target.value === "auto",
            })
          }
        >
          <LanguageOptions />
        </select>
      </Row>
      <p className="text-sm text-muted">
        30 languages supported by Qwen3-ASR, with multilingual cleanup by Qwen3.5. Cleanup keeps the
        original language and script. Accuracy varies by language and model.
      </p>
      <RecognitionCheck
        key={`${settings.microphone_device_id}:${settings.input_gain}:${settings.language}:${settings.asr.model}`}
      />
    </Section>
  );
};
