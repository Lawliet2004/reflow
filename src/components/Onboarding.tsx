import React, { useEffect, useState } from "react";
import { Check, ChevronRight, Download, Keyboard, Mic, Sparkles, Loader2, ArrowRight } from "lucide-react";
import { AppSettings, AudioDevice, ModelStatus, isModelReady } from "../types";
import { api } from "../services/tauriApi";
import { HotkeyPicker } from "./HotkeyPicker";

interface OnboardingProps {
  settings: AppSettings;
  modelStatus: ModelStatus | null;
  onUpdateSettings: (settings: Partial<AppSettings>) => void;
  onComplete: () => void;
}

const STEPS = [
  { id: "mic", label: "Microphone" },
  { id: "hotkey", label: "Hotkey" },
  { id: "model", label: "Speech model" },
];

export const Onboarding: React.FC<OnboardingProps> = ({
  settings,
  modelStatus,
  onUpdateSettings,
  onComplete,
}) => {
  const [step, setStep] = useState(0);
  const [devices, setDevices] = useState<AudioDevice[]>([]);

  const installed = Boolean(modelStatus?.installed);
  const downloading = Boolean(modelStatus?.is_downloading);
  const ready = isModelReady(modelStatus);

  useEffect(() => {
    api.getAudioDevices().then(setDevices).catch(() => {});
  }, []);

  const pickMic = (id: string) => {
    const deviceId = id === "default" ? "default" : id;
    api.setAudioDevice(deviceId).catch(() => {});
    onUpdateSettings({ microphone_device_id: id === "default" ? null : id });
  };

  const downloadModel = async () => {
    try {
      await api.installModel(settings.asr_model);
    } catch (e) {
      console.error("Model install failed:", e);
    }
  };

  const canContinue =
    step === 0 ||
    step === 1 ||
    (step === 2 && (installed || ready));

  const goNext = () => {
    if (step >= STEPS.length - 1) {
      onComplete();
      return;
    }
    setStep((s) => s + 1);
  };

  const isDone = step === STEPS.length; // success screen

  return (
    <div className="max-w-xl mx-auto px-7 py-10 animate-fade-rise">
      <p className="label-micro text-accent mb-2">First run</p>
      <h1 className="font-display text-[22px] font-semibold tracking-tight text-ink">
        Set up Reflow
      </h1>
      <p className="text-[13px] text-muted mt-1">
        Three quick steps so dictation is ready on this computer.
      </p>

      <div className="flex items-center gap-2 mt-6 mb-8">
        {STEPS.map((s, i) => (
          <div key={s.id} className="flex-1">
            <div className={`h-1 rounded-full ${i <= step ? "bg-accent" : "bg-line"}`} />
            <p
              className={`text-[11px] mt-1.5 truncate ${
                i === step ? "text-accent font-medium" : "text-muted"
              }`}
            >
              {s.label}
            </p>
          </div>
        ))}
      </div>

      {!isDone && step === 0 && (
        <section className="panel p-5 space-y-4">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-accent-soft border border-accent-border text-accent flex items-center justify-center">
              <Mic className="w-4 h-4" />
            </div>
            <div>
              <h2 className="text-[14.5px] font-semibold text-ink">Choose a microphone</h2>
              <p className="text-[12px] text-muted">You can change this later in Settings → Audio.</p>
            </div>
          </div>
          <select
            className="field w-full"
            value={settings.microphone_device_id ?? "default"}
            onChange={(e) => pickMic(e.target.value)}
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
        </section>
      )}

      {!isDone && step === 1 && (
        <section className="panel p-5 space-y-4">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-accent-soft border border-accent-border text-accent flex items-center justify-center">
              <Keyboard className="w-4 h-4" />
            </div>
            <div>
              <h2 className="text-[14.5px] font-semibold text-ink">Pick your shortcut</h2>
              <p className="text-[12px] text-muted">
                Hold it to record anywhere. You can change it later in Settings.
              </p>
            </div>
          </div>
          <div className="rounded-xl border border-accent-border bg-accent-soft px-4 py-5 flex flex-col items-center gap-3">
            <p className="text-[12px] text-muted">Press any key combination</p>
            <HotkeyPicker
              value={settings.hotkey}
              onChange={(v) => onUpdateSettings({ hotkey: v })}
            />
            <p className="text-[12px] text-muted">
              {settings.push_to_talk
                ? "Release to transcribe. Modifier-only combos like Shift+Win are supported."
                : "Press again to stop."}
            </p>
          </div>
        </section>
      )}

      {!isDone && step === 2 && (
        <section className="panel p-5 space-y-4">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-accent-soft border border-accent-border text-accent flex items-center justify-center">
              <Download className="w-4 h-4" />
            </div>
            <div>
              <h2 className="text-[14.5px] font-semibold text-ink">Download the speech model</h2>
              <p className="text-[12px] text-muted">
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
              const active = settings.asr_model === m.id;
              return (
                <button
                  key={m.id}
                  onClick={() => onUpdateSettings({ asr_model: m.id })}
                  className={`text-left rounded-xl border p-3 transition-all cursor-pointer ${
                    active
                      ? "border-accent bg-accent-soft"
                      : "border-line bg-surface hover:border-line-strong hover:bg-base-2"
                  }`}
                >
                  <p className={`text-[13px] font-semibold ${active ? "text-accent" : "text-ink-2"}`}>
                    {m.title}
                  </p>
                  <p className="text-[11.5px] text-muted mt-0.5">{m.desc}</p>
                </button>
              );
            })}
          </div>

          {installed || ready ? (
            <div className="flex items-center gap-2 text-[13px] text-emerald-600 dark:text-emerald-400 font-medium">
              <Check className="w-4 h-4" />
              Model installed
            </div>
          ) : downloading ? (
            <div>
              <div className="flex items-center gap-2 text-[13px] text-accent font-medium mb-2">
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
            <button className="btn btn-primary w-full" onClick={downloadModel}>
              <Download className="w-4 h-4" />
              Download model
            </button>
          )}
          {!installed && !ready && (
            <p className="text-[12px] text-muted">
              Continue stays locked until the speech model is on this machine.
            </p>
          )}
        </section>
      )}

      {isDone && (
        <section className="panel p-6 space-y-4 text-center">
          <div className="w-12 h-12 mx-auto rounded-2xl bg-emerald-500/15 border border-emerald-500/30 text-emerald-600 dark:text-emerald-400 flex items-center justify-center">
            <Check className="w-6 h-6" strokeWidth={2.4} />
          </div>
          <div>
            <h2 className="text-[16px] font-semibold text-ink">You're ready to dictate</h2>
            <p className="text-[13px] text-muted mt-1">
              Hold <span className="kbd !text-[11px]">{settings.hotkey}</span> anywhere on your computer to speak.
            </p>
          </div>
          <div className="flex items-center justify-center gap-2 pt-2">
            <button
              className="btn btn-primary"
              onClick={onComplete}
              autoFocus
            >
              Start dictating
              <ArrowRight className="w-4 h-4" />
            </button>
          </div>
        </section>
      )}

      {!isDone && (
        <div className="flex items-center justify-between mt-6">
          <button
            className="btn btn-ghost"
            disabled={step === 0}
            onClick={() => setStep((s) => Math.max(0, s - 1))}
          >
            Back
          </button>
          <button
            className="btn btn-primary"
            disabled={!canContinue}
            onClick={goNext}
          >
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
      )}
    </div>
  );
};
