import { AppSettings } from "../../types";
import { Section, Row } from "./ui";
import { OutputPicker } from "./OutputPicker";

export function OutputPage({
  settings,
  onUpdateSettings,
}: {
  settings: AppSettings;
  onUpdateSettings: (patch: Partial<AppSettings>) => Promise<boolean>;
}) {
  return (
    <Section
      title="Output"
      description="The default destination for Dictation. Other modes can choose their own destination."
    >
      <OutputPicker
        value={settings.output_action ?? { type: "paste" }}
        sendKey={settings.send_key ?? "enter"}
        onChange={(output_action) => {
          void onUpdateSettings({ output_action });
        }}
        onSendKey={(send_key) => {
          void onUpdateSettings({ send_key });
        }}
      />
      <Row
        label="Paste shortcut"
        hint="Use Ctrl + Shift + V for terminals when this desktop cannot detect the foreground app."
      >
        <select
          className="select-field"
          aria-label="Paste shortcut"
          value={settings.paste_shortcut ?? "auto"}
          onChange={(event) => {
            void onUpdateSettings({
              paste_shortcut: event.target.value as AppSettings["paste_shortcut"],
            });
          }}
        >
          <option value="auto">Automatic</option>
          <option value="ctrl_v">Ctrl + V</option>
          <option value="ctrl_shift_v">Ctrl + Shift + V</option>
        </select>
      </Row>
    </Section>
  );
}
