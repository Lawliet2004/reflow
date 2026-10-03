import { OutputAction, SendKey } from "../../types";
import { Row } from "./ui";

export function OutputPicker({
  value,
  sendKey,
  onChange,
  onSendKey,
}: {
  value: OutputAction;
  sendKey: SendKey;
  onChange: (value: OutputAction) => void;
  onSendKey: (key: SendKey) => void;
}) {
  return (
    <div className="space-y-3">
      <Row label="Output destination">
        <select
          className="select-field"
          aria-label="Output destination"
          value={value.type}
          onChange={(e) => {
            const type = e.target.value as OutputAction["type"];
            onChange(
              type === "append_file"
                ? { type, path: "" }
                : type === "run_command"
                  ? { type, template: "" }
                  : { type },
            );
          }}
        >
          <option value="paste">Paste</option>
          <option value="paste_enter">Paste and send</option>
          <option value="copy">Copy to clipboard</option>
          <option value="hud">Show in HUD</option>
          <option value="append_file">Append to file</option>
          <option value="run_command">Run command</option>
        </select>
      </Row>
      {value.type === "paste_enter" && (
        <Row label="Send key" hint="Sent 120 ms after a successful paste.">
          <select
            className="select-field"
            aria-label="Send key"
            value={sendKey}
            onChange={(e) => onSendKey(e.target.value as SendKey)}
          >
            <option value="enter">Enter</option>
            <option value="shift_enter">Shift + Enter</option>
            <option value="ctrl_enter">Ctrl + Enter</option>
          </select>
        </Row>
      )}
      {value.type === "append_file" && (
        <input
          className="field w-full"
          aria-label="Output file path"
          placeholder="Full path to a text file"
          value={value.path}
          onChange={(e) => onChange({ ...value, path: e.target.value })}
        />
      )}
      {value.type === "run_command" && (
        <>
          <p className="text-xs text-warning" role="note">
            Runs a shell command on your computer. Use only templates you trust. {"{text}"} inserts
            the transcript as a quoted argument; without it, text is sent on stdin.
          </p>
          <textarea
            className="field w-full"
            aria-label="Command template"
            value={value.template}
            onChange={(e) => onChange({ ...value, template: e.target.value })}
          />
        </>
      )}
    </div>
  );
}
