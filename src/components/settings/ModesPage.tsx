import { useState } from "react";
import {
  AppSettings,
  CleanupLevel,
  defaultModes,
  DictationMode,
  IntelligenceTier,
  Mode,
  TranscriptStyle,
} from "../../types";
import { api } from "../../services/tauriApi";
import { HotkeyPicker } from "../HotkeyPicker";
import { LanguageOptions } from "../LanguageOptions";
import { Section, Row, Toggle } from "./ui";
import { OutputPicker } from "./OutputPicker";

const PRESETS = [
  [
    "Jira ticket",
    "Write a Jira ticket with a concise title, description and acceptance criteria. Use only the supplied facts.",
  ],
  [
    "Commit message",
    "Write a concise commit message with a summary line and optional body. Use only the supplied facts.",
  ],
  [
    "Medical note",
    "Format the supplied facts as a medical note. Do not invent symptoms, diagnoses or treatment.",
  ],
];

export function ModesPage({
  settings,
  onUpdateSettings,
}: {
  settings: AppSettings;
  onUpdateSettings: (patch: Partial<AppSettings>) => Promise<boolean>;
}) {
  const modes = settings.modes ?? defaultModes();
  const [draft, setDraft] = useState<Mode | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sample, setSample] = useState("Please send the release notes to the team tomorrow.");
  const [preview, setPreview] = useState<string | null>(null);
  const persist = async (patch: Partial<AppSettings>, close = false) => {
    setBusy(true);
    setError(null);
    try {
      if (await onUpdateSettings(patch)) {
        if (close) setDraft(null);
      } else setError("Could not save modes. Try again.");
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  const add = (name = "New mode", instructions = "") => {
    setPreview(null);
    setDraft({
      ...structuredClone(defaultModes()[0]),
      id: crypto.randomUUID(),
      name,
      custom_instructions: instructions,
    });
  };
  const update = (patch: Partial<Mode>) => {
    if (draft) setDraft({ ...draft, ...patch });
  };
  const saveDraft = () => {
    if (!draft) return;
    const shortcut = draft.hotkey;
    const assigned = [
      settings.hotkey,
      settings.hotkeys?.command,
      settings.hotkeys?.assistant,
      settings.hotkeys?.note,
      ...modes.filter((m) => m.id !== draft.id && m.enabled).map((m) => m.hotkey),
    ];
    if (shortcut && assigned.some((s) => s?.toLowerCase() === shortcut.toLowerCase())) {
      setError(`Hotkey ${shortcut} is already assigned.`);
      return;
    }
    const patch: Partial<AppSettings> = {
      modes: modes.some((m) => m.id === draft.id)
        ? modes.map((m) => (m.id === draft.id ? draft : m))
        : [...modes, draft],
    };
    if (draft.id === "dictation")
      Object.assign(patch, {
        style: draft.style,
        cleanup_level: draft.cleanup_level,
        intelligence_tier: draft.intelligence_tier,
        dictation_mode: draft.dictation_mode,
        output_action: draft.output,
        send_key: draft.send_key,
        language: draft.language ?? settings.language,
        auto_detect_language: draft.language === null || draft.language === "auto",
      });
    void persist(patch, true);
  };
  return (
    <Section
      title="Modes"
      description="Choose how a dictation is processed, what local context it can read and where the result goes."
    >
      <fieldset disabled={busy} className="space-y-4 min-w-0">
        {error && (
          <p role="alert" className="text-sm text-danger">
            {error}
          </p>
        )}
        <Row label="Default mode">
          <select
            className="select-field"
            aria-label="Default mode"
            value={settings.default_mode_id ?? "dictation"}
            onChange={(e) => {
              void persist({ default_mode_id: e.target.value });
            }}
          >
            {modes
              .filter((m) => m.enabled)
              .map((m) => (
                <option value={m.id} key={m.id}>
                  {m.name}
                </option>
              ))}
          </select>
        </Row>
        <Row label="App and spoken triggers">
          <Toggle
            on={settings.mode_triggers_enabled ?? true}
            ariaLabel="Enable mode triggers"
            onChange={(mode_triggers_enabled) => {
              void persist({ mode_triggers_enabled });
            }}
          />
        </Row>
        <div className="flex flex-wrap gap-2">
          <button className="btn btn-primary" onClick={() => add()}>
            Add mode
          </button>
          {PRESETS.map(([name, instruction]) => (
            <button key={name} className="btn btn-ghost" onClick={() => add(name, instruction)}>
              {name}
            </button>
          ))}
        </div>
        {modes.map((mode) => (
          <div
            key={mode.id}
            className="flex flex-wrap items-center gap-3 border-b border-border pb-3"
          >
            <div className="flex-1 min-w-32">
              <strong className="text-sm">{mode.name}</strong>
              <p className="text-xs text-muted">
                {mode.hotkey ?? "No shortcut"} · {mode.output.type.replace(/_/g, " ")}
              </p>
            </div>
            <Toggle
              on={mode.enabled}
              ariaLabel={`Enable ${mode.name}`}
              onChange={(enabled) => {
                void persist({
                  modes: modes.map((m) => (m.id === mode.id ? { ...m, enabled } : m)),
                });
              }}
            />
            <button
              className="btn btn-ghost"
              onClick={() => {
                setPreview(null);
                setDraft(
                  mode.id === "dictation"
                    ? {
                        ...structuredClone(mode),
                        style: settings.style,
                        cleanup_level: settings.cleanup_level,
                        intelligence_tier: settings.intelligence_tier,
                        dictation_mode: settings.dictation_mode,
                        language: settings.auto_detect_language ? "auto" : settings.language,
                        output: settings.output_action ?? { type: "paste" },
                        send_key: settings.send_key ?? "enter",
                      }
                    : structuredClone(mode),
                );
              }}
            >
              Edit
            </button>
            <button
              className="btn btn-ghost"
              aria-label={`Duplicate ${mode.name}`}
              onClick={() =>
                setDraft({
                  ...structuredClone(mode),
                  id: crypto.randomUUID(),
                  name: `${mode.name} copy`,
                  hotkey: null,
                })
              }
            >
              Duplicate
            </button>
            {mode.id !== "dictation" && (
              <button
                className="btn btn-ghost"
                aria-label={`Delete ${mode.name}`}
                onClick={() => {
                  void persist({
                    modes: modes.filter((m) => m.id !== mode.id),
                    ...(settings.default_mode_id === mode.id
                      ? { default_mode_id: "dictation" }
                      : {}),
                  });
                }}
              >
                Delete
              </button>
            )}
          </div>
        ))}
        {draft && (
          <form
            className="space-y-4 rounded-lg border border-border p-4"
            onSubmit={(e) => {
              e.preventDefault();
              saveDraft();
            }}
          >
            <h3 className="text-sm font-semibold">Edit mode</h3>
            <label className="block text-sm">
              Mode name
              <input
                className="field w-full mt-1"
                required
                maxLength={80}
                value={draft.name}
                onChange={(e) => update({ name: e.target.value })}
              />
            </label>
            <Row label="Mode shortcut">
              <div className="flex gap-2">
                <HotkeyPicker
                  value={draft.hotkey ?? ""}
                  onChange={(hotkey) => update({ hotkey })}
                />
                <button
                  type="button"
                  className="btn btn-ghost"
                  onClick={() => update({ hotkey: null })}
                >
                  Clear
                </button>
              </div>
            </Row>
            <label className="block text-sm">
              Translate output to
              <select
                className="field w-full mt-1"
                value={draft.translate_to ?? ""}
                onChange={(e) => update({ translate_to: e.target.value || null })}
              >
                <option value="">Keep original language</option>
                <LanguageOptions includeAuto={false} />
              </select>
            </label>
            <label className="block text-sm">
              App triggers (one process per line)
              <textarea
                className="field w-full mt-1"
                value={draft.triggers.apps.join("\n")}
                onChange={(e) =>
                  update({ triggers: { ...draft.triggers, apps: e.target.value.split("\n") } })
                }
              />
            </label>
            <label className="block text-sm">
              Spoken triggers (one phrase per line)
              <textarea
                className="field w-full mt-1"
                value={draft.triggers.spoken.join("\n")}
                onChange={(e) =>
                  update({ triggers: { ...draft.triggers, spoken: e.target.value.split("\n") } })
                }
              />
            </label>
            <Row label="Language">
              <select
                className="select-field"
                aria-label="Mode language"
                value={draft.language ?? "auto"}
                onChange={(e) => update({ language: e.target.value })}
              >
                <LanguageOptions />
              </select>
            </Row>
            <Row label="Dictation mode">
              <select
                className="select-field"
                aria-label="Dictation mode"
                value={draft.dictation_mode}
                onChange={(e) => update({ dictation_mode: e.target.value as DictationMode })}
              >
                {["normal", "coding", "email", "chat", "notes"].map((value) => (
                  <option key={value}>{value}</option>
                ))}
              </select>
            </Row>
            <Row label="Intelligence">
              <select
                className="select-field"
                aria-label="Mode intelligence"
                value={draft.intelligence_tier}
                onChange={(e) => update({ intelligence_tier: e.target.value as IntelligenceTier })}
              >
                <option value="raw_verbatim">Fast</option>
                <option value="smart_flow">Polished</option>
                <option value="deep_context">Polished · Deep</option>
              </select>
            </Row>
            <Row label="Cleanup">
              <select
                className="select-field"
                aria-label="Mode cleanup"
                value={draft.cleanup_level}
                onChange={(e) => update({ cleanup_level: e.target.value as CleanupLevel })}
              >
                {["raw", "light", "medium", "high"].map((value) => (
                  <option key={value}>{value}</option>
                ))}
              </select>
            </Row>
            <Row label="Style">
              <select
                className="select-field"
                aria-label="Mode style"
                value={draft.style}
                onChange={(e) => update({ style: e.target.value as TranscriptStyle })}
              >
                {["faithful", "neutral", "decisive", "email", "chat"].map((value) => (
                  <option key={value}>{value}</option>
                ))}
              </select>
            </Row>
            <label className="block text-sm">
              Custom instructions
              <textarea
                className="field w-full mt-1 min-h-24"
                maxLength={2000}
                value={draft.custom_instructions}
                onChange={(e) => update({ custom_instructions: e.target.value })}
              />
            </label>
            <p className="text-xs text-muted">
              Coding skips AI transforms. Context is read only when you enable it below and stays on
              your computer.
            </p>
            {(["selected_text", "clipboard", "window_title"] as const).map((key) => (
              <Row key={key} label={`Read ${key.replace(/_/g, " ")}`}>
                <Toggle
                  on={draft.context[key]}
                  ariaLabel={`Read ${key.replace(/_/g, " ")}`}
                  onChange={(value) => update({ context: { ...draft.context, [key]: value } })}
                />
              </Row>
            ))}
            <OutputPicker
              value={draft.output}
              sendKey={draft.send_key}
              onChange={(output) => update({ output })}
              onSendKey={(send_key) => update({ send_key })}
            />
            <label className="block text-sm">
              Preview text
              <textarea
                className="field w-full mt-1"
                value={sample}
                onChange={(e) => setSample(e.target.value)}
              />
            </label>
            <button
              type="button"
              className="btn btn-ghost"
              onClick={async () => {
                setBusy(true);
                setError(null);
                try {
                  const result = await api.previewCleanup(
                    sample,
                    draft.intelligence_tier,
                    draft.style,
                    draft,
                  );
                  setPreview(result.text);
                } catch (error) {
                  setError(String(error));
                } finally {
                  setBusy(false);
                }
              }}
            >
              Preview
            </button>
            {preview !== null && (
              <p role="status" className="text-sm whitespace-pre-wrap">
                {preview}
              </p>
            )}
            <div className="flex gap-2">
              <button className="btn btn-primary" type="submit">
                Save mode
              </button>
              <button className="btn btn-ghost" type="button" onClick={() => setDraft(null)}>
                Cancel
              </button>
            </div>
          </form>
        )}
      </fieldset>
    </Section>
  );
}
