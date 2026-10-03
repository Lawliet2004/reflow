import React from "react";
import { AppSettings } from "../../types";
import { Section, Row, Toggle } from "./ui";
import { HotkeyPicker } from "../HotkeyPicker";
import { Keyboard } from "lucide-react";

interface Props {
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => void;
}

export const GeneralPage: React.FC<Props> = ({ settings, onUpdateSettings }) => {
  const change = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) =>
    onUpdateSettings({ [key]: value } as Partial<AppSettings>);

  const hotkeyRisky =
    !!settings.hotkey &&
    (settings.hotkey === "Shift+Space" ||
      settings.hotkey === "Alt+Space" ||
      settings.hotkey === "Space" ||
      /^(Shift\+)?[A-Z]$/.test(settings.hotkey));

  return (
    <Section icon={<Keyboard className="w-4 h-4" />} title="Dictation & window">
      <Row
        label="Push-to-talk shortcut"
        hint={
          settings.push_to_talk
            ? "Hold to dictate anywhere. Release to transcribe."
            : "Press once to start, press again to stop"
        }
      >
        <HotkeyPicker value={settings.hotkey} onChange={(v) => change("hotkey", v)} />
      </Row>

      {hotkeyRisky && (
        <div className="p-2.5 rounded-lg bg-warning/10 border border-warning/30 text-xs text-warning leading-relaxed">
          Heads-up: <span className="font-semibold">{settings.hotkey}</span> is commonly used while
          typing. Consider combos with Ctrl, Alt, or Win.
        </div>
      )}

      <Row label="Push-to-talk mode" hint="Off = toggle start/stop with the same shortcut">
        <Toggle
          on={settings.push_to_talk}
          onChange={(v) => change("push_to_talk", v)}
          ariaLabel="Push to talk"
        />
      </Row>

      {(["command", "assistant", "note"] as const).map((action) => (
        <Row
          key={action}
          label={`${action[0].toUpperCase()}${action.slice(1)} shortcut`}
          hint={
            action === "command"
              ? "Edit selected text. With no selection, your words are dictated normally."
              : "Optional shortcut"
          }
        >
          <div className="flex items-center gap-2">
            <HotkeyPicker
              value={settings.hotkeys?.[action] ?? ""}
              onChange={(value) =>
                onUpdateSettings({
                  hotkeys: {
                    dictation: settings.hotkey,
                    command: null,
                    assistant: null,
                    note: null,
                    cancel: "Escape",
                    ...settings.hotkeys,
                    [action]: value,
                  },
                })
              }
            />
            <button
              className="btn btn-ghost"
              onClick={() =>
                onUpdateSettings({
                  hotkeys: {
                    dictation: settings.hotkey,
                    command: null,
                    assistant: null,
                    note: null,
                    cancel: "Escape",
                    ...settings.hotkeys,
                    [action]: null,
                  },
                })
              }
            >
              Clear
            </button>
          </div>
        </Row>
      ))}
      <Row label="Escape to cancel">
        <Toggle
          on={settings.hotkeys?.cancel !== null}
          ariaLabel="Escape to cancel"
          onChange={(enabled) =>
            onUpdateSettings({
              hotkeys: {
                dictation: settings.hotkey,
                command: null,
                assistant: null,
                note: null,
                ...settings.hotkeys,
                cancel: enabled ? "Escape" : null,
              },
            })
          }
        />
      </Row>

      <Row label="Launch at startup" hint="Starts with Windows or your desktop session">
        <Toggle
          on={settings.launch_at_startup}
          onChange={(v) => change("launch_at_startup", v)}
          ariaLabel="Launch at startup"
        />
      </Row>

      <Row label="Start minimized" hint="Opens in the tray instead of showing the hub">
        <Toggle
          on={settings.start_minimized}
          onChange={(v) => change("start_minimized", v)}
          ariaLabel="Start minimized"
        />
      </Row>

      <p className="text-xs text-muted pt-1">
        Close hides to tray · Customize themes and HUD display in the Appearance tab
      </p>
    </Section>
  );
};
