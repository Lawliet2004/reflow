import React, { useEffect, useRef, useState } from "react";
import { Check, ChevronRight, Download, Keyboard, Mic, Loader2 } from "lucide-react";
import {
  AppSettings,
  AudioDevice,
  IntelligenceTierState,
  MicrophoneHealth,
  ModelStatus,
  PythonSetupProgress,
  SpeechSetupPlan,
  isModelReady,
} from "../types";
import { api } from "../services/tauriApi";
import { DesktopIntegrationPanel } from "./DesktopIntegrationPanel";
import {
  canCancelSpeechSetup,
  prepareSpeechRuntime,
  SPEECH_SETUP_PHASE,
} from "../services/speechSetup";
import { isModifierOnlyShortcut } from "../services/shortcutKeys";
import { HotkeyPicker } from "./HotkeyPicker";
import { SpeechDependencySetup } from "./SpeechDependencySetup";
import { ShortcutProbe } from "./ShortcutProbe";
import { DictationTrial } from "./DictationTrial";
import { NativeRuntimeSetup } from "./NativeRuntimeSetup";
import {
  dependencyConfiguration,
  microphoneConfiguration,
  readSetupCheckpoint,
  resumeSetupStage,
  shortcutConfiguration,
  speechConfiguration,
  trialConfiguration,
  usableMicrophone,
  writeSetupCheckpoint,
} from "../services/onboarding";
import { SPEECH_MODEL_GUIDANCE } from "../writingTasks";
import logoUrl from "../assets/logo.png";

interface OnboardingProps {
  settings: AppSettings;
  modelStatus: ModelStatus | null;
  onUpdateSettings: (settings: Partial<AppSettings>) => Promise<boolean> | void;
  onComplete: (outcome: "completed" | "deferred") => void;
  intelligenceTiers?: IntelligenceTierState[] | null;
}

const STEPS = [
  { id: "mic", label: "Microphone" },
  { id: "hotkey", label: "Shortcut" },
  { id: "hardware", label: "Computer" },
  { id: "runtime", label: "Dependencies" },
  { id: "model", label: "Speech model" },
  { id: "trial", label: "Try dictation" },
];

