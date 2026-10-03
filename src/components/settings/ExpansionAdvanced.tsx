import React, { useEffect, useState } from "react";
import { AppSettings, UsageStats, AudioDevice } from "../../types";
import { api } from "../../services/tauriApi";
import { Section, Row, Toggle } from "./ui";

export const ExpansionAdvanced: React.FC<{
  settings: AppSettings;
  onUpdateSettings: (patch: Partial<AppSettings>) => unknown;
}> = ({ settings, onUpdateSettings }) => {
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  useEffect(() => {
    let alive = true;
    void api
      .getAudioDevices()
      .then((devices) => {
        if (alive) setDevices(devices);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);
  const [stats, setStats] = useState<UsageStats | null>(null);
  const [journal, setJournal] = useState<{ timestamp: string; host: string; bytes: number }[]>([]);
  const [version, setVersion] = useState("");
  const [path, setPath] = useState("");
  const [replace, setReplace] = useState(false);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [token, setToken] = useState<string | null>(null);
  const [credential, setCredential] = useState<{ id: string; name: string } | null>(null);
  const [excludedDraft, setExcludedDraft] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    Promise.all([
      api.getStats(),
      api.getNetworkJournal(),
      api.getAppVersion(),
      api.getAutomationTokenStatus(),
    ])
      .then(([stats, journal, version, credential]) => {
        if (alive) {
          setStats(stats);
          setJournal(journal);
          setVersion(version);
          setCredential(credential);
        }
      })
      .catch((e) => {
        if (alive) setMessage(String(e));
      });
    return () => {
      alive = false;
    };
  }, []);
  const act = async (action: () => Promise<string>) => {
    setBusy(true);
    setMessage("");
    try {
      setMessage(await action());
    } catch (e) {
      setMessage(String(e));
    } finally {
      setBusy(false);
    }
  };
  const toggle = (key: keyof AppSettings, label: string, fallback = false, hint?: string) => (
    <Row label={label} hint={hint}>
      <Toggle
        on={Boolean(settings[key] ?? fallback)}
        onChange={() => onUpdateSettings({ [key]: !(settings[key] ?? fallback) })}
        ariaLabel={label}
      />
    </Row>
  );
  return (
    <div className="space-y-6">
      {message && (
        <p role="status" className="text-sm whitespace-pre-wrap break-words">
          {message}
        </p>
      )}
      <Section
        title="Usage on this computer"
        description="Live dictations only. Estimated typing time uses 40 words per minute."
      >
        {stats && (
          <>
            <div className="grid grid-cols-3 gap-3 text-sm">
              <p>
                <strong className="block text-xl">{stats.total_words.toLocaleString()}</strong>Words
              </p>
              <p>
                <strong className="block text-xl">{stats.dictations}</strong>Dictations
              </p>
              <p>
                <strong className="block text-xl">{Math.round(stats.time_saved_minutes)}</strong>
                Typing minutes saved
              </p>
              <p>{stats.total_characters.toLocaleString()} characters</p>
              <p>{Math.round(stats.minutes_spoken)} spoken minutes</p>
              <p>{stats.streak_days} day streak</p>
            </div>
            <div
              className="flex items-end gap-1 h-16 mt-4"
              aria-label="Daily words in the last 30 days"
            >
              {stats.per_day.map((day) => (
                <div
                  key={day.date}
                  className="flex-1 bg-accent/60 rounded-t-sm"
                  style={{
                    height: `${Math.max(3, (day.words / Math.max(1, ...stats.per_day.map((d) => d.words))) * 100)}%`,
                  }}
                  title={`${day.date}: ${day.words} words`}
                />
              ))}
            </div>
            {stats.per_app.map((app) => (
              <p key={app.process} className="text-xs text-muted mt-2">
                {app.application_name || app.process}: {app.words} words · {app.dictations} sessions
              </p>
            ))}
          </>
        )}
      </Section>
      <Section
        title="Privacy & connectivity"
        description="Recognition, cleanup and your saved data stay on this computer."
      >
        {toggle(
          "offline_mode",
          "Airplane mode",
          true,
          "Block every model/runtime download. Turn off explicitly to install artifacts.",
        )}
        {toggle(
          "history_encryption",
          "Encrypt history",
          false,
          "AES-256-GCM with an OS credential-store key. Search decrypts bounded pages; exports are readable files. Keep your OS key to recover encrypted history.",
        )}
        <label className="block text-sm">
          Excluded application processes (one per line)
          <textarea
            className="field w-full mt-2"
            value={excludedDraft ?? (settings.excluded_apps ?? []).join("\n")}
            onChange={(e) => setExcludedDraft(e.target.value)}
            onBlur={async () => {
              if (excludedDraft === null) return;
              const submitted = excludedDraft;
              try {
                const saved = await onUpdateSettings({
                  excluded_apps: submitted
                    .split("\n")
                    .map((v) => v.trim())
                    .filter(Boolean),
                });
                if (saved === false) throw new Error("Excluded applications could not be saved.");
                setExcludedDraft((current) => (current === submitted ? null : current));
              } catch (failure) {
                setMessage(String(failure));
              }
            }}
          />
        </label>
        <details className="mt-4">
          <summary className="cursor-pointer text-sm">Every connection Reflow has made</summary>
          <button
            className="btn btn-ghost"
            onClick={() =>
              act(async () => {
                setJournal(await api.getNetworkJournal());
                return "Network ledger refreshed";
              })
            }
          >
            Refresh ledger
          </button>
          {journal.length ? (
            <div className="max-h-56 overflow-auto text-xs space-y-2">
              {journal.map((entry, i) => (
                <p key={i}>
                  {new Date(entry.timestamp).toLocaleString()} · {entry.host} ·{" "}
                  {entry.bytes.toLocaleString()} bytes
                </p>
              ))}
            </div>
          ) : (
            <p className="text-xs text-muted">No recorded outbound requests.</p>
          )}
        </details>
      </Section>
      <Section
        title="Recording & insertion"
        description="Options for short captures, remote desktops and accessible feedback."
      >
        {toggle(
          "meeting_mode",
          "Meeting audio",
          false,
          "Adds a Home button to record mic and system playback. Windows uses WASAPI loopback; Linux requires an exposed monitor source. Use headphones to avoid echo.",
        )}
        {settings.meeting_mode && (
          <Row
            label="Linux monitor source"
            hint="Only needed when several monitor capture sources are exposed."
          >
            <select
              className="field max-w-52"
              aria-label="Meeting monitor source"
              value={settings.meeting_monitor_device_id ?? ""}
              onChange={(e) =>
                onUpdateSettings({ meeting_monitor_device_id: e.target.value || null })
              }
            >
              <option value="">Automatically select a single monitor</option>
              {devices
                .filter((device) => device.is_monitor)
                .map((device) => (
                  <option key={device.id} value={device.id}>
                    {device.name}
                  </option>
                ))}
            </select>
          </Row>
        )}
        {toggle("sounds_enabled", "Start and stop sounds", true)}
        <Row label="Sound volume">
          <input
            aria-label="Sound volume"
            type="range"
            min="0"
            max="1"
            step="0.05"
            value={settings.sounds_volume ?? 0.7}
            onChange={(e) => onUpdateSettings({ sounds_volume: Number(e.target.value) })}
          />
        </Row>
        {toggle(
          "duck_media",
          "Lower other media while recording",
          false,
          "Windows only; restores the previous session volume afterward.",
        )}
        {toggle(
          "follow_default_mic",
          "Follow the system microphone",
          true,
          "Changes apply while idle and at the next capture; no mid-recording failover.",
        )}
        {toggle(
          "voice_commands_enabled",
          "Voice commands",
          false,
          "Whole utterances: scratch that discards this capture; undo that pastes the previous pre-AI text.",
        )}
        <Row label="Minimum hotkey capture (ms)">
          <input
            className="field w-24"
            aria-label="Minimum capture"
            type="number"
            min="0"
            max="10000"
            value={settings.min_dictation_ms ?? 300}
            onChange={(e) => onUpdateSettings({ min_dictation_ms: Number(e.target.value) })}
          />
        </Row>
        <Row label="Paste delay (ms)">
          <input
            className="field w-24"
            aria-label="Paste delay"
            type="number"
            min="0"
            max="2000"
            value={settings.paste_delay_ms ?? 30}
            onChange={(e) => onUpdateSettings({ paste_delay_ms: Number(e.target.value) })}
          />
        </Row>
        <Row label="Insertion method" hint="Typing can be slow for more than 200 characters.">
          <select
            className="field"
            value={settings.inject_method ?? "paste"}
            onChange={(e) =>
              onUpdateSettings({ inject_method: e.target.value as "paste" | "type" })
            }
          >
            <option value="paste">Clipboard paste</option>
            <option value="type">Type text</option>
          </select>
        </Row>
        {toggle("append_space", "Append a trailing space")}
        {toggle("capitalize_first", "Capitalize the first sentence", true)}
        <Row label="HUD contrast">
          <select
            className="field"
            value={settings.hud_contrast ?? "standard"}
            onChange={(e) =>
              onUpdateSettings({ hud_contrast: e.target.value as "standard" | "high" })
            }
          >
            <option value="standard">Standard</option>
            <option value="high">High contrast</option>
          </select>
        </Row>
        <button
          className="btn btn-secondary"
          disabled={busy}
          onClick={() =>
            act(async () =>
              (await api.repasteLast())
                ? "Last transcript pasted"
                : "Transcript copied for manual paste",
            )
          }
        >
          Paste last transcript
        </button>
      </Section>
      <Section
        title="Power policy"
        description="Applies when idle; your stored performance choice remains unchanged."
      >
        <Row label="Unload models on battery">
          <Toggle
            on={settings.power_policy?.unload_on_battery ?? false}
            onChange={() =>
              onUpdateSettings({
                power_policy: {
                  unload_on_battery: !settings.power_policy?.unload_on_battery,
                  battery_preset: settings.power_policy?.battery_preset ?? null,
                },
              })
            }
            ariaLabel="Unload models on battery"
          />
        </Row>
        <Row label="Battery performance">
          <select
            className="field"
            value={settings.power_policy?.battery_preset ?? ""}
            onChange={(e) =>
              onUpdateSettings({
                power_policy: {
                  unload_on_battery: settings.power_policy?.unload_on_battery ?? false,
                  battery_preset: e.target.value || null,
                },
              })
            }
          >
            <option value="">Keep current choice</option>
            {["auto", "fast", "balanced", "accurate"].map((preset) => (
              <option key={preset}>{preset}</option>
            ))}
          </select>
        </Row>
      </Section>
      <Section
        title="Transfer & releases"
        description={`Reflow ${version || "…"}. Configuration transfers exclude tokens, device paths, privileged outputs and encryption keys.`}
      >
        <input
          className="field w-full"
          aria-label="Transfer path"
          placeholder="Path to config JSON, model GGUF, or model directory"
          value={path}
          onChange={(e) => setPath(e.target.value)}
        />
        <label className="flex gap-2 text-xs py-2">
          <input type="checkbox" checked={replace} onChange={(e) => setReplace(e.target.checked)} />
          Replace portable preferences on import
        </label>
        <div className="flex flex-wrap gap-2">
          <button
            className="btn btn-secondary"
            disabled={busy || !path}
            onClick={() => act(async () => `Config exported to ${await api.exportConfig(path)}`)}
          >
            Export config
          </button>
          <button
            className="btn btn-secondary"
            disabled={busy || !path}
            onClick={() =>
              act(async () => {
                await api.importConfig(path, replace);
                return "Configuration imported";
              })
            }
          >
            Import config
          </button>
          <button
            className="btn btn-secondary"
            disabled={busy || !path}
            onClick={() => act(async () => `Imported ${(await api.importModelFile(path)).id}`)}
          >
            Import offline model
          </button>
          <button
            className="btn btn-ghost"
            onClick={() =>
              act(async () => {
                await api.openReleases();
                return "Release page opened in your browser";
              })
            }
          >
            Check releases
          </button>
        </div>
      </Section>
      <Section
        title="Local automation"
        description="Enable the API with localhost binding to create a revocable credential for scripts."
      >
        {credential ? (
          <p className="text-sm">Automation credential is active.</p>
        ) : (
          <p className="text-sm text-muted">No automation credential.</p>
        )}
        {token && (
          <label className="block text-xs">
            Copy this token now; it is shown once.
            <textarea
              readOnly
              aria-label="Automation token"
              className="field w-full"
              value={token}
            />
          </label>
        )}
        <div className="flex gap-2 mt-2">
          <button
            className="btn btn-secondary"
            disabled={busy || !settings.api_enabled || settings.api_bind !== "localhost"}
            onClick={() =>
              act(async () => {
                const result = await api.createAutomationToken();
                setToken(result.token);
                setCredential(result.device);
                return "Token created; previous automation token revoked";
              })
            }
          >
            Create or rotate token
          </button>
          <button
            className="btn btn-ghost"
            disabled={busy || !credential}
            onClick={() =>
              act(async () => {
                await api.revokeAutomationToken();
                setCredential(null);
                setToken(null);
                return "Automation credential revoked";
              })
            }
          >
            Revoke token
          </button>
        </div>
      </Section>
    </div>
  );
};
