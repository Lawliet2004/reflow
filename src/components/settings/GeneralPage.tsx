import React from "react";
import { AppSettings, Hotkeys } from "../../types";
import { DesktopIntegrationPanel } from "../DesktopIntegrationPanel";
import { Section, Row, Toggle } from "./ui";
import { isModifierOnlyShortcut } from "../../services/shortcutKeys";
import { HotkeyPicker } from "../HotkeyPicker";
import { Keyboard } from "lucide-react";

interface Props {
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => void;
}

export const GeneralPage: React.FC<Props> = ({ settings, onUpdateSettings }) => {
  const change = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) =>
    onUpdateSettings({ [key]: value } as Partial<AppSettings>);

  /** Write one hotkeys field, filling any gaps in a legacy `hotkeys` object. */
  const updateHotkeys = (patch: Partial<Hotkeys>) =>
    onUpdateSettings({
      hotkeys: {
        dictation: settings.hotkey,
        command: null,
        assistant: null,
        note: null,
        cancel: "Escape",
        ...settings.hotkeys,
        ...patch,
      },
    });

  const hotkeyRisky =
    !!settings.hotkey &&
    (settings.hotkey === "Shift+Space" ||
      settings.hotkey === "Alt+Space" ||
      settings.hotkey === "Space" ||
      /^(Shift\+)?[A-Z]$/.test(settings.hotkey));

  return (
    <Section icon={<Keyboard className="w-4 h-4" />} title="Dictation & window">
      <DesktopIntegrationPanel
        renderShortcuts={({ status, busy, configure, setShortcut }) => {
          const wayland = status?.session === "wayland";
          const inApp = wayland && status.shortcut_editable;
          const tapOnly =
            wayland &&
            isModifierOnlyShortcut(
              status.assigned_shortcuts.dictate ?? status.shortcut.assigned_trigger,
            );
          const editingDisabled =
            busy ||
            (inApp && status.shortcut.state !== "ready" && status.shortcut.state !== "unassigned");
          const field = (action: string, value: string, onChange: (key: string) => void) => {
            if (!wayland)
              return (
                <HotkeyPicker
                  value={value}
                  onChange={onChange}
                  allowModifierOnly={action === "cancel" ? false : undefined}
                />
              );
            const trigger =
              status.assigned_shortcuts[action === "dictation" ? "dictate" : action] ??
              (action === "dictation" ? status.shortcut.assigned_trigger : null);
            const label = trigger
              ? `Change ${action} shortcut: ${trigger}`
              : `Assign ${action} shortcut`;
            if (inApp)
              return (
                <HotkeyPicker
                  value={trigger ?? "Choose shortcut…"}
                  onChange={(key) => setShortcut(action, key)}
                  allowModifierOnly
                  disabled={editingDisabled}
                  ariaLabel={label}
                  allowEscape={action === "cancel"}
                />
              );
            return (
              <button
                type="button"
                className="field !w-[180px] !text-center font-semibold"
                aria-label={label}
                title="Change this binding in desktop shortcuts"
                disabled={busy}
                onClick={configure}
              >
                {trigger ?? "Choose shortcut…"}
              </button>
            );
          };
          let dictationHint: string;
          if (wayland) {
            dictationHint = inApp
              ? "Click the key, then press your preferred combination."
              : "Your desktop assigns the active key. Click to change it.";
          } else {
            dictationHint = settings.push_to_talk
              ? "Hold to dictate. Release to transcribe. Double-tap for hands-free; press again to stop."
              : "Press once to start, press again to stop";
          }

          const actionHint = (action: string, value: string | null | undefined) => {
            if (wayland) {
              if (inApp) {
                return "Click to record a shortcut. KDE may ask you to approve a newly enabled action.";
              }
              return value
                ? "Click to change the active desktop binding."
                : "Choose a requested key to enable this action, then configure it in desktop shortcuts.";
            }
            return action === "command"
              ? "Edit selected text. With no selection, your words are dictated normally."
              : "Optional shortcut";
          };

          return (
            <>
              <Row label="Dictation shortcut" hint={dictationHint}>
                {field("dictation", settings.hotkey, (value) => change("hotkey", value))}
              </Row>
              {hotkeyRisky && !wayland && (
                <div className="p-2.5 rounded-lg bg-warning/10 border border-warning/30 text-xs text-warning leading-relaxed">
                  Heads-up: <span className="font-semibold">{settings.hotkey}</span> is commonly
                  used while typing. Consider combos with Ctrl, Alt, or Win.
                </div>
              )}
              <Row
                label="Push-to-talk mode"
                hint={
                  tapOnly
                    ? "Tap this shortcut to start. Tap again to stop."
                    : "On: hold to record, or double-tap within 0.5 seconds for hands-free. Off: press to start/stop."
                }
              >
                <Toggle
                  on={settings.push_to_talk && !tapOnly}
                  disabled={tapOnly}
                  onChange={(value) => change("push_to_talk", value)}
                  ariaLabel="Push to talk"
                />
              </Row>
              {(["command", "assistant", "note"] as const).map((action) => {
                const value = settings.hotkeys?.[action];
                const update = (key: string | null) => updateHotkeys({ [action]: key });
                return (
                  <Row
                    key={action}
                    label={`${action[0].toUpperCase()}${action.slice(1)} shortcut`}
                    hint={actionHint(action, value)}
                  >
                    <div className="flex items-center gap-2">
                      {wayland && value ? (
                        field(action, value, update)
                      ) : (
                        <HotkeyPicker value={value ?? ""} onChange={update} />
                      )}
                      <button
                        className="btn btn-ghost"
                        disabled={editingDisabled}
                        onClick={() => (inApp && value ? setShortcut(action, null) : update(null))}
                      >
                        Clear
                      </button>
                    </div>
                  </Row>
                );
              })}
              <Row label="Global cancel shortcut" hint="Disable to use Cancel in Reflow instead.">
                <div className="flex items-center gap-2">
                  {settings.hotkeys?.cancel !== null &&
                    field("cancel", settings.hotkeys?.cancel ?? "Escape", (value) =>
                      updateHotkeys({ cancel: value }),
                    )}
                  <Toggle
                    on={settings.hotkeys?.cancel !== null}
                    ariaLabel="Enable global cancel"
                    onChange={(enabled) => updateHotkeys({ cancel: enabled ? "Escape" : null })}
                  />
                </div>
              </Row>
            </>
          );
        }}
      />

      <Row
        label="Floating dictation button"
        hint="Show a microphone above other apps. Click to record, then click the cross to stop and insert your words. Position and size are set in Appearance."
      >
        <Toggle
          on={settings.floating_dictation_enabled}
          onChange={(value) => change("floating_dictation_enabled", value)}
          ariaLabel="Floating dictation button"
        />
      </Row>

      <Row
        label="Pause recording button"
        hint="Add a square button to the left of the floating waveform. Pause for a break, then click play to continue the same recording. Click the cross when you’re ready to insert your words."
      >
        <Toggle
          on={settings.recording_pause_enabled}
          onChange={(value) => change("recording_pause_enabled", value)}
          ariaLabel="Pause recording button"
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
