import React, { useEffect, useState } from "react";
import { Check, ChevronRight, Download, Keyboard, Mic, Loader2 } from "lucide-react";
import { AppSettings, AudioDevice, ModelStatus, isModelReady } from "../types";
import { api } from "../services/tauriApi";
import { HotkeyPicker } from "./HotkeyPicker";
import { RecognitionCheck } from "./RecognitionCheck";
import { LlmSelector } from "./LlmSelector";
import type { IntelligenceTierState } from "../types";

interface OnboardingProps {
  settings: AppSettings;
  modelStatus: ModelStatus | null;
  onUpdateSettings: (settings: Partial<AppSettings>) => Promise<boolean> | void;
  onComplete: () => void;
  intelligenceTiers?: IntelligenceTierState[] | null;
}

const STEPS = [
  { id: "mic", label: "Microphone" },
  { id: "hotkey", label: "Hotkey" },
  { id: "model", label: "Speech model" },
];

export const Onboarding: React.FC<OnboardingProps> = ({
  settings,
  modelStatus,
  onUpdateSettings: persistSettings,
  onComplete,
  intelligenceTiers,
}) => {
  const [step, setStep] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [testedConfiguration, setTestedConfiguration] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const onUpdateSettings = async (changes: Partial<AppSettings>) => {
    if (
      changes.asr ||
      "microphone_device_id" in changes ||
      "input_gain" in changes ||
      "language" in changes
    )
      setTestedConfiguration(null);
    setSaving(true);
    setError(null);
    try {
      if ((await persistSettings(changes)) === false) {
        setError("Settings could not be saved. Try again before testing recognition.");
        return false;
      }
      return true;
    } catch (failure) {
      setError(`Settings could not be saved: ${String(failure)}`);
      return false;
    } finally {
      setSaving(false);
    }
  };
  const configuration = `${settings.microphone_device_id}:${settings.input_gain}:${settings.language}:${settings.asr.model}`;

  const installed = Boolean(modelStatus?.installed);
  const downloading = Boolean(modelStatus?.is_downloading);
  const ready = isModelReady(modelStatus);

  useEffect(() => {
    api
      .getAudioDevices()
      .then(setDevices)
      .catch(() =>
        setError("Could not list microphones. Check your audio permissions and reopen setup."),
      );
  }, []);

  const pickMic = (id: string) => {
    onUpdateSettings({ microphone_device_id: id === "default" ? null : id });
  };

  const downloadModel = async () => {
    setError(null);
    setInstalling(true);
    try {
      await api.installModel(settings.asr.model);
    } catch (e) {
      console.error("Model install failed:", e);
      setError(`Could not download the speech model: ${String(e)}`);
    } finally {
      setInstalling(false);
    }
  };

  const canContinue = !saving && (step < 2 || (ready && testedConfiguration === configuration));

  const goNext = () => {
    if (step >= STEPS.length - 1) {
      onComplete();
      return;
    }
    setStep((s) => s + 1);
  };

  return (
    <div className="workspace-page max-w-2xl animate-fade-rise">
      {saving && (
        <p role="status" className="text-sm text-muted">
          Saving settings…
        </p>
      )}
      <fieldset disabled={saving} className="min-w-0" aria-busy={saving}>
        {error && (
          <p role="alert" className="text-sm text-danger mb-4">
            {error}
          </p>
        )}
        <p className="label-micro text-accent mb-2">First run</p>
        <h1 className="font-display text-3xl font-semibold tracking-tight text-ink">
          A few things. Then just speak.
        </h1>
        <p className="text-sm text-muted mt-1">
          Three quick steps so dictation is ready on this computer.
        </p>

        <div className="flex items-center gap-2 mt-6 mb-8">
          {STEPS.map((s, i) => (
            <div key={s.id} className="flex-1">
              <div className={`h-1 rounded-full ${i <= step ? "bg-accent" : "bg-line"}`} />
              <p
                className={`text-2xs mt-1.5 truncate ${
                  i === step ? "text-accent font-medium" : "text-muted"
                }`}
              >
                {s.label}
              </p>
            </div>
          ))}
        </div>

        {step === 0 && (
          <section className="panel p-5 space-y-4">
            <div className="flex items-center gap-2.5">
              <div className="w-8 h-8 rounded-lg bg-accent-soft border border-accent-border text-accent flex items-center justify-center">
                <Mic className="w-4 h-4" />
              </div>
              <div>
                <h2 className="text-base font-semibold text-ink">Choose a microphone</h2>
                <p className="text-xs text-muted">
                  You can change this later in Settings → Speech.
                </p>
              </div>
            </div>
            <select
              className="field w-full"
              aria-label="Microphone"
              value={settings.microphone_device_id ?? "default"}
              onChange={(e) => pickMic(e.target.value)}
            >
              <option value="default">System default</option>
              {devices
                .filter((d) => d.id !== "default")
                .map((d) => (
                  <option key={d.id} value={d.id}>
                    {d.name}
                    {d.is_default ? " · System default" : ""}
                  </option>
                ))}
            </select>
          </section>
        )}

        {step === 1 && (
          <section className="panel p-5 space-y-4">
            <div className="flex items-center gap-2.5">
              <div className="w-8 h-8 rounded-lg bg-accent-soft border border-accent-border text-accent flex items-center justify-center">
                <Keyboard className="w-4 h-4" />
              </div>
              <div>
                <h2 className="text-base font-semibold text-ink">Pick your shortcut</h2>
                <p className="text-xs text-muted">
                  Hold it to record anywhere. You can change it later in Settings.
                </p>
              </div>
            </div>
            <div className="rounded-xl border border-accent-border bg-accent-soft px-4 py-5 flex flex-col items-center gap-3">
              <p className="text-xs text-muted">Press any key combination</p>
              <HotkeyPicker
                value={settings.hotkey}
                onChange={(v) => onUpdateSettings({ hotkey: v })}
              />
              <p className="text-xs text-muted">
                {settings.push_to_talk
                  ? "Release to transcribe. Modifier-only combos like Shift+Win are supported."
                  : "Press again to stop."}
              </p>
            </div>
          </section>
        )}

        {step === 2 && (
          <section className="panel p-5 space-y-4">
            <div className="flex items-center gap-2.5">
              <div className="w-8 h-8 rounded-lg bg-accent-soft border border-accent-border text-accent flex items-center justify-center">
                <Download className="w-4 h-4" />
              </div>
              <div>
                <h2 className="text-base font-semibold text-ink">Download the speech model</h2>
                <p className="text-xs text-muted">
                  Reflow transcribes on this computer. The model is required before you can dictate.
                </p>
              </div>
            </div>

            <div className="grid grid-cols-2 gap-2.5">
              {(
                [
                  { id: "0.6b", title: "0.6B", desc: "Faster · smaller" },
                  { id: "1.7b", title: "1.7B", desc: "Higher accuracy" },
                ] as const
              ).map((m) => {
                const active = settings.asr.model === m.id;
                return (
                  <button
                    key={m.id}
                    onClick={() => onUpdateSettings({ asr: { ...settings.asr, model: m.id } })}
                    className={`text-left rounded-xl border p-3 transition-all cursor-pointer ${
                      active
                        ? "border-accent bg-accent-soft"
                        : "border-line bg-surface hover:border-line-strong hover:bg-base-2"
                    }`}
                  >
                    <p className={`text-sm font-semibold ${active ? "text-accent" : "text-ink-2"}`}>
                      {m.title}
                    </p>
                    <p className="text-xs text-muted mt-0.5">{m.desc}</p>
                  </button>
                );
              })}
            </div>

            {installed || ready ? (
              <div className="flex items-center gap-2 text-sm text-success font-medium">
                <Check className="w-4 h-4" />
                {ready ? "Speech model ready" : "Model installed — loading required"}
              </div>
            ) : downloading ? (
              <div>
                <div className="flex items-center gap-2 text-sm text-accent font-medium mb-2">
                  <Loader2 className="w-4 h-4 animate-spin" />
                  Downloading {modelStatus?.download_progress_pct ?? 0}%
                </div>
                <div className="h-1.5 rounded-full bg-line overflow-hidden">
                  <div
                    className="h-full bg-accent rounded-full transition-all"
                    style={{ width: `${modelStatus?.download_progress_pct ?? 0}%` }}
                  />
                </div>
              </div>
            ) : (
              <button
                className="btn btn-primary w-full"
                onClick={downloadModel}
                disabled={installing}
              >
                {installing ? (
                  <Loader2 className="w-4 h-4 animate-spin" />
                ) : (
                  <Download className="w-4 h-4" />
                )}
                {installing ? "Starting download…" : "Download model"}
              </button>
            )}
            {!installing && !downloading && !ready && modelStatus?.error && (
              <p role="alert" className="text-sm text-danger">
                Speech model error: {modelStatus.error}
              </p>
            )}
            {installed && !ready && !modelStatus?.is_loading && (
              <button
                className="btn btn-secondary"
                onClick={async () => {
                  setError(null);
                  try {
                    await api.reloadModel();
                  } catch (failure) {
                    setError(`Could not load the speech model: ${String(failure)}`);
                  }
                }}
              >
                Load speech model
              </button>
            )}
            {ready && (
              <RecognitionCheck
                key={configuration}
                ready={ready && !saving}
                onSuccess={() => setTestedConfiguration(configuration)}
              />
            )}
            {testedConfiguration !== configuration && (
              <p className="text-xs text-muted">
                Finish unlocks after the speech model is ready and a recognition test succeeds.
              </p>
            )}
            <div className="pt-4 border-t border-line space-y-2">
              <h3 className="text-sm font-semibold text-ink">Optional AI writing cleanup</h3>
              <p className="text-xs text-muted">
                An ASR model turns speech into text. An LLM rewrites that text. You can finish setup
                with no LLM and change it later on Home.
              </p>
              <LlmSelector
                settings={settings}
                onUpdateSettings={onUpdateSettings}
                tiers={intelligenceTiers}
                disabled={saving}
              />
            </div>
          </section>
        )}

        {
          <div className="flex items-center justify-between mt-6">
            <button
              className="btn btn-ghost"
              disabled={step === 0}
              onClick={() => setStep((s) => Math.max(0, s - 1))}
            >
              Back
            </button>
            <button className="btn btn-primary" disabled={!canContinue} onClick={goNext}>
              {step === STEPS.length - 1 ? (
                "Finish"
              ) : (
                <>
                  Continue
                  <ChevronRight className="w-4 h-4" />
                </>
              )}
            </button>
          </div>
        }
      </fieldset>
    </div>
  );
};
