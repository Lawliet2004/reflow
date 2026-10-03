import React, { useEffect, useState } from "react";
import { AppSettings, AudioRetention, HistoryRetention } from "../../types";
import { api } from "../../services/tauriApi";
import { Section, Row, Toggle } from "./ui";
import { ClipboardCopy, FolderOpen, Check, ShieldCheck } from "lucide-react";

interface Props {
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => void;
}

export const AdvancedPage: React.FC<Props> = ({ settings, onUpdateSettings }) => {
  const [diagCopied, setDiagCopied] = useState(false);
  const [copying, setCopying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [restoring, setRestoring] = useState(false);
  const [restoreNotice, setRestoreNotice] = useState<string | null>(null);

  useEffect(() => {
    if (!diagCopied) return;
    const timer = setTimeout(() => setDiagCopied(false), 1600);
    return () => clearTimeout(timer);
  }, [diagCopied]);

  const openLogs = async () => {
    setError(null);
    try {
      await api.openLogsFolder();
    } catch {
      setError("Could not open the logs folder. Please try again.");
    }
  };

  const copyDiagnostics = async () => {
    setError(null);
    setCopying(true);
    try {
      if (await api.copyDiagnostics()) setDiagCopied(true);
      else setError("Could not copy diagnostics. Please try again.");
    } catch {
      setError("Could not copy diagnostics. Please try again.");
    } finally {
      setCopying(false);
    }
  };

  const retryRestore = async () => {
    setRestoring(true);
    setError(null);
    setRestoreNotice(null);
    try {
      const restored = await api.retryClipboardRestore();
      setRestoreNotice(
        restored
          ? "Previous clipboard restored."
          : "No unchanged clipboard snapshot is available. Your current clipboard was left untouched.",
      );
    } catch (failure) {
      setError(`Could not restore the clipboard: ${String(failure)}`);
    } finally {
      setRestoring(false);
    }
  };

  return (
    <Section icon={<ShieldCheck className="w-4 h-4" />} title="Privacy & diagnostics">
      {error && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
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
        label="Audio retention"
        hint="Audio is deleted after this time. Extract audio and Retry transcript are available while it is saved. Choosing Don't save audio deletes saved recordings. Transcript retention stays separate."
      >
        <select
          className="field"
          aria-label="Audio retention"
          value={settings.audio_retention ?? "disabled"}
          onChange={(e) => onUpdateSettings({ audio_retention: e.target.value as AudioRetention })}
        >
          <option value="disabled">Don't save audio</option>
          <option value="1_day">1 day</option>
          <option value="7_days">1 week</option>
          <option value="30_days">1 month (30 days)</option>
          <option value="forever">Maximum (keep indefinitely)</option>
        </select>
      </Row>

      <Row label="Restore clipboard" hint="Puts the previous clipboard back after inserting text">
        <Toggle
          on={settings.clipboard_restore_enabled}
          onChange={(v) => onUpdateSettings({ clipboard_restore_enabled: v })}
          ariaLabel="Restore clipboard"
        />
      </Row>

      <div className="space-y-2">
        <p className="text-sm text-muted">
          If automatic restoration fails, retry while the clipboard still contains Reflow's text. A
          new copy prevents restoration.
        </p>
        <button className="btn btn-ghost" type="button" disabled={restoring} onClick={retryRestore}>
          {restoring ? "Restoring clipboard…" : "Retry clipboard restore"}
        </button>
        {restoreNotice && (
          <p role="status" className="text-sm text-ink">
            {restoreNotice}
          </p>
        )}
      </div>

      <Row label="Developer mode" hint="Shows latency chips on Home and extra diagnostics">
        <Toggle
          on={settings.developer_mode}
          onChange={(v) => onUpdateSettings({ developer_mode: v })}
          ariaLabel="Developer mode"
        />
      </Row>

      <div className="flex gap-2.5 pt-2 border-t border-line">
        <button className="btn btn-ghost flex-1" onClick={openLogs}>
          <FolderOpen className="w-4 h-4 text-muted" />
          Open logs
        </button>
        <button className="btn btn-ghost flex-1" onClick={copyDiagnostics} disabled={copying}>
          {diagCopied ? (
            <Check className="w-4 h-4 text-success" />
          ) : (
            <ClipboardCopy className="w-4 h-4 text-muted" />
          )}
          {diagCopied ? "Copied" : "Copy diagnostics"}
        </button>
      </div>
    </Section>
  );
};
