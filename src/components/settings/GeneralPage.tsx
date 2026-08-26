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
    <Section
      icon={<Keyboard className="w-4 h-4" />}
      title="Dictation & window"
    >
      <Row
        label="Push-to-talk shortcut"
        hint={
          settings.push_to_talk
            ? "Hold to dictate anywhere. Release to transcribe."
            : "Press once to start, press again to stop"
        }
      >
        <HotkeyPicker
          value={settings.hotkey}
          onChange={(v) => change("hotkey", v)}
        />
      </Row>

      {hotkeyRisky && (
        <div className="p-2.5 rounded-lg bg-amber-500/10 border border-amber-500/30 text-[11.5px] text-amber-700 dark:text-amber-300 leading-relaxed">
          Heads-up: <span className="font-semibold">{settings.hotkey}</span> is commonly
          used while typing. Consider combos with Ctrl, Alt, or Win.
        </div>
      )}

      <Row label="Push-to-talk mode" hint="Off = toggle start/stop with the same shortcut">
        <Toggle
          on={settings.push_to_talk}
          onChange={(v) => change("push_to_talk", v)}
          ariaLabel="Push to talk"
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

      <p className="text-[12px] text-muted pt-1">
        Close hides to tray · Customize themes and HUD display in the Appearance tab
      </p>
    </Section>
  );
};
