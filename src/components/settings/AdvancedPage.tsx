import React, { useState } from "react";
import { AppSettings, HistoryRetention } from "../../types";
import { api } from "../../services/tauriApi";
import { Section, Row, Toggle } from "./ui";
import { ClipboardCopy, FolderOpen, Check, ShieldCheck } from "lucide-react";

interface Props {
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => void;
}

export const AdvancedPage: React.FC<Props> = ({ settings, onUpdateSettings }) => {
  const [diagCopied, setDiagCopied] = useState(false);

  return (
    <Section
      icon={<ShieldCheck className="w-4 h-4" />}
      title="Privacy & diagnostics"
    >
      <Row label="History retention" hint="Older transcripts are purged automatically">
        <select
          className="field"
          value={settings.history_retention}
          onChange={(e) =>
            onUpdateSettings({ history_retention: e.target.value as HistoryRetention })
          }
        >
          <option value="1_day">1 day</option>
          <option value="7_days">7 days</option>
          <option value="30_days">30 days (recommended)</option>
          <option value="90_days">90 days</option>
          <option value="forever">Forever</option>
          <option value="disabled">Don't store history</option>
        </select>
      </Row>

      <Row
        label="Restore clipboard"
        hint="Puts the previous clipboard back after inserting text"
      >
        <Toggle
          on={settings.clipboard_restore_enabled}
          onChange={(v) => onUpdateSettings({ clipboard_restore_enabled: v })}
          ariaLabel="Restore clipboard"
        />
      </Row>

      <Row
        label="Developer mode"
        hint="Shows latency chips on Home and extra diagnostics"
      >
        <Toggle
          on={settings.developer_mode}
          onChange={(v) => onUpdateSettings({ developer_mode: v })}
          ariaLabel="Developer mode"
        />
      </Row>

      <div className="flex gap-2.5 pt-2 border-t border-line">
        <button className="btn btn-ghost flex-1" onClick={() => api.openLogsFolder()}>
          <FolderOpen className="w-4 h-4 text-muted" />
          Open logs
        </button>
        <button
          className="btn btn-ghost flex-1"
          onClick={async () => {
            const ok = await api.copyDiagnostics();
            if (ok) {
              setDiagCopied(true);
              setTimeout(() => setDiagCopied(false), 1600);
            }
          }}
        >
          {diagCopied ? (
            <Check className="w-4 h-4 text-emerald-600 dark:text-emerald-400" />
          ) : (
            <ClipboardCopy className="w-4 h-4 text-muted" />
          )}
          {diagCopied ? "Copied" : "Copy diagnostics"}
        </button>
      </div>
    </Section>
  );
};