export const Onboarding: React.FC<OnboardingProps> = ({
  settings,
  modelStatus,
  onUpdateSettings: persistSettings,
  onComplete,
}) => {
  const selectedModel =
    settings.asr.runtime === "native" ? `native-${settings.asr.model}` : settings.asr.model;
  const ready =
    isModelReady(modelStatus) &&
    !modelStatus?.is_downloading &&
    (!modelStatus?.selected_model || modelStatus.selected_model === selectedModel);
  const [checkpoint] = useState(readSetupCheckpoint);
  const [step, setStep] = useState(() => resumeSetupStage(checkpoint, settings));
  const [microphoneProof, setMicrophoneProof] = useState(checkpoint.microphone);
  const [shortcutProof, setShortcutProof] = useState(checkpoint.shortcut);
  const [deliveryMode, setDeliveryMode] = useState<"inserted" | "clipboard">();
  const [error, setError] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [dependenciesBusy, setDependenciesBusy] = useState(false);
  const [trialBusy, setTrialBusy] = useState(false);
  const [preparedProfile, setPreparedProfile] = useState<string>();
  const runtimePrepared = preparedProfile === dependencyConfiguration(settings);
  const [setupPlan, setSetupPlan] = useState<SpeechSetupPlan | null>(null);
  const [runtimeProgress, setRuntimeProgress] = useState<PythonSetupProgress | null>(null);
  const setupController = useRef<AbortController | null>(null);
  useEffect(() => () => setupController.current?.abort(), []);
  const [downloadsBlocked, setDownloadsBlocked] = useState(false);
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [micTesting, setMicTesting] = useState(false);
  const [micHealth, setMicHealth] = useState<MicrophoneHealth | null>(null);
  const [micLevel, setMicLevel] = useState(0);
  const [testedConfiguration, setTestedConfiguration] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const hotkeyHeard =
    shortcutProof?.mode === "shortcut" &&
    shortcutProof.configuration === shortcutConfiguration(settings);
  const [recommended, setRecommended] = useState<string | null>(null);
  const [importPath, setImportPath] = useState("");
  const [importing, setImporting] = useState(false);
  const userPickedModelRef = useRef(false);
  useEffect(() => {
    writeSetupCheckpoint({
      version: 1,
      stage: step,
      outcome: "in_progress",
      microphone: microphoneProof,
      shortcut: shortcutProof,
      speech: speechConfiguration(settings),
    });
  }, [step, microphoneProof, shortcutProof, settings]);
  const finish = (outcome: "completed" | "deferred") => {
    writeSetupCheckpoint({
      version: 1,
      stage: step,
      outcome,
      microphone: microphoneProof,
      shortcut: shortcutProof,
      speech: speechConfiguration(settings),
      ...(outcome === "completed" ? { delivery: deliveryMode } : {}),
    });
    onComplete(outcome);
  };
  const onUpdateSettings = async (changes: Partial<AppSettings>) => {
    if (
      changes.asr ||
      "microphone_device_id" in changes ||
      "input_gain" in changes ||
      "language" in changes
    ) {
      setTestedConfiguration(null);
      if ("microphone_device_id" in changes || "input_gain" in changes) {
        setMicHealth(null);
        setMicrophoneProof(undefined);
      }
    }
    if (changes.hotkey || "push_to_talk" in changes) setShortcutProof(undefined);
    setSaving(true);
    setError(null);
    try {
      if ((await persistSettings(changes)) === false) {
        setError("Settings could not be saved. Try again before testing recognition.");
        return false;
      }
      return true;
    } catch (failure) {
      setError(
        changes.hotkey
          ? "Couldn’t save that shortcut. Try another combination."
          : `Settings could not be saved: ${String(failure)}`,
      );
      return false;
    } finally {
      setSaving(false);
    }
  };
  const configuration = trialConfiguration(settings);

  const installed = Boolean(modelStatus?.installed);
  const downloading = Boolean(modelStatus?.is_downloading);
  const runtimeMissing =
    !runtimePrepared &&
    settings.asr.runtime === "python" &&
    (modelStatus?.failure_kind === "python_missing" ||
      modelStatus?.failure_kind === "missing_dependency");

  // Hardware-based model suggestion: the resolver recommends 0.6B whenever
  // the 1.7B manifest estimates don't fit, and the card gets the badge.
  useEffect(() => {
    let alive = true;
    void api
      .recommendedAsrModel()
      .then((id) => {
        if (!alive) return;
        setRecommended(id);
        // Preselect only while the stored choice is still the factory
        // default — a user's explicit pick is never overwritten.
        if (
          (id === "0.6b" || id === "zipformer-20m") &&
          !installed &&
          !userPickedModelRef.current
        ) {
          // Read the current settings, not the mount-time snapshot — an asr
          // edit between mount and now must not be clobbered.
          void api.getSettings().then((current) => {
            if (!alive || userPickedModelRef.current) return;
            // A lightweight English engine must not silently replace a
            // multilingual choice. Its card remains available for an explicit pick.
            if (
              current.asr.model === "1.7b" &&
              (id !== "zipformer-20m" || current.language === "en")
            ) {
              void persistSettings({
                asr: {
                  ...current.asr,
                  model: id,
                  ...(id === "zipformer-20m"
                    ? { runtime: "python" as const, precision: "auto" as const }
                    : {}),
                },
              });
            }
          });
        }
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    let alive = true;
    const refreshDevices = () =>
      api
        .getAudioDevices()
        .then((list) => {
          if (alive) {
            setDevices(list);
            if (
              settings.microphone_device_id &&
              !list.some(
                (device) =>
                  device.id === settings.microphone_device_id && device.is_available !== false,
              )
            ) {
              setMicrophoneProof(undefined);
              setError(
                "Your selected microphone is unavailable. Reconnect it or choose another microphone.",
              );
              setStep(0);
            }
          }
        })
        .catch(() => {
          if (alive)
            setError("Could not list microphones. Check your audio permissions and reopen setup.");
        });
    void refreshDevices();
    window.addEventListener("focus", refreshDevices);
    return () => {
      alive = false;
      window.removeEventListener("focus", refreshDevices);
    };
  }, [settings.microphone_device_id]);

  useEffect(() => {
    if (!micTesting) return;
    let alive = true;
    const poll = () =>
      api
        .testMicrophoneLevel()
        .then((level) => {
          if (alive) setMicLevel(level);
        })
        .catch(() => {});
    void poll();
    const timer = setInterval(poll, 150);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [micTesting]);

  const testMicrophone = async () => {
    setMicLevel(0);
    setMicTesting(true);
    setMicHealth(null);
    setMicrophoneProof(undefined);
    setError(null);
    try {
      const health = await api.testMicrophone();
      setMicHealth(health);
      if (usableMicrophone(health)) setMicrophoneProof(microphoneConfiguration(settings));
      else setError(health.assessment);
    } catch (failure) {
      setError(`Microphone test failed: ${String(failure)}`);
    } finally {
      setMicTesting(false);
    }
  };

  const pickMic = (id: string) => {
    setMicHealth(null);
    onUpdateSettings({ microphone_device_id: id === "default" ? null : id });
  };

  const downloadModel = async () => {
    if (setupController.current || dependenciesBusy) return;
    const controller = new AbortController();
    setupController.current = controller;
    setError(null);
    setInstalling(true);
    setRuntimeProgress(null);
    try {
      if (settings.asr.runtime === "python") {
        // Refresh immediately before downloading: the check shown earlier may
        // predate a driver change or files consuming the remaining storage.
        const [plan, status] = await Promise.all([
          api.getSpeechSetupPlan(),
          api.getPythonRuntimeStatus(),
        ]);
        controller.signal.throwIfAborted();
        if (plan) {
          if (plan.blocker && (!status.managed_installed || runtimeMissing))
            throw new Error(plan.blocker);
          if (!plan.supported_models.includes(settings.asr.model)) {
            throw new Error(
              "This computer supports Zipformer speech. Choose Zipformer below to continue.",
            );
          }
        }
        await prepareSpeechRuntime(
          controller.signal,
          setRuntimeProgress,
          runtimePrepared ? null : modelStatus?.failure_kind,
        );
        setPreparedProfile(dependencyConfiguration(settings));
      }
      controller.signal.throwIfAborted();
      setRuntimeProgress(null);
      if (installed) await api.reloadModel();
      else await api.installModel(settings.asr.model);
    } catch (e) {
      if (controller.signal.aborted || (e instanceof DOMException && e.name === "AbortError"))
        return;
      if (String(e).includes("Offline mode is on")) {
        setDownloadsBlocked(true);
      } else {
        setError(
          `Could not set up dictation: ${e instanceof Error ? e.message : String(e)}. You can retry setup below.`,
        );
      }
    } finally {
      if (!controller.signal.aborted) {
        setInstalling(false);
        setRuntimeProgress(null);
      }
      setupController.current = null;
    }
  };

  const allowDownloads = async () => {
    setError(null);
    if (await onUpdateSettings({ offline_mode: false })) {
      setDownloadsBlocked(false);
      void downloadModel();
    }
  };

  const importModel = async () => {
    const path = importPath.trim();
    if (!path) return;
    setError(null);
    setImporting(true);
    try {
      const imported = await api.importModelFile(path);
      // Point the selection at what was imported: native GGUFs get the
      // native runtime, everything else stays on the Python sidecar.
      const native = imported.id.startsWith("native-");
      await onUpdateSettings({
        asr: {
          ...settings.asr,
          model: native ? imported.id.slice(7) : imported.id,
          ...(native ? { runtime: "native" as const } : { runtime: "python" as const }),
        },
      });
      setDownloadsBlocked(false);
    } catch (failure) {
      setError(`Could not import the model: ${String(failure)}`);
    } finally {
      setImporting(false);
    }
  };

  const canContinue =
    !saving &&
    !micTesting &&
    !dependenciesBusy &&
    !trialBusy &&
    !installing &&
    !importing &&
    (step === 0
      ? microphoneProof === microphoneConfiguration(settings)
      : step === 1
        ? shortcutProof?.configuration === shortcutConfiguration(settings)
        : step === 2
          ? Boolean(
              setupPlan &&
              (settings.asr.runtime === "native" ||
                (!setupPlan.blocker && setupPlan.supported_models.includes(settings.asr.model))),
            )
          : step === 3
            ? runtimePrepared
            : step === 4
              ? ready
              : ready && testedConfiguration === configuration);

  const nativeRuntime = settings.asr.runtime === "native";
  const speechModels = [
    {
      id: "0.6b",
      title: "Qwen3-ASR 0.6B",
      desc: "Multilingual · faster · smaller",
      size: nativeRuntime ? "1.0 GB" : "1.6 GB",
    },
    {
      id: "1.7b",
      title: "Qwen3-ASR 1.7B",
      desc: "Multilingual · higher accuracy",
      size: nativeRuntime ? "2.5 GB" : "4.1 GB",
    },
    { id: "phonon-2", title: "Phonon-2", desc: "English only · runs on CPU", size: "164 MB" },
    {
      id: "zipformer-20m",
      title: "Zipformer 20M INT8",
      desc: "English only · CPU streaming",
      size: "44 MB",
    },
  ] as const;

  const goNext = () => {
    if (step >= STEPS.length - 1) {
      finish("completed");
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
          <p role="alert" className="text-sm text-danger mb-4 break-words whitespace-pre-line">
            {error}
          </p>
        )}
        <img
          src={logoUrl}
          alt=""
          aria-hidden
          className="w-11 h-11 mb-5 pointer-events-none"
          draggable={false}
        />
        <h1 className="font-display text-3xl font-medium text-ink">
          A few things. Then just speak.
        </h1>
        <p className="text-sm text-muted mt-1">
          We’ll check your input, prepare speech, and try a real dictation.
        </p>

        <ol
          className="grid grid-cols-3 sm:grid-cols-6 gap-x-3 gap-y-4 mt-6 mb-8"
          aria-label="Setup progress"
        >
          {STEPS.map((s, i) => (
            <li key={s.id} aria-current={i === step ? "step" : undefined}>
              <div className={`h-1 rounded-full ${i <= step ? "bg-accent" : "bg-line"}`} />
              <p
                className={`text-2xs mt-1.5 truncate ${
                  i === step ? "text-accent font-medium" : "text-muted"
                }`}
              >
                {s.label}
              </p>
            </li>
          ))}
        </ol>

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
              {settings.microphone_device_id &&
                !devices.some((d) => d.id === settings.microphone_device_id) && (
                  <option value={settings.microphone_device_id}>
                    Selected microphone unavailable
                  </option>
                )}
              {devices
                .filter(
                  (d) =>
                    d.id !== "default" &&
                    !d.is_monitor &&
                    !d.name.toLowerCase().includes("loopback"),
                )
                .map((d) => (
                  <option key={d.id} value={d.id} disabled={d.is_available === false}>
                    {d.name}
                    {d.is_available === false
                      ? " (unplugged)"
                      : d.is_default
                        ? " · System default"
                        : ""}
                  </option>
                ))}
            </select>
            <div className="space-y-2">
              <button className="btn btn-secondary" disabled={micTesting} onClick={testMicrophone}>
                {micTesting ? "Listening for 3 seconds…" : "Test microphone"}
              </button>
              {micTesting && (
                <div
                  className="h-1.5 rounded-full bg-line overflow-hidden"
                  aria-label="Microphone level"
                >
                  <div
                    className="progress-fill bg-accent"
                    style={{
                      transform: `scaleX(${Math.max(0, Math.min(1, micLevel))})`,
                    }}
                  />
                </div>
              )}
              <div aria-live="polite">
                {micTesting && (
                  <p className="text-sm text-muted">Speak normally into your microphone now.</p>
                )}
                {micHealth && (
                  <div className="text-sm text-muted space-y-1">
                    <p>{micHealth.assessment}</p>
                    <p>
                      {micHealth.device} · {(micHealth.duration_ms / 1000).toFixed(1)} s · Peak{" "}
                      {Math.round(micHealth.peak * 100)}% · Clipped{" "}
                      {micHealth.clipped_pct.toFixed(1)}% · {micHealth.dropped_chunks} dropped
                      frames
                    </p>
                  </div>
                )}
              </div>
            </div>
          </section>
        )}

        {step === 1 && (
          <section className="panel p-5 space-y-4">
            <div className="flex items-center gap-2.5">
              <div className="w-8 h-8 rounded-lg bg-accent-soft border border-accent-border text-accent flex items-center justify-center">
                <Keyboard className="w-4 h-4" />
              </div>
              <div>
                <h2 className="text-base font-semibold text-ink">Your dictation shortcut</h2>
                <p className="text-xs text-muted">Click the shortcut to change it.</p>
              </div>
            </div>
            <DesktopIntegrationPanel
              compact
              onShortcutSaved={() => setShortcutProof(undefined)}
              renderShortcuts={({ status, busy, configure, setShortcut }) => (
                <div className="py-4 flex flex-col items-center gap-3">
                  <p className="text-xs text-muted">
                    {status?.session === "wayland" &&
                    isModifierOnlyShortcut(
                      status.assigned_shortcuts.dictate ?? status.shortcut.assigned_trigger,
                    )
                      ? "Tap to speak. Tap again to finish."
                      : settings.push_to_talk
                        ? "Hold to speak. Release to finish."
                        : "Press to speak. Press again to finish."}
                  </p>
                  {status?.session === "wayland" && status.shortcut_editable ? (
                    <HotkeyPicker
                      value={
                        status.assigned_shortcuts.dictate ??
                        status.shortcut.assigned_trigger ??
                        "Choose shortcut…"
                      }
                      onChange={(key) => setShortcut("dictation", key)}
                      disabled={
                        busy ||
                        (status.shortcut.state !== "ready" &&
                          status.shortcut.state !== "unassigned")
                      }
                      allowModifierOnly
                      ariaLabel="Change dictation shortcut"
                    />
                  ) : status?.session === "wayland" ? (
                    <button
                      type="button"
                      className="field !text-center"
                      disabled={busy}
                      onClick={configure}
                    >
                      {status.assigned_shortcuts.dictate ??
                        status.assigned_shortcuts.dictation ??
                        status.shortcut.assigned_trigger ??
                        "Choose shortcut…"}
                    </button>
                  ) : (
                    <HotkeyPicker
                      value={settings.hotkey}
                      onChange={(value) => {
                        setShortcutProof(undefined);
                        void onUpdateSettings({ hotkey: value });
                      }}
                      disabled={busy || !status}
                      ariaLabel="Change dictation shortcut"
                    />
                  )}
                </div>
              )}
            />
            <ShortcutProbe
              key={settings.hotkey}
              onSuccess={() =>
                setShortcutProof({
                  configuration: shortcutConfiguration(settings),
                  mode: "shortcut",
                })
              }
            />
            <div aria-live="polite">
              {hotkeyHeard ? (
                <p className="text-sm text-success font-medium flex items-center gap-1.5">
                  <Check className="w-4 h-4" aria-hidden />
                  Shortcut works. You’re ready to continue.
                </p>
              ) : (
                <p className="text-xs text-muted">
                  Test the shortcut before installing speech. This test does not record audio.
                </p>
              )}
            </div>
            {!hotkeyHeard && (
              <button
                type="button"
                className="btn btn-ghost"
                onClick={() =>
                  setShortcutProof({
                    configuration: shortcutConfiguration(settings),
                    mode: "button",
                  })
                }
              >
                Use the Record button instead
              </button>
            )}
            {shortcutProof?.mode === "button" && (
              <p role="status" className="text-xs text-muted">
                Record button selected. A global shortcut has not been verified.
              </p>
            )}
          </section>
        )}

        {step === 2 && (
          <section className="panel p-5 space-y-4">
            <div className="flex items-center gap-2.5">
              <div className="w-8 h-8 rounded-lg bg-accent-soft border border-accent-border text-accent flex items-center justify-center">
                <Download className="w-4 h-4" />
              </div>
              <div>
                <h2 className="text-base font-semibold text-ink">Check your computer</h2>
                <p className="text-xs text-muted">
                  Review the suggested speech model and the hardware it will use.
                </p>
              </div>
            </div>

            <SpeechDependencySetup
              key={`hardware:${dependencyConfiguration(settings)}`}
              settings={settings}
              failureKind={modelStatus?.failure_kind}
              inspectionOnly
              disabled={saving}
              onUpdateSettings={onUpdateSettings}
              onPlan={setSetupPlan}
            />
            <h3 className="text-sm font-semibold text-ink">Choose a speech model</h3>
            <p className="text-xs text-muted">
              Choose a model for your language and computer. Dependencies install next, followed by
              model weights.
            </p>
            {!nativeRuntime && setupPlan?.blocker && (
              <p role="alert" className="text-sm text-danger whitespace-pre-line break-words">
                {setupPlan.blocker}
              </p>
            )}
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
              {speechModels.map((m) => {
                const active = settings.asr.model === m.id;
                return (
                  <button
                    key={m.id}
                    aria-pressed={active}
                    disabled={
                      installing ||
                      dependenciesBusy ||
                      downloading ||
                      Boolean(modelStatus?.is_loading) ||
                      (settings.asr.runtime === "python" &&
                        Boolean(setupPlan && !setupPlan.supported_models.includes(m.id)))
                    }
                    onClick={() => {
                      userPickedModelRef.current = true;
                      onUpdateSettings({
                        asr: {
                          ...settings.asr,
                          model: m.id,
                          ...(m.id === "phonon-2" || m.id === "zipformer-20m"
                            ? { runtime: "python" as const, precision: "auto" as const }
                            : {}),
                        },
                      });
                    }}
                    className={`text-left rounded-xl border p-3 transition-colors cursor-pointer ${
                      active
                        ? "border-accent bg-accent-soft"
                        : "border-line bg-surface hover:border-line-strong hover:bg-base-2"
                    }`}
                  >
                    <p
                      className={`text-sm font-semibold flex items-center gap-1.5 ${active ? "text-accent" : "text-ink-2"}`}
                    >
                      {m.title}
                      {recommended === m.id && (
                        <span className="text-2xs font-medium text-accent bg-accent-soft border border-accent-border rounded-full px-1.5 py-0.5">
                          Recommended
                        </span>
                      )}
                    </p>
                    <p className="text-xs text-muted mt-0.5">{m.desc}</p>
                    <p className="text-2xs text-muted mt-1">About {m.size} download</p>
                  </button>
                );
              })}
            </div>
          </section>
        )}

        {step === 3 && (
          <section className="panel p-5 space-y-4">
            <h2 className="text-base font-semibold text-ink">Prepare speech dependencies</h2>
            {settings.asr.runtime === "python" ? (
              <SpeechDependencySetup
                key={dependencyConfiguration(settings)}
                settings={settings}
                failureKind={modelStatus?.failure_kind}
                onUpdateSettings={onUpdateSettings}
                onBusyChange={setDependenciesBusy}
                onPlan={setSetupPlan}
                onReady={() => setPreparedProfile(dependencyConfiguration(settings))}
              />
            ) : (
              <NativeRuntimeSetup
                settings={settings}
                onUpdateSettings={onUpdateSettings}
                onBusyChange={setDependenciesBusy}
                onReady={() => setPreparedProfile(dependencyConfiguration(settings))}
              />
            )}
          </section>
        )}

        {step === 4 && (
          <section className="panel p-5 space-y-4">
            <h2 className="text-base font-semibold text-ink">Download your speech model</h2>
            <p className="text-sm text-muted">
              {speechModels.find((model) => model.id === settings.asr.model)?.title ??
                settings.asr.model}
              . Your runtime will be checked again before the download starts.
            </p>
            {downloading && (
              <button
                className="btn btn-secondary"
                onClick={() => {
                  api
                    .cancelModelDownload(settings.asr.model)
                    .catch((failure) => setError(`Could not cancel download: ${String(failure)}`));
                }}
              >
                Cancel model download
              </button>
            )}
            {installing && runtimeProgress ? (
              <div role="status" className="space-y-2">
                <p className="text-sm text-accent flex items-center gap-2">
                  <Loader2 className="w-4 h-4 animate-spin" aria-hidden />
                  {SPEECH_SETUP_PHASE[runtimeProgress.phase]}
                  {typeof runtimeProgress.progress_pct === "number"
                    ? ` · ${runtimeProgress.progress_pct}%`
                    : "…"}
                </p>
                <button
                  className="btn btn-secondary"
                  disabled={!canCancelSpeechSetup(runtimeProgress)}
                  onClick={() => {
                    api.cancelPythonRuntimeSetup(runtimeProgress?.attempt_id).catch((failure) => {
                      setError(`Could not cancel setup: ${String(failure)}`);
                    });
                  }}
                >
                  Cancel setup
                </button>
                {runtimeProgress.detail?.startsWith("Downloading a speech package:") && (
                  <p className="text-xs text-muted tabular-nums">{runtimeProgress.detail}</p>
                )}
              </div>
            ) : ready ||
              (installed && !(runtimeMissing && (settings.offline_mode || downloadsBlocked))) ? (
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
                    className="progress-fill bg-accent"
                    style={{
                      transform: `scaleX(${Math.max(0, Math.min(100, modelStatus?.download_progress_pct ?? 0)) / 100})`,
                    }}
                  />
                </div>
              </div>
            ) : settings.offline_mode || downloadsBlocked ? (
              <div className="rounded-xl border border-line bg-base-2 p-4 space-y-3">
                <p className="text-sm font-medium text-ink-2">
                  Reflow needs internet once to set up dictation.
                </p>
                <p className="text-xs text-muted leading-relaxed">
                  This downloads the speech runtime and selected model from their configured
                  sources. Transcription stays on this computer. You can turn Airplane mode back on
                  afterwards.
                </p>
                <button
                  className="btn btn-primary w-full"
                  onClick={allowDownloads}
                  disabled={installing || dependenciesBusy || saving}
                >
                  {installing ? (
                    <Loader2 className="w-4 h-4 animate-spin" />
                  ) : (
                    <Download className="w-4 h-4" />
                  )}
                  {installing ? "Setting up…" : "Allow downloads and set up"}
                </button>
                <div className="space-y-1.5">
                  <p className="text-2xs text-muted">
                    Or bring your own copy — no internet needed:
                  </p>
                  <div className="flex gap-2">
                    <input
                      className="field flex-1"
                      aria-label="Model import path"
                      placeholder="Path to a model folder or GGUF file"
                      value={importPath}
                      onChange={(event) => setImportPath(event.target.value)}
                    />
                    <button
                      className="btn btn-secondary"
                      disabled={importing || !importPath.trim()}
                      onClick={importModel}
                    >
                      {importing ? "Importing…" : "Import"}
                    </button>
                  </div>
                </div>
              </div>
            ) : (
              <button
                className="btn btn-primary w-full"
                onClick={downloadModel}
                disabled={installing || dependenciesBusy}
              >
                {installing ? (
                  <Loader2 className="w-4 h-4 animate-spin" />
                ) : (
                  <Download className="w-4 h-4" />
                )}
                {installing ? "Setting up…" : error ? "Retry setup" : "Download and set up"}
              </button>
            )}
            {!installing && !downloading && !ready && !error && modelStatus?.error && (
              <p role="alert" className="text-sm text-danger">
                {runtimeMissing
                  ? "The speech runtime needs setup. Use the setup button to install its dependencies automatically."
                  : `Speech model error: ${modelStatus.error}`}
              </p>
            )}
            {installed &&
              !ready &&
              !installing &&
              !(runtimeMissing && (settings.offline_mode || downloadsBlocked)) &&
              !modelStatus?.is_loading && (
                <button
                  className="btn btn-secondary"
                  onClick={() => {
                    if (settings.offline_mode && !runtimeMissing) {
                      api.reloadModel().catch((failure) => {
                        setError(`Could not load the speech model: ${String(failure)}`);
                      });
                    } else {
                      void downloadModel();
                    }
                  }}
                >
                  Set up and load speech model
                </button>
              )}
            <details className="text-xs text-muted">
              <summary className="cursor-pointer">Help choosing a speech model</summary>
              <p className="mt-2 leading-relaxed">{SPEECH_MODEL_GUIDANCE}</p>
            </details>
          </section>
        )}

        {step === 5 && (
          <section className="panel p-5 space-y-4">
            <h2 className="text-base font-semibold text-ink">Try your first dictation</h2>
            {!ready ? (
              <p role="alert" className="text-sm text-danger">
                Speech is no longer ready. Go back to check and load the model.
              </p>
            ) : (
              <DictationTrial
                key={configuration}
                onBusyChange={setTrialBusy}
                onReset={() => {
                  setTestedConfiguration(null);
                  setDeliveryMode(undefined);
                }}
                onSuccess={async (mode) => {
                  if (
                    mode === "clipboard" &&
                    !(await onUpdateSettings({ output_action: { type: "copy" } }))
                  ) {
                    throw new Error("Clipboard mode could not be saved. Try again.");
                  }
                  setTestedConfiguration(configuration);
                  setDeliveryMode(mode);
                }}
              />
            )}
            <p className="text-xs text-muted">
              Finish unlocks after a real dictation is recognized and insertion is confirmed, or you
              explicitly choose clipboard mode.
            </p>
            <p className="text-xs text-muted">
              Optional: a local writing model can rewrite your dictation — enable it later in
              Settings → Cleanup.
            </p>
          </section>
        )}

        <div className="flex items-center justify-between mt-6">
          <div className="flex items-center gap-2">
            <button
              className="btn btn-ghost"
              disabled={
                step === 0 ||
                installing ||
                dependenciesBusy ||
                trialBusy ||
                micTesting ||
                downloading
              }
              onClick={() => setStep((s) => Math.max(0, s - 1))}
            >
              Back
            </button>
            <button
              className="btn btn-ghost"
              disabled={installing || dependenciesBusy || trialBusy || micTesting}
              onClick={() => finish("deferred")}
            >
              Set up later
            </button>
          </div>
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
      </fieldset>
    </div>
  );
};
