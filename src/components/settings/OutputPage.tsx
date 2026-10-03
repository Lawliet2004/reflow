import { AppSettings } from "../../types";
import { Section } from "./ui";
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
    </Section>
  );
}
